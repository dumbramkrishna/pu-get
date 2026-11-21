//! Command-line surface and dispatch for `pu-get`.
//!
//! Parsing lives here so the binary entry point stays tiny. Every command is
//! implemented; the TUI is launched through `browse`.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io::{self, IsTerminal, Write as _};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use clap::{ArgAction, Args, Parser, Subcommand};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tracing_subscriber::EnvFilter;
use url::Url;

use crate::catalog::{
    BoardCatalog, BoardInfo, Catalog, CatalogEntry, Entry, Filter, FlatEntry, MaterialType,
};
use crate::config::{AppPaths, Config, ConfigKey, Language, MAX_CONCURRENCY, MIN_CONCURRENCY};
use crate::download::{
    spawn, ConflictPolicy, ConflictResolution, DownloadEvent, DownloadSpec, JobId, ManagerConfig,
    ProgressSnapshot,
};
use crate::resolver::{self, Registry};
use crate::verify::{self, CheckStatus, VerifyResult, VerifyTarget};

/// Browse and download official study material from the terminal.
#[derive(Debug, Parser)]
#[command(
    name = "pu-get",
    version,
    about = "Browse and download official study material for Karnataka and CBSE/NCERT boards",
    long_about = "pu-get lets students pick a board and level, browse a curated catalog of \
                  textbooks, model papers, previous year papers, sample papers, marking \
                  schemes and syllabi, then download the exact PDF they need.\n\n\
                  Run `pu-get` with no subcommand to open the interactive terminal UI.",
    propagate_version = true
)]
pub struct Cli {
    /// Use an alternative configuration file.
    #[arg(long, global = true, value_name = "PATH", env = "PU_GET_CONFIG")]
    pub config: Option<PathBuf>,

    /// Increase log verbosity (-v info, -vv debug, -vvv trace).
    #[arg(short, long, global = true, action = ArgAction::Count)]
    pub verbose: u8,

    /// Disable coloured output. The NO_COLOR environment variable is also honoured.
    #[arg(long, global = true)]
    pub no_color: bool,

    #[command(subcommand)]
    pub command: Option<Command>,
}

/// Top-level subcommands.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Launch the interactive terminal browser (also available as `tui`).
    #[command(visible_alias = "tui")]
    Browse,

    /// List matching catalog entries without downloading anything.
    List(ListArgs),

    /// Download matching catalog entries.
    Get(GetArgs),

    /// Inspect or change pu-get configuration.
    Config(ConfigArgs),

    /// Crawl official portals and regenerate the embedded catalogs.
    Crawl(CrawlArgs),

    /// Validate catalog URLs with HTTP HEAD requests.
    Verify(VerifyArgs),

    /// Fetch the latest remotely published catalog.
    Update(UpdateArgs),
}

/// Filters shared by `list`, `get` and `verify`.
#[derive(Debug, Clone, Default, Args)]
pub struct FilterArgs {
    /// Restrict to a board id (for example kseeb, pue, cbse or ncert).
    #[arg(long, value_name = "BOARD")]
    pub board: Option<String>,

    /// Restrict to a level ("SSLC", "PUC 1", "PUC 2" or 1-12).
    #[arg(long, value_name = "LEVEL")]
    pub level: Option<String>,

    /// Restrict to a stream (Science, Commerce, Arts).
    #[arg(long, value_name = "STREAM")]
    pub stream: Option<String>,

    /// Restrict to a subject (case-insensitive match).
    #[arg(long, value_name = "SUBJECT")]
    pub subject: Option<String>,

    /// Restrict to a material type: textbook, model_paper, previous_year_paper,
    /// sample_paper, marking_scheme or syllabus.
    #[arg(long = "type", value_name = "TYPE", value_parser = parse_material_type)]
    pub kind: Option<MaterialType>,

    /// Restrict to a publication or exam year, for example 2025.
    #[arg(long, value_name = "YEAR")]
    pub year: Option<u16>,

    /// Restrict to a language code: en, kn, hi or ur.
    #[arg(long, value_name = "LANG", value_parser = parse_language)]
    pub language: Option<Language>,

    /// Free-text match across title, subject and source.
    #[arg(long, value_name = "TEXT")]
    pub search: Option<String>,
}

/// Arguments for `pu-get list`.
#[derive(Debug, Args)]
pub struct ListArgs {
    #[command(flatten)]
    pub filters: FilterArgs,

    /// Keep only the newest entry per subject, type and language.
    #[arg(long)]
    pub latest: bool,

    /// Show at most N entries.
    #[arg(long, value_name = "N")]
    pub limit: Option<usize>,

    /// Emit machine-readable JSON instead of a table.
    #[arg(long)]
    pub json: bool,
}

/// Arguments for `pu-get get`.
#[derive(Debug, Args)]
#[command(after_help = "Examples:\n  \
    pu-get get --board cbse --level 12 --subject Chemistry --type textbook\n  \
    pu-get get --board kseeb --level SSLC --type model_paper --latest --out ./papers")]
pub struct GetArgs {
    #[command(flatten)]
    pub filters: FilterArgs,

    /// Override download_dir for this run.
    #[arg(long, short, value_name = "DIR")]
    pub out: Option<PathBuf>,

    /// How to handle an existing destination: ask, skip, rename or overwrite.
    /// Non-interactive runs default to rename instead of prompting.
    #[arg(
        long,
        value_name = "MODE",
        default_value = "ask",
        value_parser = parse_conflict_policy
    )]
    pub conflict: ConflictPolicy,

    /// Override the configured concurrency for this run (1-16).
    #[arg(long, value_name = "N", value_parser = parse_concurrency)]
    pub concurrency: Option<usize>,

    /// Keep only the newest entry per subject, type and language.
    #[arg(long)]
    pub latest: bool,

    /// Download at most N files.
    #[arg(long, value_name = "N")]
    pub limit: Option<usize>,

    /// Do not prompt for confirmation before downloading several files.
    #[arg(long, short = 'y')]
    pub yes: bool,

    /// Print the resolved destination paths and exit without downloading.
    #[arg(long)]
    pub dry_run: bool,
}

