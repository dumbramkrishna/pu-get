//! Background download manager.
//!
//! The manager owns the queue and concurrency limit, while each active job runs
//! in a Tokio task. Commands arrive through an unbounded channel; progress and
//! terminal events are sent through a bounded channel so a stalled UI can apply
//! backpressure instead of consuming unbounded memory.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use reqwest::header::{ACCEPT, CONTENT_RANGE, CONTENT_TYPE, RANGE};
use reqwest::redirect::Policy;
use reqwest::{Client, Response, StatusCode};
use tokio::fs::{self, OpenOptions};
use tokio::io::AsyncWriteExt as _;
use tokio::sync::mpsc;
use tokio::task::JoinSet;
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;

use crate::download::path::{next_available_path_with, part_path};
use crate::download::progress::ProgressTracker;
use crate::download::validate::{content_type_allows_pdf, validate_pdf};
use crate::download::{ConflictPolicy, DownloadSpec, JobId};
use crate::error::DownloadError;

const EVENT_BUFFER: usize = 1_024;
const PROGRESS_INTERVAL: Duration = Duration::from_millis(150);
const RETRY_BASE_MILLIS: u64 = 500;
const MAX_RETRY_DELAY: Duration = Duration::from_secs(8);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const ACCEPT_HEADER: &str = "application/pdf,application/octet-stream;q=0.9,*/*;q=0.8";
const USER_AGENT: &str = concat!(
    "pu-get/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/dumbramkrishna/pu-get)"
);

/// Manager configuration derived from `Config` by the CLI or TUI.
#[derive(Debug, Clone)]
pub struct ManagerConfig {
    /// Maximum simultaneous downloads.
    pub concurrency: usize,
    /// Automatic retries after the initial attempt.
    pub retries: u32,
    /// Per-read timeout. Long downloads that keep making progress are allowed.
    pub request_timeout: Duration,
}

impl Default for ManagerConfig {
    fn default() -> Self {
        Self {
            concurrency: 3,
            retries: 3,
            request_timeout: Duration::from_secs(60),
        }
    }
}

/// Why a download was skipped without contacting the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    /// A file with the expected size already exists.
    AlreadyDownloaded,
    /// The destination exists and the policy is `skip`.
    DestinationExists,
}

impl SkipReason {
    /// Human-readable reason.
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::AlreadyDownloaded => "already downloaded (matching size)",
            Self::DestinationExists => "destination already exists",
        }
    }
}

/// User decision for a pending [`DownloadEvent::Conflict`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictResolution {
    /// Leave the existing file.
    Skip,
    /// Download beside it with a numbered suffix.
    Rename,
    /// Replace the existing file.
    Overwrite,
    /// Abandon the download.
    Cancel,
}

/// Events emitted by the manager.
#[derive(Debug, Clone)]
pub enum DownloadEvent {
    /// Job added to the queue.
    Queued {
        /// Job identifier.
        id: JobId,
        /// Entry title.
        title: String,
    },
    /// A task started.
    Started {
        /// Job identifier.
        id: JobId,
        /// Final destination.
        destination: PathBuf,
        /// Bytes already present in the `.part` file.
        resumed_from: u64,
        /// Expected total size, if known before headers arrive.
        total: Option<u64>,
    },
    /// Progress sample.
    Progress {
        /// Job identifier.
        id: JobId,
        /// Current progress.
        progress: crate::download::ProgressSnapshot,
    },
    /// A transient error will be retried after `delay`.
    Retrying {
        /// Job identifier.
        id: JobId,
        /// Upcoming attempt number (1-based).
        attempt: u32,
        /// Total attempts including the first.
        max_attempts: u32,
        /// Backoff delay.
        delay: Duration,
        /// Error that triggered the retry.
        error: String,
    },
    /// The job was paused; the `.part` file is kept.
    Paused {
        /// Job identifier.
        id: JobId,
        /// Bytes preserved for resuming.
        downloaded: u64,
    },
    /// The destination exists and the policy is `ask`.
    Conflict {
        /// Job identifier.
        id: JobId,
        /// Existing destination.
        path: PathBuf,
        /// Size of the existing file.
        existing_size: u64,
        /// Size reported by the catalog, if any.
        expected_size: Option<u64>,
    },
    /// The job was skipped.
    Skipped {
        /// Job identifier.
        id: JobId,
        /// Destination that already exists.
        path: PathBuf,
        /// Why it was skipped.
        reason: SkipReason,
    },
    /// The file was validated and published.
    Completed {
        /// Job identifier.
        id: JobId,
        /// Final path.
        path: PathBuf,
        /// File size.
        bytes: u64,
    },
    /// The job failed permanently (after retries, when applicable).
    Failed {
        /// Job identifier.
        id: JobId,
        /// Failure details.
        error: DownloadError,
    },
    /// The job was cancelled and its `.part` file removed.
    Cancelled {
        /// Job identifier.
        id: JobId,
    },
}