/// Arguments for `pu-get config`.
#[derive(Debug, Args)]
#[command(after_help = "Keys:\n  \
    download_dir, folder_template, language, concurrency, latest_only,\n  \
    update_url, request_timeout_secs, retries\n\n\
    Examples:\n  \
    pu-get config set download_dir ~/Study\n  \
    pu-get config set language kn\n  \
    pu-get config set latest_only false")]
pub struct ConfigArgs {
    #[command(subcommand)]
    pub command: ConfigCommand,
}

/// Configuration subcommands.
#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    /// Print the configuration file path.
    Path,

    /// Print the effective configuration as TOML (or JSON).
    Show {
        /// Emit JSON instead of TOML.
        #[arg(long)]
        json: bool,
    },

    /// List every configuration key and its current value.
    List,

    /// Print one configuration value.
    Get {
        /// Configuration key name.
        #[arg(value_name = "KEY", value_parser = parse_config_key)]
        key: ConfigKey,
    },

    /// Set and persist one configuration value.
    Set {
        /// Configuration key name.
        #[arg(value_name = "KEY", value_parser = parse_config_key)]
        key: ConfigKey,
        /// New value. Empty string clears optional values such as update_url.
        #[arg(value_name = "VALUE", allow_hyphen_values = true)]
        value: String,
    },
}

/// Arguments for `pu-get crawl`.
#[derive(Debug, Args)]
pub struct CrawlArgs {
    /// Crawl only these source ids (repeatable; default: every enabled source).
    #[arg(long = "source", value_name = "ID")]
    pub sources: Vec<String>,

    /// Output directory for generated catalogs (default: sources/generated).
    #[arg(long, value_name = "DIR")]
    pub out: Option<PathBuf>,

    /// Skip HTTP validation of discovered URLs.
    #[arg(long)]
    pub no_validate: bool,

    /// Report what would be generated without writing any files.
    #[arg(long)]
    pub dry_run: bool,

    /// Include unverified sources (verified sources are used by default).
    #[arg(long)]
    pub include_unverified: bool,

    /// Simultaneous validation requests (1-16).
    #[arg(long, value_name = "N", default_value_t = 4, value_parser = parse_concurrency)]
    pub concurrency: usize,
}

/// Arguments for `pu-get verify`.
#[derive(Debug, Args)]
pub struct VerifyArgs {
    #[command(flatten)]
    pub filters: FilterArgs,

    /// Write a validation report to this path.
    #[arg(long, value_name = "PATH")]
    pub report: Option<PathBuf>,

    /// Simultaneous HEAD requests (1-16).
    #[arg(long, value_name = "N", default_value_t = 4, value_parser = parse_concurrency)]
    pub concurrency: usize,

    /// Check at most N entries.
    #[arg(long, value_name = "N")]
    pub limit: Option<usize>,
}

/// Arguments for `pu-get update`.
#[derive(Debug, Args)]
pub struct UpdateArgs {
    /// Manifest URL for this run, overriding config update_url.
    #[arg(long, value_name = "URL")]
    pub remote: Option<String>,

    /// Only report whether a newer catalog is available.
    #[arg(long)]
    pub check: bool,

    /// Reinstall even when the remote version is unchanged.
    #[arg(long)]
    pub force: bool,
}

/// Parse, load configuration and dispatch a parsed command line.
pub async fn run(cli: Cli) -> Result<()> {
    let paths = AppPaths::discover(cli.config.as_deref())?;

    let colours = colors_enabled(cli.no_color);

    match cli.command.unwrap_or(Command::Browse) {
        Command::Browse => {
            let config = Config::load(&paths)?;
            crate::app::run(paths, config, colours).await
        }
        Command::List(args) => {
            let _config = Config::load(&paths)?;
            run_list(args, &paths)
        }
        Command::Get(args) => {
            let config = Config::load(&paths)?;
            run_get(args, &config, &paths).await
        }
        Command::Crawl(args) => {
            let config = Config::load(&paths)?;
            run_crawl(args, &config).await
        }
        Command::Verify(args) => {
            let config = Config::load(&paths)?;
            run_verify(args, &config, &paths).await
        }
        Command::Update(args) => {
            let config = Config::load(&paths)?;
            run_update(args, &config, &paths).await
        }
        Command::Config(args) => run_config(args, &paths),
    }
}

/// Initialise tracing on stderr. Logs go to stderr so piping stdout stays
/// clean for the non-interactive commands.
///
/// `interactive` disables logging entirely: writes to stderr would corrupt the
/// alternate screen while the TUI is running. Errors are still surfaced through
/// the status bar and the final `anyhow` report.
pub fn init_tracing(verbose: u8, interactive: bool) {
    let filter = if interactive {
        EnvFilter::new("off")
    } else {
        let default_level = match verbose {
            0 => "warn",
            1 => "info",
            2 => "debug",
            _ => "trace",
        };
        EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| EnvFilter::new(format!("pu_get={default_level}")))
    };
    let ansi = std::io::stderr().is_terminal() && std::env::var_os("NO_COLOR").is_none();

    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .with_ansi(ansi)
        .with_writer(std::io::stderr)
        .try_init();
}

/// Whether coloured output is allowed, honouring `--no-color` and `NO_COLOR`.
#[must_use]
pub fn colors_enabled(no_color_flag: bool) -> bool {
    !no_color_flag && std::env::var_os("NO_COLOR").is_none()
}