/// Handle used by the UI/CLI to control the manager.
#[derive(Debug, Clone)]
pub struct DownloadHandle {
    commands: mpsc::UnboundedSender<Command>,
    next_id: Arc<AtomicU64>,
}

impl DownloadHandle {
    /// Queue a download and return its id immediately.
    pub fn enqueue(&self, spec: DownloadSpec) -> Result<JobId, DownloadError> {
        let id = JobId(self.next_id.fetch_add(1, Ordering::Relaxed));
        self.send(Command::Enqueue { id, spec })?;
        Ok(id)
    }

    /// Pause a queued or running job.
    pub fn pause(&self, id: JobId) -> Result<(), DownloadError> {
        self.send(Command::Pause(id))
    }

    /// Resume a paused job.
    pub fn resume(&self, id: JobId) -> Result<(), DownloadError> {
        self.send(Command::Resume(id))
    }

    /// Cancel a job and delete its `.part` file.
    pub fn cancel(&self, id: JobId) -> Result<(), DownloadError> {
        self.send(Command::Cancel(id))
    }

    /// Retry a failed job, resuming any partial file.
    pub fn retry(&self, id: JobId) -> Result<(), DownloadError> {
        self.send(Command::Retry(id))
    }

    /// Answer a [`DownloadEvent::Conflict`].
    pub fn resolve_conflict(
        &self,
        id: JobId,
        resolution: ConflictResolution,
    ) -> Result<(), DownloadError> {
        self.send(Command::ResolveConflict { id, resolution })
    }

    /// Ask the manager to stop. Partial files are left for resuming.
    pub fn shutdown(&self) -> Result<(), DownloadError> {
        self.send(Command::Shutdown)
    }

    fn send(&self, command: Command) -> Result<(), DownloadError> {
        self.commands
            .send(command)
            .map_err(|_| DownloadError::ManagerStopped)
    }
}

/// Start the manager on the current Tokio runtime.
///
/// Returns a command handle and the event stream. The manager keeps running
/// until [`DownloadHandle::shutdown`] is called or all handles are dropped.
pub fn spawn(
    config: ManagerConfig,
) -> Result<(DownloadHandle, mpsc::Receiver<DownloadEvent>), DownloadError> {
    if tokio::runtime::Handle::try_current().is_err() {
        return Err(DownloadError::InvalidConfiguration {
            message: "the download manager must be started inside a Tokio runtime".to_owned(),
        });
    }

    if config.concurrency == 0 {
        return Err(DownloadError::InvalidConfiguration {
            message: "concurrency must be at least 1".to_owned(),
        });
    }

    if config.request_timeout.is_zero() {
        return Err(DownloadError::InvalidConfiguration {
            message: "request_timeout must be greater than zero".to_owned(),
        });
    }

    let client = Client::builder()
        .user_agent(USER_AGENT)
        .connect_timeout(CONNECT_TIMEOUT)
        .read_timeout(config.request_timeout)
        .redirect(Policy::limited(10))
        .build()
        .map_err(|error| DownloadError::Client {
            message: error.to_string(),
        })?;

    let (commands_tx, commands_rx) = mpsc::unbounded_channel();
    let (events_tx, events_rx) = mpsc::channel(EVENT_BUFFER);

    let handle = DownloadHandle {
        commands: commands_tx,
        next_id: Arc::new(AtomicU64::new(1)),
    };

    let manager = Manager {
        concurrency: config.concurrency,
        retries: config.retries,
        request_timeout: config.request_timeout,
        client,
        commands: commands_rx,
        events: events_tx,
        jobs: HashMap::new(),
        queue: VecDeque::new(),
        active: HashSet::new(),
        cancels: HashMap::new(),
        claims: HashSet::new(),
        joins: JoinSet::new(),
    };

    tokio::spawn(manager.run());
    Ok((handle, events_rx))
}