fn run_config(args: ConfigArgs, paths: &AppPaths) -> Result<()> {
    match args.command {
        ConfigCommand::Path => {
            println!("{}", paths.config_file.display());
        }
        ConfigCommand::Show { json } => {
            let config = Config::load(paths)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&config)?);
            } else {
                print!("{}", toml::to_string_pretty(&config)?);
            }
        }
        ConfigCommand::List => {
            let config = Config::load(paths)?;
            for key in ConfigKey::ALL {
                println!(
                    "{:<20} {:<44} # {}",
                    key.as_str(),
                    config.get(key),
                    key.description()
                );
            }
        }
        ConfigCommand::Get { key } => {
            let config = Config::load(paths)?;
            println!("{}", config.get(key));
        }
        ConfigCommand::Set { key, value } => {
            let mut config = Config::load(paths)?;
            config.set(key, &value)?;
            config.save(paths)?;
            println!("{} = {}", key.as_str(), config.get(key));
        }
    }

    Ok(())
}

fn run_list(args: ListArgs, paths: &AppPaths) -> Result<()> {
    let catalog = Catalog::load(paths)?;
    let filter = filter_from_args(&args.filters, args.latest);
    let mut entries = filter.apply(&catalog);

    if let Some(limit) = args.limit {
        entries.truncate(limit);
    }

    if args.json {
        let flat: Vec<FlatEntry<'_>> = entries.iter().map(|entry| entry.flat()).collect();
        println!("{}", serde_json::to_string_pretty(&flat)?);
        return Ok(());
    }

    if entries.is_empty() {
        eprintln!("No matching entries.");
        return Ok(());
    }

    print_table(&entries);
    Ok(())
}

async fn run_get(args: GetArgs, config: &Config, paths: &AppPaths) -> Result<()> {
    let catalog = Catalog::load(paths)?;
    let filter = filter_from_args(&args.filters, args.latest);
    let mut entries = filter.apply(&catalog);

    if let Some(limit) = args.limit {
        entries.truncate(limit);
    }

    if entries.is_empty() {
        anyhow::bail!("no entries matched the given filters");
    }

    let root = args
        .out
        .clone()
        .unwrap_or_else(|| config.resolved_download_dir());

    let mut specs = Vec::with_capacity(entries.len());
    for item in &entries {
        specs.push(DownloadSpec::from_entry(
            *item,
            &root,
            &config.folder_template,
            args.conflict,
        )?);
    }

    if args.dry_run {
        for spec in &specs {
            println!("{}\t{}", spec.destination.display(), spec.title);
        }
        return Ok(());
    }

    if !io::stdin().is_terminal() {
        if specs.iter().any(|spec| spec.destination.exists()) {
            eprintln!(
                "note: stdin is not a terminal; existing files will be renamed instead of prompting"
            );
        }
        for spec in &mut specs {
            if spec.conflict == ConflictPolicy::Ask {
                spec.conflict = ConflictPolicy::Rename;
            }
        }
    }

    if specs.len() > 1 && !args.yes && io::stdin().is_terminal() {
        let question = format!("Download {} file(s) to {}?", specs.len(), root.display());
        if !confirm(&question)? {
            println!("Cancelled.");
            return Ok(());
        }
    }

    let manager_config = ManagerConfig {
        concurrency: args.concurrency.unwrap_or(config.concurrency),
        retries: config.retries,
        request_timeout: Duration::from_secs(config.request_timeout_secs),
    };
    let (handle, mut events) = spawn(manager_config)?;

    let progress = MultiProgress::new();
    let (bar_style, pending_style) = progress_styles()?;
    let mut bars: HashMap<JobId, ProgressBar> = HashMap::new();
    let mut titles: HashMap<JobId, String> = HashMap::new();
    let mut remaining: HashSet<JobId> = HashSet::new();

    for spec in &specs {
        let id = handle.enqueue(spec.clone())?;
        let bar = progress.add(ProgressBar::new(spec.expected_size.unwrap_or(0)));
        if spec.expected_size.is_some() {
            bar.set_style(bar_style.clone());
        } else {
            bar.set_style(pending_style.clone());
        }
        bar.set_message(spec.title.clone());

        bars.insert(id, bar);
        titles.insert(id, spec.title.clone());
        remaining.insert(id);
    }

    let mut failures = 0_usize;

    while !remaining.is_empty() {
        let Some(event) = events.recv().await else {
            anyhow::bail!("download manager stopped unexpectedly");
        };

        match event {
            DownloadEvent::Queued { id, title } => {
                if let Some(bar) = bars.get(&id) {
                    bar.set_message(title);
                }
            }
            DownloadEvent::Started {
                id,
                resumed_from,
                total,
                ..
            } => {
                if let Some(bar) = bars.get(&id) {
                    match total {
                        Some(total) => {
                            bar.set_style(bar_style.clone());
                            bar.set_length(total);
                        }
                        None => bar.set_style(pending_style.clone()),
                    }
                    bar.set_position(resumed_from);
                }
            }
            DownloadEvent::Progress { id, progress } => {
                if let Some(bar) = bars.get(&id) {
                    set_bar_progress(bar, progress, &bar_style);
                }
            }
            DownloadEvent::Retrying {
                id,
                attempt,
                max_attempts,
                delay,
                error,
            } => {
                if let Some(bar) = bars.get(&id) {
                    let title = titles.get(&id).map_or("download", String::as_str);
                    bar.set_message(format!(
                        "{title} - retrying {attempt}/{max_attempts} in {:.1}s: {error}",
                        delay.as_secs_f64()
                    ));
                }
            }
            DownloadEvent::Paused { id, downloaded } => {
                if let Some(bar) = bars.get(&id) {
                    bar.finish_with_message(format!(
                        "paused ({} downloaded)",
                        human_bytes(downloaded)
                    ));
                }
                remaining.remove(&id);
            }
            DownloadEvent::Conflict {
                id,
                path,
                existing_size,
                expected_size,
            } => {
                let resolution =
                    progress.suspend(|| prompt_conflict(&path, existing_size, expected_size))?;
                handle.resolve_conflict(id, resolution)?;
            }
            DownloadEvent::Skipped { id, path, reason } => {
                if let Some(bar) = bars.get(&id) {
                    bar.finish_with_message(format!(
                        "skipped: {} ({})",
                        reason.message(),
                        path.display()
                    ));
                }
                remaining.remove(&id);
            }
            DownloadEvent::Completed { id, path, bytes } => {
                if let Some(bar) = bars.get(&id) {
                    bar.set_position(bytes);
                    bar.finish_with_message(format!("done: {}", path.display()));
                }
                remaining.remove(&id);
            }
            DownloadEvent::Failed { id, error } => {
                let title = titles.get(&id).map_or("download", String::as_str);
                progress.println(format!("failed: {title}: {error}"))?;
                if let Some(bar) = bars.get(&id) {
                    bar.finish_with_message("failed");
                }
                remaining.remove(&id);
                failures += 1;
            }
            DownloadEvent::Cancelled { id } => {
                if let Some(bar) = bars.get(&id) {
                    bar.finish_with_message("cancelled");
                }
                remaining.remove(&id);
            }
        }
    }

    let _ = handle.shutdown();

    if failures > 0 {
        anyhow::bail!("{failures} download(s) failed");
    }

    Ok(())
}