#[derive(Debug)]
enum Command {
    Enqueue {
        id: JobId,
        spec: DownloadSpec,
    },
    Pause(JobId),
    Resume(JobId),
    Cancel(JobId),
    Retry(JobId),
    ResolveConflict {
        id: JobId,
        resolution: ConflictResolution,
    },
    Shutdown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum JobState {
    Queued,
    Running,
    Pausing,
    Paused,
    Cancelling,
    AwaitingConflict,
    Completed,
    Failed,
    Skipped,
    Cancelled,
}

#[derive(Debug)]
struct Job {
    spec: DownloadSpec,
    state: JobState,
}

#[derive(Debug)]
enum Outcome {
    Completed { bytes: u64 },
    Failed(DownloadError),
    Interrupted,
}

#[derive(Debug)]
enum JobError {
    Interrupted,
    Failed(DownloadError),
}

#[derive(Debug)]
enum Preflight {
    Start,
    Skipped(SkipReason),
    Conflict { existing_size: u64 },
    Failed(DownloadError),
}

struct Manager {
    concurrency: usize,
    retries: u32,
    request_timeout: Duration,
    client: Client,
    commands: mpsc::UnboundedReceiver<Command>,
    events: mpsc::Sender<DownloadEvent>,
    jobs: HashMap<JobId, Job>,
    queue: VecDeque<JobId>,
    active: HashSet<JobId>,
    cancels: HashMap<JobId, CancellationToken>,
    claims: HashSet<PathBuf>,
    joins: JoinSet<(JobId, Outcome)>,
}

impl Manager {
    async fn run(mut self) {
        loop {
            self.dispatch_ready().await;

            tokio::select! {
                command = self.commands.recv() => {
                    match command {
                        Some(Command::Shutdown) | None => {
                            self.shutdown().await;
                            break;
                        }
                        Some(command) => self.handle_command(command).await,
                    }
                }
                joined = self.joins.join_next(), if !self.joins.is_empty() => {
                    if let Some(joined) = joined {
                        self.on_complete(joined).await;
                    }
                }
            }
        }
    }

    async fn dispatch_ready(&mut self) {
        while self.active.len() < self.concurrency {
            let Some(id) = self.queue.pop_front() else {
                break;
            };

            if !matches!(
                self.jobs.get(&id).map(|job| job.state),
                Some(JobState::Queued)
            ) {
                continue;
            }

            match self.preflight(id).await {
                Preflight::Start => {
                    let Some(spec) = self.jobs.get_mut(&id).map(|job| {
                        job.state = JobState::Running;
                        job.spec.clone()
                    }) else {
                        continue;
                    };

                    let cancel = CancellationToken::new();
                    self.cancels.insert(id, cancel.clone());
                    self.active.insert(id);

                    let events = self.events.clone();
                    let client = self.client.clone();
                    let retries = self.retries;
                    let request_timeout = self.request_timeout;

                    self.joins.spawn(async move {
                        let outcome =
                            run_job(id, spec, client, events, cancel, retries, request_timeout)
                                .await;
                        (id, outcome)
                    });
                }
                Preflight::Skipped(reason) => {
                    let destination = self
                        .jobs
                        .get(&id)
                        .map_or_else(PathBuf::new, |job| job.spec.destination.clone());
                    let _ = fs::remove_file(part_path(&destination)).await;
                    self.claims.remove(&destination);

                    if let Some(job) = self.jobs.get_mut(&id) {
                        job.state = JobState::Skipped;
                    }

                    self.emit(DownloadEvent::Skipped {
                        id,
                        path: destination,
                        reason,
                    })
                    .await;
                }
                Preflight::Conflict { existing_size } => {
                    let (destination, expected_size) = self.jobs.get(&id).map_or_else(
                        || (PathBuf::new(), None),
                        |job| (job.spec.destination.clone(), job.spec.expected_size),
                    );

                    if let Some(job) = self.jobs.get_mut(&id) {
                        job.state = JobState::AwaitingConflict;
                    }

                    self.emit(DownloadEvent::Conflict {
                        id,
                        path: destination,
                        existing_size,
                        expected_size,
                    })
                    .await;
                }
                Preflight::Failed(error) => {
                    if let Some(job) = self.jobs.get_mut(&id) {
                        job.state = JobState::Failed;
                    }
                    self.emit(DownloadEvent::Failed { id, error }).await;
                }
            }
        }
    }

    async fn preflight(&mut self, id: JobId) -> Preflight {
        let Some(mut spec) = self.jobs.get(&id).map(|job| job.spec.clone()) else {
            return Preflight::Failed(DownloadError::ManagerStopped);
        };

        let existing_size = match file_size(&spec.destination).await {
            Ok(Some(size)) => size,
            Ok(None) => return Preflight::Start,
            Err(error) => return Preflight::Failed(error),
        };

        // If the catalog did not record a size, ask the server once so a
        // complete file can be skipped instead of prompting or renaming.
        if spec.expected_size.is_none() && spec.conflict != ConflictPolicy::Overwrite {
            if let Some(remote_size) = probe_remote_size(
                &self.client,
                &spec.url,
                self.request_timeout.min(Duration::from_secs(10)),
            )
            .await
            {
                spec.expected_size = Some(remote_size);
                if let Some(job) = self.jobs.get_mut(&id) {
                    job.spec.expected_size = Some(remote_size);
                }
            }
        }

        if spec.conflict != ConflictPolicy::Overwrite && spec.expected_size == Some(existing_size) {
            return Preflight::Skipped(SkipReason::AlreadyDownloaded);
        }

        match spec.conflict {
            ConflictPolicy::Skip => Preflight::Skipped(SkipReason::DestinationExists),
            ConflictPolicy::Ask => Preflight::Conflict { existing_size },
            ConflictPolicy::Rename => {
                let new_destination = {
                    let claims = &self.claims;
                    next_available_path_with(&spec.destination, |candidate| {
                        claims.contains(candidate)
                            || candidate.exists()
                            || part_path(candidate).exists()
                    })
                };

                self.claims.remove(&spec.destination);
                self.claims.insert(new_destination.clone());
                if let Some(job) = self.jobs.get_mut(&id) {
                    job.spec.destination = new_destination;
                }
                Preflight::Start
            }
            ConflictPolicy::Overwrite => {
                match fs::remove_file(&spec.destination).await {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => {
                        return Preflight::Failed(io_error(&spec.destination, error));
                    }
                }
                let _ = fs::remove_file(part_path(&spec.destination)).await;
                Preflight::Start
            }
        }
    }