fn set_bar_progress(bar: &ProgressBar, progress: ProgressSnapshot, bar_style: &ProgressStyle) {
    if let Some(total) = progress.total {
        bar.set_style(bar_style.clone());
        bar.set_length(total);
    }
    bar.set_position(progress.downloaded);
}

fn progress_styles() -> Result<(ProgressStyle, ProgressStyle)> {
    let bar = ProgressStyle::with_template(
        "{msg}\n  {bar:32} {bytes}/{total_bytes} {bytes_per_sec} {eta}",
    )?
    .progress_chars("=>-");
    let pending = ProgressStyle::with_template("{msg}\n  {bytes} {bytes_per_sec}")?;
    Ok((bar, pending))
}

fn confirm(question: &str) -> io::Result<bool> {
    eprint!("{question} [y/N] ");
    io::stderr().flush()?;

    let mut line = String::new();
    if io::stdin().read_line(&mut line)? == 0 {
        return Ok(false);
    }

    Ok(matches!(
        line.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}

fn prompt_conflict(
    path: &Path,
    existing_size: u64,
    expected_size: Option<u64>,
) -> io::Result<ConflictResolution> {
    loop {
        eprint!(
            "Destination exists: {}\n  existing: {}\n  expected: {}\n  [s]kip, [r]ename, [o]verwrite, [c]ancel? ",
            path.display(),
            human_bytes(existing_size),
            expected_size.map_or_else(|| "unknown".to_owned(), human_bytes)
        );
        io::stderr().flush()?;

        let mut line = String::new();
        if io::stdin().read_line(&mut line)? == 0 {
            return Ok(ConflictResolution::Skip);
        }

        match line.trim().to_ascii_lowercase().as_str() {
            "s" | "skip" | "" => return Ok(ConflictResolution::Skip),
            "r" | "rename" => return Ok(ConflictResolution::Rename),
            "o" | "overwrite" => return Ok(ConflictResolution::Overwrite),
            "c" | "cancel" | "q" => return Ok(ConflictResolution::Cancel),
            _ => eprintln!("Please answer s, r, o or c."),
        }
    }
}

fn human_bytes(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = KIB * 1024.0;
    const GIB: f64 = MIB * 1024.0;

    let value = bytes as f64;
    if value >= GIB {
        format!("{:.1} GiB", value / GIB)
    } else if value >= MIB {
        format!("{:.1} MiB", value / MIB)
    } else if value >= KIB {
        format!("{:.1} KiB", value / KIB)
    } else {
        format!("{bytes} B")
    }
}

struct SourceReport {
    id: String,
    name: String,
    status: String,
    board: String,
    found: usize,
    kept: usize,
    dropped: Vec<(String, String)>,
    notes: Vec<String>,
    error: Option<String>,
    duration: Duration,
}

async fn run_crawl(args: CrawlArgs, config: &Config) -> Result<()> {
    let registry = Registry::load(None)?;
    let sources = registry.select(&args.sources, args.include_unverified)?;
    if sources.is_empty() {
        anyhow::bail!("no sources selected");
    }

    let client = verify::build_client(Duration::from_secs(config.request_timeout_secs.max(30)))?;
    let started_unix = now_unix();
    let mut reports = Vec::new();
    let mut boards: BTreeMap<String, Vec<Entry>> = BTreeMap::new();
    let mut successful_boards = HashSet::new();

    for source in &sources {
        let start = Instant::now();
        let outcome = resolver::resolve_source(&client, source).await;

        match outcome {
            Ok(mut resolved) => {
                resolved.dedupe();
                let found = resolved.entries.len();
                let mut dropped = Vec::new();

                let kept = if args.no_validate {
                    resolved
                        .notes
                        .push("URL validation skipped (--no-validate)".to_owned());
                    resolved.entries
                } else {
                    let mut unique = Vec::new();
                    let mut seen = HashSet::new();
                    for entry in &resolved.entries {
                        if seen.insert(entry.url.clone()) {
                            unique.push(entry.url.clone());
                        }
                    }

                    let checks = verify::check_many(&client, &unique, args.concurrency).await;
                    let statuses: HashMap<String, CheckStatus> = checks.into_iter().collect();

                    let mut kept = Vec::new();
                    for entry in resolved.entries {
                        match statuses.get(&entry.url) {
                            Some(status) if status.is_ok() => kept.push(entry),
                            Some(status) => {
                                dropped.push((entry.url.clone(), status.label()));
                            }
                            None => {
                                dropped.push((entry.url.clone(), "FAIL (not checked)".to_owned()))
                            }
                        }
                    }
                    kept
                };

                let kept_count = kept.len();
                successful_boards.insert(source.board.clone());
                boards.entry(source.board.clone()).or_default().extend(kept);

                reports.push(SourceReport {
                    id: source.id.clone(),
                    name: source.name.clone(),
                    status: source.status.as_str().to_owned(),
                    board: source.board.clone(),
                    found,
                    kept: kept_count,
                    dropped,
                    notes: resolved.notes,
                    error: None,
                    duration: start.elapsed(),
                });
            }
            Err(error) => reports.push(SourceReport {
                id: source.id.clone(),
                name: source.name.clone(),
                status: source.status.as_str().to_owned(),
                board: source.board.clone(),
                found: 0,
                kept: 0,
                dropped: Vec::new(),
                notes: Vec::new(),
                error: Some(error.to_string()),
                duration: start.elapsed(),
            }),
        }
    }

    let out_dir = args
        .out
        .clone()
        .unwrap_or_else(|| PathBuf::from("sources/generated"));
    let header = format!(
        "# Generated by `pu-get crawl` on unix {started_unix} from {} source(s).\n\
         # Do not edit by hand; edit sources/registry.toml and re-run crawl.\n",
        reports.len()
    );
    let mut generated = Vec::new();
    let mut generation_errors = Vec::new();

    for (board_id, entries) in boards {
        let board = BoardCatalog {
            board: BoardInfo {
                id: board_id.clone(),
                name: resolver::board_name(&board_id),
            },
            entries,
        };

        match Catalog::from_boards(vec![board]) {
            Ok(catalog) => {
                let board = &catalog.boards()[0];
                let contents = toml::to_string_pretty(board)?;
                let path = out_dir.join(format!("{board_id}.toml"));

                if !args.dry_run {
                    atomic_write_bytes(&path, format!("{header}{contents}").as_bytes())?;
                }

                generated.push((board_id, path, board.entries.len()));
            }
            Err(error) => generation_errors.push(format!("{board_id}: {error}")),
        }
    }

    let unchanged: Vec<String> = reports
        .iter()
        .filter(|report| report.error.is_some() && !successful_boards.contains(&report.board))
        .map(|report| report.board.clone())
        .collect();

    let report_text = render_crawl_report(
        started_unix,
        &reports,
        &generated,
        &generation_errors,
        &unchanged,
    );

    let ok_count = reports
        .iter()
        .filter(|report| report.error.is_none())
        .count();
    let failed_count = reports.len() - ok_count;
    let kept_total: usize = reports.iter().map(|report| report.kept).sum();

    if args.dry_run {
        print!("{report_text}");
    } else {
        let report_path = out_dir.join("crawl-report.txt");
        atomic_write_bytes(&report_path, report_text.as_bytes())?;

        println!(
            "Crawled {} source(s): {} ok, {} failed; {} entries kept.",
            reports.len(),
            ok_count,
            failed_count,
            kept_total
        );
        for (board, path, count) in &generated {
            println!("  {board}: {count} entries -> {}", path.display());
        }
        println!("  report: {}", report_path.display());
    }

    if ok_count == 0 {
        anyhow::bail!("all selected sources failed; review the crawl report");
    }

    Ok(())
}

fn render_crawl_report(
    started_unix: u64,
    reports: &[SourceReport],
    generated: &[(String, PathBuf, usize)],
    generation_errors: &[String],
    unchanged: &[String],
) -> String {
    let ok_count = reports
        .iter()
        .filter(|report| report.error.is_none())
        .count();
    let mut output = format!(
        "pu-get crawl report\nunix_time: {started_unix}\nsources: {} selected, {} ok, {} failed\n",
        reports.len(),
        ok_count,
        reports.len() - ok_count
    );

    for report in reports {
        let outcome = if report.error.is_some() {
            "error"
        } else {
            "ok"
        };
        output.push_str(&format!(
            "\n[{outcome}] {} ({} {}) board={}\n  found={} kept={} dropped={} duration_ms={}\n",
            report.id,
            report.name,
            report.status,
            report.board,
            report.found,
            report.kept,
            report.dropped.len(),
            report.duration.as_millis()
        ));

        if let Some(error) = &report.error {
            output.push_str(&format!("  error: {error}\n"));
        }
        for (url, reason) in &report.dropped {
            output.push_str(&format!("  dropped: {url} ({reason})\n"));
        }
        for note in &report.notes {
            output.push_str(&format!("  note: {note}\n"));
        }
    }

    output.push_str("\ngenerated:\n");
    if generated.is_empty() {
        output.push_str("  (none)\n");
    }
    for (board, path, count) in generated {
        output.push_str(&format!(
            "  {board}: {count} entries -> {}\n",
            path.display()
        ));
    }

    if !generation_errors.is_empty() {
        output.push_str("\ngeneration errors:\n");
        for error in generation_errors {
            output.push_str(&format!("  {error}\n"));
        }
    }

    if !unchanged.is_empty() {
        output.push_str("\nleft unchanged (no successful source):\n");
        for board in unchanged {
            output.push_str(&format!("  {board}\n"));
        }
    }

    output
}

async fn run_verify(args: VerifyArgs, config: &Config, paths: &AppPaths) -> Result<()> {
    let catalog = Catalog::load(paths)?;
    let filter = filter_from_args(&args.filters, false);
    let entries = filter.apply(&catalog);

    let mut targets = Vec::new();
    let mut seen = HashSet::new();
    for item in entries {
        if seen.insert(item.entry.url.clone()) {
            targets.push(VerifyTarget {
                label: format!(
                    "{}/{} {} - {}",
                    item.board.id, item.entry.level, item.entry.subject, item.entry.title
                ),
                url: item.entry.url.clone(),
            });
        }
    }

    if let Some(limit) = args.limit {
        targets.truncate(limit);
    }

    if targets.is_empty() {
        eprintln!("No matching entries.");
        return Ok(());
    }

    let client = verify::build_client(Duration::from_secs(config.request_timeout_secs))?;
    let urls: Vec<String> = targets.iter().map(|target| target.url.clone()).collect();
    let checks = verify::check_many(&client, &urls, args.concurrency).await;
    let statuses: HashMap<String, CheckStatus> = checks.into_iter().collect();

    let mut results = Vec::new();
    let mut failures = 0_usize;

    for target in targets {
        let status = statuses
            .get(&target.url)
            .cloned()
            .unwrap_or(CheckStatus::Failed {
                reason: "not checked".to_owned(),
            });
        if !status.is_ok() {
            failures += 1;
        }
        println!("{:<18} {} <- {}", status.label(), target.url, target.label);
        results.push(VerifyResult { target, status });
    }

    if let Some(path) = args.report {
        let report = verify::render_report(&results);
        atomic_write_bytes(&path, report.as_bytes())?;
        println!("Report: {}", path.display());
    }

    println!(
        "{} checked, {} ok, {} failed",
        results.len(),
        results.len() - failures,
        failures
    );

    if failures > 0 {
        anyhow::bail!("{failures} URL(s) failed verification");
    }

    Ok(())
}

#[derive(Debug, Clone, Deserialize)]
struct UpdateManifest {
    schema_version: u32,
    catalog_version: String,
    catalog_url: String,
    sha256: String,
    #[serde(default)]
    generated_at: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct UpdateState {
    #[serde(default)]
    catalog_version: Option<String>,
    #[serde(default)]
    sha256: Option<String>,
    #[serde(default)]
    catalog_url: Option<String>,
    #[serde(default)]
    catalog_path: Option<PathBuf>,
    #[serde(default)]
    installed_at: Option<u64>,
}

async fn run_update(args: UpdateArgs, config: &Config, paths: &AppPaths) -> Result<()> {
    let raw_url = args
        .remote
        .clone()
        .or_else(|| config.update_url.clone())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "no update URL configured; run `pu-get config set update_url <manifest-url>` or pass --remote <url>"
            )
        })?;

    let manifest_url = Url::parse(&raw_url).context("invalid update manifest URL")?;
    if !matches!(manifest_url.scheme(), "http" | "https") || manifest_url.host_str().is_none() {
        anyhow::bail!("update manifest URL must be absolute http(s)");
    }

    let client = verify::build_client(Duration::from_secs(config.request_timeout_secs.max(30)))?;
    let manifest_bytes = fetch_bytes(&client, &manifest_url, 1024 * 1024).await?;
    let manifest = parse_manifest(manifest_url.as_str(), &manifest_bytes)?;
    validate_manifest(&manifest)?;

    let state = load_update_state(paths)?;
    let expected_sha = manifest.sha256.trim().to_ascii_lowercase();
    let up_to_date = state.catalog_version.as_deref() == Some(manifest.catalog_version.as_str())
        && state.sha256.as_deref().map(str::to_ascii_lowercase) == Some(expected_sha.clone())
        && state
            .catalog_path
            .as_ref()
            .is_some_and(|path| path.exists());

    if args.check {
        println!(
            "remote catalog: {} ({})",
            manifest.catalog_version, manifest_url
        );
        if let Some(generated_at) = &manifest.generated_at {
            println!("published: {generated_at}");
        }
        match &state.catalog_version {
            Some(version) => println!("installed catalog: {version}"),
            None => println!("installed catalog: none"),
        }
        println!(
            "{}",
            if up_to_date {
                "up to date"
            } else {
                "update available"
            }
        );
        return Ok(());
    }

    if up_to_date && !args.force {
        println!(
            "Catalog is up to date (version {}).",
            manifest.catalog_version
        );
        return Ok(());
    }

    let catalog_url = manifest_url
        .join(&manifest.catalog_url)
        .context("invalid catalog_url in update manifest")?;
    if !matches!(catalog_url.scheme(), "http" | "https") || catalog_url.host_str().is_none() {
        anyhow::bail!("catalog_url must resolve to an absolute http(s) URL");
    }

    let bytes = fetch_bytes(&client, &catalog_url, 32 * 1024 * 1024).await?;
    let actual_sha = sha256_hex(&bytes);
    if actual_sha != expected_sha {
        anyhow::bail!("catalog checksum mismatch: expected {expected_sha}, got {actual_sha}");
    }

    let text = std::str::from_utf8(&bytes).context("downloaded catalog is not UTF-8")?;
    let trimmed = text.trim_start();
    let (catalog, is_json) = if trimmed.starts_with('{') {
        (
            Catalog::from_json(catalog_url.as_str(), text).with_context(|| {
                format!("catalog downloaded from {catalog_url} failed validation")
            })?,
            true,
        )
    } else {
        match Catalog::from_toml(catalog_url.as_str(), text) {
            Ok(catalog) => (catalog, false),
            Err(toml_error) if trimmed.starts_with('[') => (
                Catalog::from_json(catalog_url.as_str(), text).with_context(|| {
                    format!(
                        "catalog downloaded from {catalog_url} is neither valid TOML ({toml_error}) nor JSON"
                    )
                })?,
                true,
            ),
            Err(toml_error) => {
                return Err(toml_error)
                    .with_context(|| format!("catalog downloaded from {catalog_url} failed validation"));
            }
        }
    };

    if catalog.is_empty() {
        anyhow::bail!("downloaded catalog contains no entries");
    }

    let extension = if is_json { "json" } else { "toml" };
    paths.ensure_config_dirs()?;
    let target = paths.catalog_dir.join(format!("remote.{extension}"));
    atomic_write_bytes(&target, &bytes)?;

    let stale = paths.catalog_dir.join(if is_json {
        "remote.toml"
    } else {
        "remote.json"
    });
    if stale.exists() {
        let _ = fs::remove_file(stale);
    }

    let state = UpdateState {
        catalog_version: Some(manifest.catalog_version.clone()),
        sha256: Some(actual_sha),
        catalog_url: Some(catalog_url.to_string()),
        catalog_path: Some(target.clone()),
        installed_at: Some(now_unix()),
    };
    save_update_state(paths, &state)?;

    println!(
        "Updated catalog to {} ({} entries) at {}",
        manifest.catalog_version,
        catalog.len(),
        target.display()
    );
    Ok(())
}