    async fn handle_command(&mut self, command: Command) {
        match command {
            Command::Enqueue { id, spec } => {
                let mut spec = spec;
                reserve_destination(&mut self.claims, &mut spec);
                let title = spec.title.clone();

                self.jobs.insert(
                    id,
                    Job {
                        spec,
                        state: JobState::Queued,
                    },
                );
                self.queue.push_back(id);
                self.emit(DownloadEvent::Queued { id, title }).await;
            }
            Command::Pause(id) => self.pause(id).await,
            Command::Resume(id) => self.resume(id).await,
            Command::Cancel(id) => self.cancel(id).await,
            Command::Retry(id) => self.retry(id).await,
            Command::ResolveConflict { id, resolution } => {
                self.resolve_conflict(id, resolution).await;
            }
            Command::Shutdown => {}
        }
    }

    async fn pause(&mut self, id: JobId) {
        let Some(state) = self.jobs.get(&id).map(|job| job.state) else {
            return;
        };

        match state {
            JobState::Queued => {
                self.queue.retain(|queued| *queued != id);
                let destination = self.jobs[&id].spec.destination.clone();
                let downloaded = file_size(&part_path(&destination))
                    .await
                    .ok()
                    .flatten()
                    .unwrap_or(0);

                if let Some(job) = self.jobs.get_mut(&id) {
                    job.state = JobState::Paused;
                }
                self.emit(DownloadEvent::Paused { id, downloaded }).await;
            }
            JobState::AwaitingConflict => {
                if let Some(job) = self.jobs.get_mut(&id) {
                    job.state = JobState::Paused;
                }
                self.emit(DownloadEvent::Paused { id, downloaded: 0 }).await;
            }
            JobState::Running => {
                if let Some(job) = self.jobs.get_mut(&id) {
                    job.state = JobState::Pausing;
                }
                if let Some(token) = self.cancels.get(&id) {
                    token.cancel();
                }
            }
            _ => {}
        }
    }

    async fn resume(&mut self, id: JobId) {
        let Some(state) = self.jobs.get(&id).map(|job| job.state) else {
            return;
        };

        if state != JobState::Paused {
            return;
        }

        let title = self.jobs[&id].spec.title.clone();
        if let Some(job) = self.jobs.get_mut(&id) {
            job.state = JobState::Queued;
        }
        self.queue.push_back(id);
        self.emit(DownloadEvent::Queued { id, title }).await;
    }

    async fn retry(&mut self, id: JobId) {
        let Some(state) = self.jobs.get(&id).map(|job| job.state) else {
            return;
        };

        if state != JobState::Failed {
            return;
        }

        let title = self.jobs[&id].spec.title.clone();
        if let Some(job) = self.jobs.get_mut(&id) {
            job.state = JobState::Queued;
        }
        self.queue.push_back(id);
        self.emit(DownloadEvent::Queued { id, title }).await;
    }

    async fn cancel(&mut self, id: JobId) {
        let Some(state) = self.jobs.get(&id).map(|job| job.state) else {
            return;
        };

        match state {
            JobState::Queued | JobState::Paused | JobState::Failed | JobState::AwaitingConflict => {
                self.queue.retain(|queued| *queued != id);
                let destination = self.jobs[&id].spec.destination.clone();
                let _ = fs::remove_file(part_path(&destination)).await;
                self.claims.remove(&destination);

                if let Some(job) = self.jobs.get_mut(&id) {
                    job.state = JobState::Cancelled;
                }
                self.emit(DownloadEvent::Cancelled { id }).await;
            }
            JobState::Running | JobState::Pausing => {
                if let Some(job) = self.jobs.get_mut(&id) {
                    job.state = JobState::Cancelling;
                }
                if let Some(token) = self.cancels.get(&id) {
                    token.cancel();
                }
            }
            _ => {}
        }
    }

    async fn resolve_conflict(&mut self, id: JobId, resolution: ConflictResolution) {
        let Some(state) = self.jobs.get(&id).map(|job| job.state) else {
            return;
        };

        if state != JobState::AwaitingConflict {
            return;
        }

        let destination = self.jobs[&id].spec.destination.clone();

        match resolution {
            ConflictResolution::Skip => {
                self.claims.remove(&destination);
                if let Some(job) = self.jobs.get_mut(&id) {
                    job.state = JobState::Skipped;
                }
                self.emit(DownloadEvent::Skipped {
                    id,
                    path: destination,
                    reason: SkipReason::DestinationExists,
                })
                .await;
            }
            ConflictResolution::Rename => {
                let new_destination = {
                    let claims = &self.claims;
                    next_available_path_with(&destination, |candidate| {
                        claims.contains(candidate)
                            || candidate.exists()
                            || part_path(candidate).exists()
                    })
                };

                self.claims.remove(&destination);
                self.claims.insert(new_destination.clone());
                let title = self.jobs[&id].spec.title.clone();
                if let Some(job) = self.jobs.get_mut(&id) {
                    job.spec.destination = new_destination;
                    job.state = JobState::Queued;
                }
                self.queue.push_back(id);
                self.emit(DownloadEvent::Queued { id, title }).await;
            }
            ConflictResolution::Overwrite => {
                match fs::remove_file(&destination).await {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => {
                        if let Some(job) = self.jobs.get_mut(&id) {
                            job.state = JobState::Failed;
                        }
                        self.emit(DownloadEvent::Failed {
                            id,
                            error: io_error(&destination, error),
                        })
                        .await;
                        return;
                    }
                }
                let _ = fs::remove_file(part_path(&destination)).await;
                let title = self.jobs[&id].spec.title.clone();
                if let Some(job) = self.jobs.get_mut(&id) {
                    job.state = JobState::Queued;
                }
                self.queue.push_back(id);
                self.emit(DownloadEvent::Queued { id, title }).await;
            }
            ConflictResolution::Cancel => {
                self.claims.remove(&destination);
                if let Some(job) = self.jobs.get_mut(&id) {
                    job.state = JobState::Cancelled;
                }
                self.emit(DownloadEvent::Cancelled { id }).await;
            }
        }
    }

    async fn on_complete(&mut self, joined: Result<(JobId, Outcome), tokio::task::JoinError>) {
        let Ok((id, outcome)) = joined else {
            tracing::error!("download worker panicked or was aborted");
            return;
        };

        self.active.remove(&id);
        self.cancels.remove(&id);

        let Some(state) = self.jobs.get(&id).map(|job| job.state) else {
            return;
        };
        let destination = self.jobs[&id].spec.destination.clone();

        match outcome {
            Outcome::Completed { bytes } => {
                self.claims.remove(&destination);
                if let Some(job) = self.jobs.get_mut(&id) {
                    job.state = JobState::Completed;
                }
                self.emit(DownloadEvent::Completed {
                    id,
                    path: destination,
                    bytes,
                })
                .await;
            }
            Outcome::Failed(error) => {
                if let Some(job) = self.jobs.get_mut(&id) {
                    job.state = JobState::Failed;
                }
                self.emit(DownloadEvent::Failed { id, error }).await;
            }
            Outcome::Interrupted => {
                if state == JobState::Cancelling {
                    let _ = fs::remove_file(part_path(&destination)).await;
                    self.claims.remove(&destination);
                    if let Some(job) = self.jobs.get_mut(&id) {
                        job.state = JobState::Cancelled;
                    }
                    self.emit(DownloadEvent::Cancelled { id }).await;
                } else {
                    let downloaded = file_size(&part_path(&destination))
                        .await
                        .ok()
                        .flatten()
                        .unwrap_or(0);
                    if let Some(job) = self.jobs.get_mut(&id) {
                        job.state = JobState::Paused;
                    }
                    self.emit(DownloadEvent::Paused { id, downloaded }).await;
                }
            }
        }
    }

    async fn emit(&self, event: DownloadEvent) {
        let _ = self.events.send(event).await;
    }