fn parse_manifest(origin: &str, bytes: &[u8]) -> Result<UpdateManifest> {
    let text = std::str::from_utf8(bytes).context("manifest is not UTF-8")?;

    match toml::from_str::<UpdateManifest>(text) {
        Ok(manifest) => Ok(manifest),
        Err(toml_error) => match serde_json::from_str::<UpdateManifest>(text) {
            Ok(manifest) => Ok(manifest),
            Err(json_error) => anyhow::bail!(
                "manifest `{origin}` is neither valid TOML ({toml_error}) nor JSON ({json_error})"
            ),
        },
    }
}

fn validate_manifest(manifest: &UpdateManifest) -> Result<()> {
    if manifest.schema_version != 1 {
        anyhow::bail!(
            "unsupported update manifest schema version {}",
            manifest.schema_version
        );
    }
    if manifest.catalog_version.trim().is_empty() {
        anyhow::bail!("update manifest has an empty catalog_version");
    }
    if manifest.catalog_url.trim().is_empty() {
        anyhow::bail!("update manifest has an empty catalog_url");
    }
    let sha = manifest.sha256.trim();
    if sha.len() != 64 || !sha.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        anyhow::bail!("update manifest sha256 must be 64 hexadecimal characters");
    }
    Ok(())
}

async fn fetch_bytes(client: &reqwest::Client, url: &Url, max_bytes: usize) -> Result<Vec<u8>> {
    let response = client
        .get(url.as_str())
        .send()
        .await
        .with_context(|| format!("could not fetch {url}"))?;

    if !response.status().is_success() {
        anyhow::bail!("HTTP {} for {url}", response.status());
    }

    if let Some(length) = response.content_length() {
        if length > max_bytes as u64 {
            anyhow::bail!("{url} is too large ({length} bytes)");
        }
    }

    let bytes = response
        .bytes()
        .await
        .with_context(|| format!("could not read {url}"))?;

    if bytes.len() > max_bytes {
        anyhow::bail!("{url} is too large ({} bytes)", bytes.len());
    }

    Ok(bytes.to_vec())
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

fn update_state_path(paths: &AppPaths) -> PathBuf {
    paths.state_dir.join("update-state.json")
}

fn load_update_state(paths: &AppPaths) -> Result<UpdateState> {
    let path = update_state_path(paths);
    match fs::read_to_string(&path) {
        Ok(contents) => match serde_json::from_str(&contents) {
            Ok(state) => Ok(state),
            Err(error) => {
                eprintln!(
                    "warning: ignoring malformed update state {}: {error}",
                    path.display()
                );
                Ok(UpdateState::default())
            }
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(UpdateState::default()),
        Err(error) => Err(error).with_context(|| format!("could not read {}", path.display())),
    }
}

fn save_update_state(paths: &AppPaths, state: &UpdateState) -> Result<()> {
    paths.ensure_cache_dirs()?;
    let contents = serde_json::to_vec_pretty(state)?;
    atomic_write_bytes(&update_state_path(paths), &contents)
}

fn atomic_write_bytes(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).with_context(|| format!("could not create {}", parent.display()))?;

    let mut temporary = tempfile::NamedTempFile::new_in(parent)
        .with_context(|| format!("could not create a temporary file in {}", parent.display()))?;
    temporary
        .write_all(bytes)
        .with_context(|| format!("could not write {}", path.display()))?;
    temporary
        .as_file()
        .sync_all()
        .with_context(|| format!("could not flush {}", path.display()))?;
    temporary
        .persist(path)
        .map_err(|error| anyhow::anyhow!("could not write {}: {}", path.display(), error.error))?;
    Ok(())
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

fn filter_from_args(filters: &FilterArgs, latest_only: bool) -> Filter {
    Filter {
        board: filters.board.clone(),
        level: filters.level.clone(),
        stream: filters.stream.clone(),
        subject: filters.subject.clone(),
        kind: filters.kind,
        year: filters.year,
        language: filters.language,
        search: filters.search.clone(),
        latest_only,
    }
}

fn print_table(entries: &[CatalogEntry<'_>]) {
    println!(
        "{:<8} {:<10} {:<10} {:<22} {:<22} {:<4} {:<4} TITLE",
        "BOARD", "LEVEL", "STREAM", "SUBJECT", "TYPE", "YEAR", "LANG"
    );

    for item in entries {
        let entry = item.entry;
        let year = entry
            .year
            .map_or_else(|| "-".to_owned(), |year| year.to_string());
        println!(
            "{:<8} {:<10} {:<10} {:<22} {:<22} {:<4} {:<4} {} [{}]",
            clip(&item.board.id, 8),
            clip(&entry.level, 10),
            clip(entry.stream.as_deref().unwrap_or("-"), 10),
            clip(&entry.subject, 22),
            clip(entry.kind.as_str(), 22),
            year,
            entry.language.code(),
            entry.title,
            entry.source
        );
    }
}

fn clip(value: &str, width: usize) -> String {
    if value.chars().count() <= width {
        return value.to_owned();
    }

    let mut clipped: String = value.chars().take(width.saturating_sub(3)).collect();
    clipped.push_str("...");
    clipped
}

fn parse_material_type(value: &str) -> Result<MaterialType, String> {
    value
        .parse::<MaterialType>()
        .map_err(|error| error.to_string())
}

fn parse_language(value: &str) -> Result<Language, String> {
    value.parse::<Language>().map_err(|error| error.to_string())
}

fn parse_config_key(value: &str) -> Result<ConfigKey, String> {
    value
        .parse::<ConfigKey>()
        .map_err(|error| error.to_string())
}

fn parse_concurrency(value: &str) -> Result<usize, String> {
    let concurrency: usize = value
        .parse()
        .map_err(|_| "expected a whole number".to_owned())?;

    if (MIN_CONCURRENCY..=MAX_CONCURRENCY).contains(&concurrency) {
        Ok(concurrency)
    } else {
        Err(format!(
            "must be between {MIN_CONCURRENCY} and {MAX_CONCURRENCY}"
        ))
    }
}

fn parse_conflict_policy(value: &str) -> Result<ConflictPolicy, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "ask" => Ok(ConflictPolicy::Ask),
        "skip" | "skip-existing" => Ok(ConflictPolicy::Skip),
        "rename" => Ok(ConflictPolicy::Rename),
        "overwrite" | "force" => Ok(ConflictPolicy::Overwrite),
        _ => Err("expected one of ask, skip, rename, overwrite".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn clap_definition_is_consistent() {
        Cli::command().debug_assert();
    }

    #[test]
    fn no_subcommand_means_browse() {
        let cli = Cli::try_parse_from(["pu-get"]).unwrap();
        assert!(cli.command.is_none());
    }

    #[test]
    fn parses_list_filters() {
        let cli = Cli::try_parse_from([
            "pu-get",
            "list",
            "--board",
            "kseeb",
            "--level",
            "SSLC",
            "--subject",
            "Mathematics",
            "--type",
            "model_paper",
            "--year",
            "2025",
            "--language",
            "kn",
            "--latest",
            "--json",
        ])
        .unwrap();

        match cli.command {
            Some(Command::List(args)) => {
                assert_eq!(args.filters.board.as_deref(), Some("kseeb"));
                assert_eq!(args.filters.level.as_deref(), Some("SSLC"));
                assert_eq!(args.filters.subject.as_deref(), Some("Mathematics"));
                assert_eq!(args.filters.kind, Some(MaterialType::ModelPaper));
                assert_eq!(args.filters.year, Some(2025));
                assert_eq!(args.filters.language, Some(Language::Kn));
                assert!(args.latest);
                assert!(args.json);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parses_config_set() {
        let cli =
            Cli::try_parse_from(["pu-get", "config", "set", "download_dir", "~/Study"]).unwrap();

        match cli.command {
            Some(Command::Config(ConfigArgs {
                command: ConfigCommand::Set { key, value },
            })) => {
                assert_eq!(key, ConfigKey::DownloadDir);
                assert_eq!(value, "~/Study");
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parses_get_conflict_policy() {
        let cli = Cli::try_parse_from(["pu-get", "get", "--board", "cbse", "--conflict", "rename"])
            .unwrap();

        match cli.command {
            Some(Command::Get(args)) => {
                assert_eq!(args.conflict, ConflictPolicy::Rename);
            }
            other => panic!("unexpected command: {other:?}"),
        }

        assert!(Cli::try_parse_from(["pu-get", "get", "--conflict", "explode"]).is_err());
    }

    #[test]
    fn accepts_tui_alias() {
        let cli = Cli::try_parse_from(["pu-get", "tui"]).unwrap();
        assert!(matches!(cli.command, Some(Command::Browse)));
    }

    #[test]
    fn rejects_unsupported_language() {
        assert!(Cli::try_parse_from(["pu-get", "list", "--language", "fr"]).is_err());
    }

    #[test]
    fn rejects_out_of_range_concurrency() {
        assert!(Cli::try_parse_from(["pu-get", "crawl", "--concurrency", "0"]).is_err());
        assert!(Cli::try_parse_from(["pu-get", "crawl", "--concurrency", "99"]).is_err());
    }

    #[test]
    fn accepts_global_no_color() {
        let cli = Cli::try_parse_from(["pu-get", "--no-color", "list"]).unwrap();
        assert!(cli.no_color);
    }

    #[test]
    fn parses_toml_and_json_update_manifests() {
        let sha = "0".repeat(64);
        let toml_manifest = format!(
            "schema_version = 1\ncatalog_version = \"2026.11.21\"\ncatalog_url = \"catalog.json\"\nsha256 = \"{sha}\"\n"
        );
        let parsed = parse_manifest("manifest.toml", toml_manifest.as_bytes()).unwrap();
        assert_eq!(parsed.catalog_version, "2026.11.21");
        validate_manifest(&parsed).unwrap();

        let json_manifest = format!(
            r#"{{ "schema_version": 1, "catalog_version": "2026.11.21", "catalog_url": "catalog.json", "sha256": "{sha}" }}"#
        );
        let parsed = parse_manifest("manifest.json", json_manifest.as_bytes()).unwrap();
        assert_eq!(parsed.catalog_url, "catalog.json");
    }

    #[test]
    fn rejects_bad_manifest_checksums() {
        let manifest = UpdateManifest {
            schema_version: 1,
            catalog_version: "1".to_owned(),
            catalog_url: "catalog.json".to_owned(),
            sha256: "not-a-checksum".to_owned(),
            generated_at: None,
        };
        assert!(validate_manifest(&manifest).is_err());
    }

    #[test]
    fn sha256_known_vector() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn update_state_round_trips() {
        let directory = tempfile::tempdir().unwrap();
        let paths = AppPaths::new(
            directory.path().join("config.toml"),
            directory.path().join("cache"),
        );
        let state = UpdateState {
            catalog_version: Some("2026.11.21".to_owned()),
            sha256: Some("abc".to_owned()),
            catalog_url: Some("https://example.com/catalog.json".to_owned()),
            catalog_path: Some(directory.path().join("remote.json")),
            installed_at: Some(1_700_000_000),
        };
        save_update_state(&paths, &state).unwrap();

        let loaded = load_update_state(&paths).unwrap();
        assert_eq!(loaded.catalog_version.as_deref(), Some("2026.11.21"));
        assert_eq!(loaded.installed_at, Some(1_700_000_000));
    }
}