    async fn shutdown(&mut self) {
        for token in self.cancels.values() {
            token.cancel();
        }
        self.joins.abort_all();
        while self.joins.join_next().await.is_some() {}
    }
}

fn reserve_destination(claims: &mut HashSet<PathBuf>, spec: &mut DownloadSpec) {
    let base = spec.destination.clone();
    let destination = if claims.contains(&base) {
        next_available_path_with(&base, |candidate| {
            claims.contains(candidate) || candidate.exists() || part_path(candidate).exists()
        })
    } else {
        base
    };

    claims.insert(destination.clone());
    spec.destination = destination;
}

async fn run_job(
    id: JobId,
    spec: DownloadSpec,
    client: Client,
    events: mpsc::Sender<DownloadEvent>,
    cancel: CancellationToken,
    retries: u32,
    request_timeout: Duration,
) -> Outcome {
    if let Err(error) = validate_url(&spec.url) {
        return Outcome::Failed(error);
    }

    let part = part_path(&spec.destination);
    if let Some(parent) = spec.destination.parent() {
        if let Err(error) = fs::create_dir_all(parent).await {
            return Outcome::Failed(io_error(parent, error));
        }
    }

    let resumed_from = match fs::metadata(&part).await {
        Ok(metadata) if metadata.is_file() => metadata.len(),
        Ok(_) => 0,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
        Err(error) => return Outcome::Failed(io_error(&part, error)),
    };

    let mut tracker = ProgressTracker::new(resumed_from, spec.expected_size);
    send_event(
        &events,
        DownloadEvent::Started {
            id,
            destination: spec.destination.clone(),
            resumed_from,
            total: spec.expected_size,
        },
    )
    .await;

    let max_attempts = retries.saturating_add(1);
    let mut attempt = 0_u32;

    loop {
        attempt += 1;

        match download_once(
            id,
            &spec,
            &client,
            &events,
            &cancel,
            &mut tracker,
            request_timeout,
        )
        .await
        {
            Ok(bytes) => return Outcome::Completed { bytes },
            Err(JobError::Interrupted) => return Outcome::Interrupted,
            Err(JobError::Failed(error)) if error.is_retryable() && attempt < max_attempts => {
                let delay = retry_delay(attempt);
                send_event(
                    &events,
                    DownloadEvent::Retrying {
                        id,
                        attempt: attempt + 1,
                        max_attempts,
                        delay,
                        error: error.to_string(),
                    },
                )
                .await;

                tokio::select! {
                    () = cancel.cancelled() => return Outcome::Interrupted,
                    () = sleep(delay) => {}
                }
            }
            Err(JobError::Failed(error)) => return Outcome::Failed(error),
        }
    }
}

async fn download_once(
    id: JobId,
    spec: &DownloadSpec,
    client: &Client,
    events: &mpsc::Sender<DownloadEvent>,
    cancel: &CancellationToken,
    tracker: &mut ProgressTracker,
    request_timeout: Duration,
) -> Result<u64, JobError> {
    let part = part_path(&spec.destination);

    let mut offset = match fs::metadata(&part).await {
        Ok(metadata) if metadata.is_file() => metadata.len(),
        Ok(_) => 0,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
        Err(error) => return Err(JobError::Failed(io_error(&part, error))),
    };
    tracker.set_position(offset);

    let mut response = send_get(client, &spec.url, offset, cancel, request_timeout).await?;

    if response.status() == StatusCode::RANGE_NOT_SATISFIABLE && offset > 0 {
        OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&part)
            .await
            .map_err(|error| JobError::Failed(io_error(&part, error)))?;
        offset = 0;
        tracker.set_position(0);
        response = send_get(client, &spec.url, 0, cancel, request_timeout).await?;
    }

    if !response.status().is_success() {
        return Err(JobError::Failed(DownloadError::Http {
            status: response.status().as_u16(),
            url: spec.url.clone(),
        }));
    }

    if let Some(content_type) = response.headers().get(CONTENT_TYPE) {
        let content_type = content_type.to_str().unwrap_or_default();
        if !content_type_allows_pdf(content_type) {
            let _ = fs::remove_file(&part).await;
            return Err(JobError::Failed(DownloadError::InvalidContentType {
                url: spec.url.clone(),
                content_type: content_type.to_owned(),
            }));
        }
    }

    let append = offset > 0 && response.status() == StatusCode::PARTIAL_CONTENT;
    if !append {
        tracker.set_position(0);
    }

    let content_length = response.content_length();
    let total = content_length
        .map(|length| {
            if append {
                offset.saturating_add(length)
            } else {
                length
            }
        })
        .or(spec.expected_size);
    tracker.set_total(total);

    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .append(append)
        .truncate(!append)
        .open(&part)
        .await
        .map_err(|error| JobError::Failed(io_error(&part, error)))?;

    send_event(
        events,
        DownloadEvent::Progress {
            id,
            progress: tracker.snapshot(),
        },
    )
    .await;

    let mut last_emit = Instant::now();

    loop {
        let chunk = tokio::select! {
            () = cancel.cancelled() => {
                let _ = file.flush().await;
                return Err(JobError::Interrupted);
            }
            chunk = response.chunk() => chunk,
        };

        match chunk {
            Ok(Some(chunk)) => {
                file.write_all(&chunk)
                    .await
                    .map_err(|error| JobError::Failed(io_error(&part, error)))?;
                tracker.record(chunk.len() as u64);

                if last_emit.elapsed() >= PROGRESS_INTERVAL {
                    send_event(
                        events,
                        DownloadEvent::Progress {
                            id,
                            progress: tracker.snapshot(),
                        },
                    )
                    .await;
                    last_emit = Instant::now();
                }
            }
            Ok(None) => break,
            Err(error) => {
                return Err(JobError::Failed(map_reqwest_error(
                    error,
                    &spec.url,
                    request_timeout,
                )));
            }
        }
    }

    file.flush()
        .await
        .map_err(|error| JobError::Failed(io_error(&part, error)))?;
    drop(file);

    send_event(
        events,
        DownloadEvent::Progress {
            id,
            progress: tracker.snapshot(),
        },
    )
    .await;

    let final_size = match fs::metadata(&part).await {
        Ok(metadata) => metadata.len(),
        Err(error) => return Err(JobError::Failed(io_error(&part, error))),
    };

    if let Some(expected_total) = total {
        if final_size < expected_total {
            return Err(JobError::Failed(DownloadError::Network {
                message: format!(
                    "connection closed after {final_size} of {expected_total} bytes for {}",
                    spec.url
                ),
            }));
        }
    }

    if let Err(error) = validate_pdf(&part).await {
        let _ = fs::remove_file(&part).await;
        return Err(JobError::Failed(error));
    }

    publish(&part, &spec.destination)
        .await
        .map_err(JobError::Failed)?;

    tracker.set_position(final_size);
    Ok(final_size)
}

async fn send_get(
    client: &Client,
    url: &str,
    offset: u64,
    cancel: &CancellationToken,
    request_timeout: Duration,
) -> Result<Response, JobError> {
    let mut request = client.get(url).header(ACCEPT, ACCEPT_HEADER);
    if offset > 0 {
        request = request.header(RANGE, format!("bytes={offset}-"));
    }

    tokio::select! {
        () = cancel.cancelled() => Err(JobError::Interrupted),
        response = request.send() => response.map_err(|error| {
            JobError::Failed(map_reqwest_error(error, url, request_timeout))
        }),
    }
}

async fn publish(part: &Path, destination: &Path) -> Result<(), DownloadError> {
    // Hard links publish atomically and never overwrite an existing file.
    match fs::hard_link(part, destination).await {
        Ok(()) => {
            if let Err(error) = fs::remove_file(part).await {
                tracing::warn!(
                    part = %part.display(),
                    %error,
                    "could not remove temporary file after publishing"
                );
            }
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            Err(DownloadError::AlreadyExists {
                path: destination.to_path_buf(),
            })
        }
        Err(_) => {
            // Some filesystems do not support hard links. Fall back to a
            // no-clobber rename after re-checking the destination.
            if fs::metadata(destination).await.is_ok() {
                return Err(DownloadError::AlreadyExists {
                    path: destination.to_path_buf(),
                });
            }

            fs::rename(part, destination)
                .await
                .map_err(|error| io_error(destination, error))
        }
    }
}

async fn probe_remote_size(client: &Client, url: &str, timeout: Duration) -> Option<u64> {
    if let Some(size) = probe_head_size(client, url, timeout).await {
        return Some(size);
    }

    // Some portals answer HEAD with `Content-Length: 0` or 405. Ask for the
    // first byte instead and read the total from Content-Range.
    probe_range_size(client, url, timeout).await
}

async fn probe_head_size(client: &Client, url: &str, timeout: Duration) -> Option<u64> {
    let request = client.head(url).header(ACCEPT, ACCEPT_HEADER);
    let response = tokio::time::timeout(timeout, request.send())
        .await
        .ok()?
        .ok()?;

    if response.status().is_success() {
        // A zero here is more likely a proxy artefact than an empty PDF.
        response.content_length().filter(|size| *size > 0)
    } else {
        None
    }
}

async fn probe_range_size(client: &Client, url: &str, timeout: Duration) -> Option<u64> {
    let request = client
        .get(url)
        .header(ACCEPT, ACCEPT_HEADER)
        .header(RANGE, "bytes=0-0");
    let response = tokio::time::timeout(timeout, request.send())
        .await
        .ok()?
        .ok()?;

    if response.status() == StatusCode::PARTIAL_CONTENT {
        response
            .headers()
            .get(CONTENT_RANGE)
            .and_then(|value| value.to_str().ok())
            .and_then(parse_content_range_total)
    } else if response.status().is_success() {
        response.content_length()
    } else {
        None
    }
}

fn parse_content_range_total(value: &str) -> Option<u64> {
    value.rsplit_once('/')?.1.trim().parse().ok()
}

async fn file_size(path: &Path) -> Result<Option<u64>, DownloadError> {
    match fs::metadata(path).await {
        Ok(metadata) if metadata.is_file() => Ok(Some(metadata.len())),
        Ok(_) => Err(DownloadError::Io {
            path: path.to_path_buf(),
            message: "path exists but is not a regular file".to_owned(),
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(io_error(path, error)),
    }
}

fn io_error(path: &Path, error: std::io::Error) -> DownloadError {
    let message = error.to_string();

    if error.kind() == std::io::ErrorKind::PermissionDenied {
        DownloadError::PermissionDenied {
            path: path.to_path_buf(),
            message,
        }
    } else if is_disk_full(&error) {
        DownloadError::DiskFull {
            path: path.to_path_buf(),
            message,
        }
    } else {
        DownloadError::Io {
            path: path.to_path_buf(),
            message,
        }
    }
}

fn is_disk_full(error: &std::io::Error) -> bool {
    if matches!(
        error.raw_os_error(),
        Some(28 | 39 | 112 | 122) // ENOSPC/EDQUOT on Unix, disk-full on Windows
    ) {
        return true;
    }

    let message = error.to_string().to_ascii_lowercase();
    message.contains("no space left") || message.contains("disk full")
}

fn map_reqwest_error(error: reqwest::Error, url: &str, timeout: Duration) -> DownloadError {
    if error.is_timeout() {
        DownloadError::Timeout {
            seconds: timeout.as_secs().max(1),
        }
    } else if error.is_connect() {
        DownloadError::Network {
            message: format!("could not connect to {url}: {error}"),
        }
    } else {
        DownloadError::Network {
            message: format!("request to {url} failed: {error}"),
        }
    }
}

fn validate_url(value: &str) -> Result<(), DownloadError> {
    let parsed = url::Url::parse(value).map_err(|_| DownloadError::InvalidUrl {
        url: value.to_owned(),
    })?;

    if matches!(parsed.scheme(), "http" | "https") && parsed.host_str().is_some() {
        Ok(())
    } else {
        Err(DownloadError::InvalidUrl {
            url: value.to_owned(),
        })
    }
}

fn retry_delay(attempt: u32) -> Duration {
    let exponent = attempt.saturating_sub(1).min(4);
    let millis = RETRY_BASE_MILLIS.saturating_mul(1_u64 << exponent);
    Duration::from_millis(millis).min(MAX_RETRY_DELAY)
}

async fn send_event(events: &mpsc::Sender<DownloadEvent>, event: DownloadEvent) {
    let _ = events.send(event).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(destination: PathBuf) -> DownloadSpec {
        DownloadSpec {
            url: "https://example.com/file.pdf".to_owned(),
            destination,
            title: "File".to_owned(),
            source: "Test".to_owned(),
            expected_size: None,
            conflict: ConflictPolicy::Ask,
        }
    }

    #[test]
    fn retry_delays_grow_and_are_capped() {
        assert_eq!(retry_delay(1), Duration::from_millis(500));
        assert_eq!(retry_delay(2), Duration::from_secs(1));
        assert_eq!(retry_delay(3), Duration::from_secs(2));
        assert_eq!(retry_delay(10), MAX_RETRY_DELAY);
    }

    #[test]
    fn claims_force_a_numbered_destination() {
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("paper.pdf");
        let mut claims = HashSet::new();
        let mut first = spec(destination.clone());
        reserve_destination(&mut claims, &mut first);
        assert_eq!(first.destination, destination);

        let mut second = spec(destination);
        reserve_destination(&mut claims, &mut second);
        assert_eq!(second.destination, directory.path().join("paper (1).pdf"));
    }

    #[test]
    fn http_errors_are_retryable_only_when_transient() {
        let retryable = DownloadError::Http {
            status: 503,
            url: "https://example.com".to_owned(),
        };
        let permanent = DownloadError::Http {
            status: 404,
            url: "https://example.com".to_owned(),
        };
        assert!(retryable.is_retryable());
        assert!(!permanent.is_retryable());
        assert!(!DownloadError::NotPdf {
            path: PathBuf::from("x.pdf")
        }
        .is_retryable());
    }

    #[test]
    fn url_validation_rejects_non_http_schemes() {
        assert!(validate_url("https://example.com/a.pdf").is_ok());
        assert!(validate_url("ftp://example.com/a.pdf").is_err());
        assert!(validate_url("not a url").is_err());
    }

    #[test]
    fn disk_full_detection_handles_platform_codes() {
        #[cfg(unix)]
        {
            let error = std::io::Error::from_raw_os_error(28);
            assert!(is_disk_full(&error));
        }

        let error = std::io::Error::other("No space left on device");
        assert!(is_disk_full(&error));
    }
}
