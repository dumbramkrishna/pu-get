//! Application state for the interactive browser.
//!
//! The state machine owns the catalog selection, browse filters, download
//! queue and settings. Rendering reads this state without mutating it; input
//! handling (in [`crate::app::events`]) is the only thing that moves between
//! screens.

use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::catalog::{canonical_level, Catalog, CatalogEntry, Entry, Filter, MaterialType};
use crate::config::{AppPaths, Config, ConfigKey, Language, MAX_CONCURRENCY, MIN_CONCURRENCY};
use crate::download::{
    ConflictPolicy, ConflictResolution, DownloadEvent, DownloadHandle, DownloadSpec, JobId,
    ProgressSnapshot,
};
use crate::error::DownloadError;

/// How long the splash screen stays up when no key is pressed.
const SPLASH_DURATION: Duration = Duration::from_millis(1_500);

/// How long informational status messages remain visible.
const STATUS_LIFETIME: Duration = Duration::from_secs(6);

/// One screen in the browser.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    /// Logo and version.
    Splash,
    /// Board selection.
    Board,
    /// Level / class selection.
    Level,
    /// Stream selection (only for boards that use streams).
    Stream,
    /// Subject selection.
    Subject,
    /// Material type selection.
    Material,
    /// The PDF list.
    Browse,
    /// Download queue.
    Queue,
    /// Persistent settings.
    Settings,
    /// Keyboard-shortcut overlay.
    Help,
}

impl Screen {
    /// Short label used in the header and empty states.
    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Self::Splash => "Welcome",
            Self::Board => "Boards",
            Self::Level => "Level",
            Self::Stream => "Stream",
            Self::Subject => "Subject",
            Self::Material => "Material",
            Self::Browse => "Browse",
            Self::Queue => "Downloads",
            Self::Settings => "Settings",
            Self::Help => "Help",
        }
    }
}

/// The path the user has chosen through the catalog.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Selection {
    /// Board id (`kseeb`, `pue`, `cbse`, `ncert`, ...).
    pub board: Option<String>,
    /// Raw level value from the catalog (`SSLC`, `PUC 1`, `12`, ...).
    pub level: Option<String>,
    /// Stream (`Science`, `Commerce`, `Arts`) when the board uses streams.
    pub stream: Option<String>,
    /// Subject name exactly as stored in the catalog.
    pub subject: Option<String>,
    /// Material type.
    pub kind: Option<MaterialType>,
}

/// One row on a selection screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OptionItem {
    /// Value stored in [`Selection`]. An empty value means "all".
    pub value: String,
    /// Human-readable label.
    pub label: String,
    /// Number of catalog entries behind this option.
    pub count: usize,
}

/// How a setting is edited.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingKind {
    /// Free-form text edited in a popup.
    Text,
    /// A fixed list of values cycled with Enter or Left/Right.
    Cycle,
    /// A bounded number adjusted with Left/Right.
    Number,
    /// A boolean toggled with Enter or Space.
    Toggle,
}

/// Settings exposed by the TUI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingField {
    /// Root download directory.
    DownloadDir,
    /// Folder layout template.
    FolderTemplate,
    /// Preferred language.
    Language,
    /// Parallel downloads.
    Concurrency,
    /// Browse with the latest-only filter.
    LatestOnly,
}

impl SettingField {
    /// Every setting, in display order.
    pub const ALL: [Self; 5] = [
        Self::DownloadDir,
        Self::FolderTemplate,
        Self::Concurrency,
        Self::Language,
        Self::LatestOnly,
    ];

    /// Row label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::DownloadDir => "Download directory",
            Self::FolderTemplate => "Folder template",
            Self::Language => "Language preference",
            Self::Concurrency => "Concurrency",
            Self::LatestOnly => "Latest only",
        }
    }

    /// Longer explanation shown beneath the list.
    #[must_use]
    pub const fn help(self) -> &'static str {
        match self {
            Self::DownloadDir => "Root folder for downloads. `~` is expanded.",
            Self::FolderTemplate => {
                "Placeholders: {board} {level} {subject} {type} {year} {language}."
            }
            Self::Language => "Matching files are listed first; other languages stay visible.",
            Self::Concurrency => "Parallel downloads (1-16). Applied when pu-get starts.",
            Self::LatestOnly => "Hide older editions when a subject has several years.",
        }
    }

    /// Configuration key written when the setting changes.
    #[must_use]
    pub const fn key(self) -> ConfigKey {
        match self {
            Self::DownloadDir => ConfigKey::DownloadDir,
            Self::FolderTemplate => ConfigKey::FolderTemplate,
            Self::Language => ConfigKey::Language,
            Self::Concurrency => ConfigKey::Concurrency,
            Self::LatestOnly => ConfigKey::LatestOnly,
        }
    }

    /// Editor used for this setting.
    #[must_use]
    pub const fn kind(self) -> SettingKind {
        match self {
            Self::DownloadDir | Self::FolderTemplate => SettingKind::Text,
            Self::Language => SettingKind::Cycle,
            Self::Concurrency => SettingKind::Number,
            Self::LatestOnly => SettingKind::Toggle,
        }
    }

    /// Current value rendered for the row.
    #[must_use]
    pub fn value(self, config: &Config) -> String {
        match self {
            Self::Language => config.language.display_name().to_owned(),
            Self::LatestOnly => {
                if config.latest_only {
                    "on".to_owned()
                } else {
                    "off".to_owned()
                }
            }
            _ => config.get(self.key()),
        }
    }
}

/// Lifecycle of one queued download as shown in the TUI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobStatus {
    /// Waiting for a free download slot.
    Queued,
    /// Transferring bytes.
    Running,
    /// User paused the job; the `.part` file is kept.
    Paused,
    /// A transient error triggered a retry.
    Retrying,
    /// The destination exists and needs a decision.
    Conflict,
    /// File validated and published.
    Completed,
    /// Permanent failure.
    Failed,
    /// Existing file skipped.
    Skipped,
    /// User cancelled.
    Cancelled,
}

impl JobStatus {
    /// Compact label for the queue screen.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Paused => "paused",
            Self::Retrying => "retrying",
            Self::Conflict => "conflict",
            Self::Completed => "done",
            Self::Failed => "failed",
            Self::Skipped => "skipped",
            Self::Cancelled => "cancelled",
        }
    }

    /// Whether the manager may still make progress without user action.
    #[must_use]
    pub const fn is_active(self) -> bool {
        matches!(
            self,
            Self::Queued | Self::Running | Self::Paused | Self::Retrying | Self::Conflict
        )
    }

    /// Whether the job reached a terminal state.
    #[must_use]
    pub const fn is_finished(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Failed | Self::Skipped | Self::Cancelled
        )
    }
}

/// UI-side view of one download job.
#[derive(Debug, Clone)]
pub struct JobView {
    /// Manager-assigned id.
    pub id: JobId,
    /// Entry title.
    pub title: String,
    /// Current state.
    pub status: JobStatus,
    /// Final destination, when known.
    pub destination: Option<PathBuf>,
    /// Latest progress sample.
    pub progress: ProgressSnapshot,
    /// Extra information (retry error, skip reason, saved path, ...).
    pub note: Option<String>,
}

impl JobView {
    fn new(id: JobId, title: String, destination: Option<PathBuf>) -> Self {
        Self {
            id,
            title,
            status: JobStatus::Queued,
            destination,
            progress: ProgressSnapshot::default(),
            note: None,
        }
    }
}

/// A destination collision waiting for a decision.
#[derive(Debug, Clone)]
pub struct ConflictPrompt {
    /// Job that hit the conflict.
    pub id: JobId,
    /// Existing destination.
    pub path: PathBuf,
    /// Size of the existing file.
    pub existing_size: u64,
    /// Size reported by the catalog, when known.
    pub expected_size: Option<u64>,
}

/// Download queue state.
#[derive(Debug, Default)]
pub struct QueueState {
    /// Selected job.
    pub cursor: usize,
    /// Jobs in enqueue order.
    pub jobs: Vec<JobView>,
    /// Pending conflict decisions.
    pub conflicts: VecDeque<ConflictPrompt>,
}

/// Settings screen state.
#[derive(Debug, Default)]
pub struct SettingsState {
    /// Selected field.
    pub cursor: usize,
    /// Whether a text field is being edited.
    pub editing: bool,
    /// Text editor buffer.
    pub buffer: String,
}

/// Complete TUI state.
pub struct App {
    /// Current screen.
    pub screen: Screen,
    /// Loaded catalog.
    pub catalog: Catalog,
    /// Persisted configuration.
    pub config: Config,
    /// Platform paths used for catalogs and the config file.
    pub paths: AppPaths,
    /// Whether colour is enabled.
    pub color: bool,
    /// Set by `q`, Ctrl-C or a fatal terminal error.
    pub should_quit: bool,
    /// Chosen catalog path.
    pub selection: Selection,
    /// Board cursor.
    pub board_cursor: usize,
    /// Level cursor.
    pub level_cursor: usize,
    /// Stream cursor.
    pub stream_cursor: usize,
    /// Subject cursor.
    pub subject_cursor: usize,
    /// Material type cursor.
    pub material_cursor: usize,
    /// PDF list cursor.
    pub browse_cursor: usize,
    /// URLs toggled with Space.
    pub selected: HashSet<String>,
    /// Fuzzy search query.
    pub search: String,
    /// Whether the search box currently receives keystrokes.
    pub search_active: bool,
    search_before: String,
    /// Year filter; `None` means every year.
    pub year_filter: Option<u16>,
    /// Cursor in the year picker (0 is "All years").
    pub year_cursor: usize,
    /// Whether the year picker overlay is open.
    pub year_picker_open: bool,
    /// Latest-only filter.
    pub latest_only: bool,
    /// Transient status message.
    pub status: String,
    /// Whether the status message is an error.
    pub status_error: bool,
    status_at: Instant,
    /// Splash start time.
    pub started_at: Instant,
    /// Download manager handle; absent in render-only tests.
    pub downloads: Option<DownloadHandle>,
    /// Download queue state.
    pub queue: QueueState,
    /// Settings state.
    pub settings: SettingsState,
    /// Screen to return to when help closes.
    pub help_return: Screen,
    /// Screen to return to when settings close.
    pub settings_return: Screen,
    /// Screen to return to when the queue closes.
    pub queue_return: Screen,
}

impl App {
    /// Build the initial state.
    #[must_use]
    pub fn new(catalog: Catalog, config: Config, paths: AppPaths, color: bool) -> Self {
        let latest_only = config.latest_only;
        Self {
            screen: Screen::Splash,
            catalog,
            config,
            paths,
            color,
            should_quit: false,
            selection: Selection::default(),
            board_cursor: 0,
            level_cursor: 0,
            stream_cursor: 0,
            subject_cursor: 0,
            material_cursor: 0,
            browse_cursor: 0,
            selected: HashSet::new(),
            search: String::new(),
            search_active: false,
            search_before: String::new(),
            year_filter: None,
            year_cursor: 0,
            year_picker_open: false,
            latest_only,
            status: String::new(),
            status_error: false,
            status_at: Instant::now(),
            started_at: Instant::now(),
            downloads: None,
            queue: QueueState::default(),
            settings: SettingsState::default(),
            help_return: Screen::Board,
            settings_return: Screen::Browse,
            queue_return: Screen::Browse,
        }
    }

    /// Attach the background download manager.
    pub fn attach_downloads(&mut self, handle: DownloadHandle) {
        self.downloads = Some(handle);
    }

    /// Record a successful informational message.
    pub fn set_status(&mut self, message: impl Into<String>) {
        self.status = message.into();
        self.status_error = false;
        self.status_at = Instant::now();
    }

    /// Record an error message.
    pub fn set_error(&mut self, message: impl Into<String>) {
        self.status = message.into();
        self.status_error = true;
        self.status_at = Instant::now();
    }

    /// Dismiss the splash screen.
    pub fn leave_splash(&mut self) {
        if self.screen == Screen::Splash {
            self.screen = Screen::Board;
        }
    }

    /// Advance the splash timer and expire old status messages.
    pub fn on_tick(&mut self) {
        if self.screen == Screen::Splash && self.started_at.elapsed() >= SPLASH_DURATION {
            self.leave_splash();
        }

        if !self.status.is_empty()
            && !self.status_error
            && self.status_at.elapsed() >= STATUS_LIFETIME
        {
            self.status.clear();
        }
    }

    // -----------------------------------------------------------------
    // Catalog options
    // -----------------------------------------------------------------

    /// Boards in curated order: Karnataka first, then CBSE/NCERT, then any
    /// additional catalogs alphabetically.
    #[must_use]
    pub fn boards(&self) -> Vec<OptionItem> {
        let mut boards: Vec<_> = self.catalog.boards().iter().collect();
        boards.sort_by(|left, right| {
            board_rank(&left.board.id)
                .cmp(&board_rank(&right.board.id))
                .then_with(|| {
                    left.board
                        .name
                        .to_lowercase()
                        .cmp(&right.board.name.to_lowercase())
                })
        });

        boards
            .into_iter()
            .map(|board| OptionItem {
                value: board.board.id.clone(),
                label: board.board.name.clone(),
                count: board.entries.len(),
            })
            .collect()
    }

    /// Levels available for the selected board.
    #[must_use]
    pub fn levels(&self) -> Vec<OptionItem> {
        let mut counts: HashMap<&str, usize> = HashMap::new();
        for entry in self.board_entries() {
            *counts.entry(entry.level.as_str()).or_insert(0) += 1;
        }

        let mut levels: Vec<(&str, usize)> = counts.into_iter().collect();
        levels.sort_by(|(left, _), (right, _)| {
            level_sort_key(left)
                .cmp(&level_sort_key(right))
                .then_with(|| left.cmp(right))
        });

        levels
            .into_iter()
            .map(|(level, count)| OptionItem {
                value: level.to_owned(),
                label: level_label(level),
                count,
            })
            .collect()
    }

    /// Streams available for the selected board and level.
    ///
    /// The first item is always "All streams" with an empty value. Boards
    /// without streams return an empty list and the stream screen is skipped.
    #[must_use]
    pub fn stream_options(&self) -> Vec<OptionItem> {
        let mut counts: HashMap<&str, usize> = HashMap::new();
        let mut total = 0;

        for entry in self.board_entries() {
            if !self.level_matches(entry) {
                continue;
            }
            total += 1;
            if let Some(stream) = entry.stream.as_deref() {
                *counts.entry(stream).or_insert(0) += 1;
            }
        }

        if counts.is_empty() {
            return Vec::new();
        }

        let mut streams: Vec<(&str, usize)> = counts.into_iter().collect();
        streams.sort_by_key(|(name, _)| name.to_lowercase());

        let mut items = vec![OptionItem {
            value: String::new(),
            label: "All streams".to_owned(),
            count: total,
        }];
        items.extend(streams.into_iter().map(|(name, count)| OptionItem {
            value: name.to_owned(),
            label: name.to_owned(),
            count,
        }));
        items
    }

    /// Subjects available for the selected board, level and stream.
    #[must_use]
    pub fn subjects(&self) -> Vec<OptionItem> {
        let mut counts: HashMap<&str, usize> = HashMap::new();
        for entry in self.board_entries() {
            if self.context_matches(entry) {
                *counts.entry(entry.subject.as_str()).or_insert(0) += 1;
            }
        }

        let mut subjects: Vec<(&str, usize)> = counts.into_iter().collect();
        subjects.sort_by_key(|(name, _)| name.to_lowercase());

        subjects
            .into_iter()
            .map(|(subject, count)| OptionItem {
                value: subject.to_owned(),
                label: subject.to_owned(),
                count,
            })
            .collect()
    }

    /// Material types available for the selected subject.
    #[must_use]
    pub fn materials(&self) -> Vec<OptionItem> {
        let mut counts: HashMap<MaterialType, usize> = HashMap::new();
        for entry in self.board_entries() {
            if self.context_matches(entry) && self.subject_matches(entry) {
                *counts.entry(entry.kind).or_insert(0) += 1;
            }
        }

        MaterialType::ALL
            .iter()
            .filter_map(|kind| {
                counts.get(kind).map(|count| OptionItem {
                    value: kind.as_str().to_owned(),
                    label: kind.display_name().to_owned(),
                    count: *count,
                })
            })
            .collect()
    }

    fn board_entries(&self) -> impl Iterator<Item = &Entry> + '_ {
        let board_id = self.selection.board.as_deref();
        self.catalog
            .entries()
            .filter(move |item| board_id.is_none_or(|id| item.board.id.eq_ignore_ascii_case(id)))
            .map(|item| item.entry)
    }

    fn level_matches(&self, entry: &Entry) -> bool {
        self.selection
            .level
            .as_deref()
            .is_none_or(|level| canonical_level(level) == canonical_level(&entry.level))
    }

    fn stream_matches(&self, entry: &Entry) -> bool {
        self.selection.stream.as_deref().is_none_or(|selected| {
            entry
                .stream
                .as_deref()
                .is_none_or(|stream| stream.eq_ignore_ascii_case(selected))
        })
    }

    fn subject_matches(&self, entry: &Entry) -> bool {
        self.selection
            .subject
            .as_deref()
            .is_none_or(|subject| entry.subject.eq_ignore_ascii_case(subject))
    }

    fn context_matches(&self, entry: &Entry) -> bool {
        self.level_matches(entry) && self.stream_matches(entry)
    }

    // -----------------------------------------------------------------
    // Cursors and navigation
    // -----------------------------------------------------------------

    /// Current menu cursor for the active screen.
    #[must_use]
    pub fn current_cursor(&self) -> usize {
        match self.screen {
            Screen::Board => self.board_cursor,
            Screen::Level => self.level_cursor,
            Screen::Stream => self.stream_cursor,
            Screen::Subject => self.subject_cursor,
            Screen::Material => self.material_cursor,
            Screen::Browse => self.browse_cursor,
            Screen::Queue => self.queue.cursor,
            Screen::Settings => self.settings.cursor,
            Screen::Splash | Screen::Help => 0,
        }
    }

    /// Number of rows on the active screen.
    #[must_use]
    pub fn menu_len(&self) -> usize {
        match self.screen {
            Screen::Board => self.boards().len(),
            Screen::Level => self.levels().len(),
            Screen::Stream => self.stream_options().len(),
            Screen::Subject => self.subjects().len(),
            Screen::Material => self.materials().len(),
            Screen::Browse => self.filtered().len(),
            Screen::Queue => self.queue.jobs.len(),
            Screen::Settings => SettingField::ALL.len(),
            Screen::Splash | Screen::Help => 0,
        }
    }

    /// Move the current cursor by `delta` rows, clamped to the screen.
    pub fn move_selection(&mut self, delta: i64) {
        let len = self.menu_len();
        if len == 0 {
            self.set_cursor(0);
            return;
        }
        let next = shift_cursor(self.current_cursor(), len, delta);
        self.set_cursor(next);
    }

    /// Select the first row on the active screen.
    pub fn move_selection_start(&mut self) {
        self.set_cursor(0);
    }

    /// Select the last row on the active screen.
    pub fn move_selection_end(&mut self) {
        let len = self.menu_len();
        if len > 0 {
            self.set_cursor(len - 1);
        }
    }

    fn set_cursor(&mut self, value: usize) {
        match self.screen {
            Screen::Board => self.board_cursor = value,
            Screen::Level => self.level_cursor = value,
            Screen::Stream => self.stream_cursor = value,
            Screen::Subject => self.subject_cursor = value,
            Screen::Material => self.material_cursor = value,
            Screen::Browse => self.browse_cursor = value,
            Screen::Queue => self.queue.cursor = value,
            Screen::Settings => self.settings.cursor = value,
            Screen::Splash | Screen::Help => {}
        }
    }

    /// Activate the selected row (Enter).
    pub fn confirm_current(&mut self) {
        match self.screen {
            Screen::Board => self.choose_board(),
            Screen::Level => self.choose_level(),
            Screen::Stream => self.choose_stream(),
            Screen::Subject => self.choose_subject(),
            Screen::Material => self.choose_material(),
            Screen::Browse => {
                self.queue_selection();
            }
            Screen::Queue => self.toggle_pause(),
            Screen::Settings => self.settings_activate(),
            Screen::Splash | Screen::Help => {}
        }
    }

    /// Move one screen back (Esc / Backspace).
    pub fn go_back(&mut self) {
        match self.screen {
            Screen::Board => {}
            Screen::Level => {
                self.selection.level = None;
                self.selection.stream = None;
                self.selection.subject = None;
                self.selection.kind = None;
                self.screen = Screen::Board;
                self.sync_board_cursor();
            }
            Screen::Stream => {
                self.selection.stream = None;
                self.selection.subject = None;
                self.selection.kind = None;
                self.screen = Screen::Level;
                self.sync_level_cursor();
            }
            Screen::Subject => {
                self.selection.subject = None;
                self.selection.kind = None;
                if self.stream_options().is_empty() {
                    self.screen = Screen::Level;
                    self.sync_level_cursor();
                } else {
                    self.screen = Screen::Stream;
                    self.sync_stream_cursor();
                }
            }
            Screen::Material => {
                self.selection.kind = None;
                self.screen = Screen::Subject;
                self.sync_subject_cursor();
            }
            Screen::Browse => {
                self.search_active = false;
                self.year_picker_open = false;
                self.screen = Screen::Material;
                self.sync_material_cursor();
            }
            Screen::Queue => {
                self.screen = self.queue_return;
            }
            Screen::Settings => {
                if self.settings.editing {
                    self.cancel_setting_editor();
                } else {
                    self.screen = self.settings_return;
                }
            }
            Screen::Help => self.close_help(),
            Screen::Splash => self.leave_splash(),
        }
    }

    fn choose_board(&mut self) {
        let index = self.board_cursor;
        let Some(item) = self.boards().get(index).cloned() else {
            return;
        };

        self.selection = Selection {
            board: Some(item.value),
            ..Selection::default()
        };
        self.board_cursor = index;
        self.level_cursor = 0;
        self.stream_cursor = 0;
        self.subject_cursor = 0;
        self.material_cursor = 0;
        self.screen = Screen::Level;
    }

    fn choose_level(&mut self) {
        let Some(item) = self.levels().get(self.level_cursor).cloned() else {
            return;
        };

        self.selection.level = Some(item.value);
        self.selection.stream = None;
        self.selection.subject = None;
        self.selection.kind = None;
        self.stream_cursor = 0;
        self.subject_cursor = 0;
        self.material_cursor = 0;
        self.screen = if self.stream_options().is_empty() {
            Screen::Subject
        } else {
            Screen::Stream
        };
    }

    fn choose_stream(&mut self) {
        let Some(item) = self.stream_options().get(self.stream_cursor).cloned() else {
            return;
        };

        self.selection.stream = if item.value.is_empty() {
            None
        } else {
            Some(item.value)
        };
        self.selection.subject = None;
        self.selection.kind = None;
        self.subject_cursor = 0;
        self.material_cursor = 0;
        self.screen = Screen::Subject;
    }

    fn choose_subject(&mut self) {
        let Some(item) = self.subjects().get(self.subject_cursor).cloned() else {
            return;
        };

        self.selection.subject = Some(item.value);
        self.selection.kind = None;
        self.material_cursor = 0;
        self.screen = Screen::Material;
    }

    fn choose_material(&mut self) {
        let Some(item) = self.materials().get(self.material_cursor).cloned() else {
            return;
        };

        let Ok(kind) = item.value.parse::<MaterialType>() else {
            return;
        };
        self.selection.kind = Some(kind);
        self.enter_browse();
    }

    fn enter_browse(&mut self) {
        self.selected.clear();
        self.search.clear();
        self.search_active = false;
        self.search_before.clear();
        self.year_filter = None;
        self.year_picker_open = false;
        self.browse_cursor = 0;
        self.screen = Screen::Browse;
    }

    fn sync_board_cursor(&mut self) {
        let boards = self.boards();
        self.board_cursor = self
            .selection
            .board
            .as_deref()
            .and_then(|id| boards.iter().position(|board| board.value == id))
            .unwrap_or(0);
    }

    fn sync_level_cursor(&mut self) {
        let levels = self.levels();
        self.level_cursor = self
            .selection
            .level
            .as_deref()
            .and_then(|level| levels.iter().position(|item| item.value == level))
            .unwrap_or(0);
    }

    fn sync_stream_cursor(&mut self) {
        let streams = self.stream_options();
        self.stream_cursor = self
            .selection
            .stream
            .as_deref()
            .and_then(|stream| streams.iter().position(|item| item.value == stream))
            .unwrap_or(0);
    }

    fn sync_subject_cursor(&mut self) {
        let subjects = self.subjects();
        self.subject_cursor = self
            .selection
            .subject
            .as_deref()
            .and_then(|subject| subjects.iter().position(|item| item.value == subject))
            .unwrap_or(0);
    }

    fn sync_material_cursor(&mut self) {
        let materials = self.materials();
        self.material_cursor = self
            .selection
            .kind
            .and_then(|kind| {
                materials
                    .iter()
                    .position(|item| item.value == kind.as_str())
            })
            .unwrap_or(0);
    }

    // -----------------------------------------------------------------
    // Browse filtering
    // -----------------------------------------------------------------

    /// Entries for the PDF screen, applying the year and latest-only filters
    /// plus the fuzzy search box. Results keep catalog order except that the
    /// preferred language is moved to the front.
    #[must_use]
    pub fn filtered(&self) -> Vec<CatalogEntry<'_>> {
        self.entries_with(self.year_filter, self.latest_only)
    }

    /// Years available in the current selection, newest first. Latest-only is
    /// deliberately ignored so older editions remain selectable.
    #[must_use]
    pub fn available_years(&self) -> Vec<u16> {
        let mut years = BTreeSet::new();
        for item in self.entries_with(None, false) {
            if let Some(year) = item.entry.year {
                years.insert(year);
            }
        }
        years.into_iter().rev().collect()
    }

    fn entries_with(&self, year: Option<u16>, latest_only: bool) -> Vec<CatalogEntry<'_>> {
        let filter = Filter {
            board: self.selection.board.clone(),
            level: self.selection.level.clone(),
            stream: self.selection.stream.clone(),
            subject: self.selection.subject.clone(),
            kind: self.selection.kind,
            year,
            language: None,
            search: None,
            latest_only,
        };

        let mut entries = filter.apply(&self.catalog);
        let query = self.search.trim();
        if !query.is_empty() {
            entries.retain(|item| entry_matches_search(*item, query));
        }

        let preferred = self.config.language;
        entries.sort_by_key(|item| item.entry.language != preferred);
        entries
    }

    /// Move the PDF cursor.
    pub fn browse_move(&mut self, delta: i64) {
        let len = self.filtered().len();
        self.browse_cursor = shift_cursor(self.browse_cursor, len, delta);
    }

    /// Toggle the selected PDF with Space.
    pub fn toggle_selected(&mut self) {
        let Some(url) = self
            .filtered()
            .get(self.browse_cursor)
            .map(|item| item.entry.url.clone())
        else {
            return;
        };

        if !self.selected.remove(&url) {
            self.selected.insert(url);
        }
    }

    /// Select every visible PDF, or clear the selection when all are selected.
    pub fn toggle_select_all(&mut self) {
        let urls: Vec<String> = self
            .filtered()
            .iter()
            .map(|item| item.entry.url.clone())
            .collect();

        if urls.is_empty() {
            return;
        }

        if urls.iter().all(|url| self.selected.contains(url)) {
            for url in urls {
                self.selected.remove(&url);
            }
            self.set_status("Selection cleared.");
        } else {
            for url in urls {
                self.selected.insert(url);
            }
            self.set_status("All visible PDFs selected.");
        }
    }

    /// Start the fuzzy search box.
    pub fn open_search(&mut self) {
        if self.screen != Screen::Browse || self.search_active {
            return;
        }
        self.search_before = self.search.clone();
        self.search_active = true;
    }

    /// Accept the current search and return to list navigation.
    pub fn commit_search(&mut self) {
        self.search_active = false;
    }

    /// Leave the search box and restore the previous query.
    pub fn cancel_search(&mut self) {
        if !self.search_active {
            return;
        }
        self.search = self.search_before.clone();
        self.search_active = false;
        self.on_filter_changed();
    }

    /// Append a character to the query.
    pub fn push_search_char(&mut self, character: char) {
        self.search.push(character);
        self.on_filter_changed();
    }

    /// Delete the last query character.
    pub fn pop_search_char(&mut self) {
        self.search.pop();
        self.on_filter_changed();
    }

    /// Clear the query.
    pub fn clear_search(&mut self) {
        self.search.clear();
        self.on_filter_changed();
    }

    fn on_filter_changed(&mut self) {
        self.selected.clear();
        self.browse_cursor = 0;
    }

    /// Toggle the latest-only filter.
    pub fn toggle_latest(&mut self) {
        self.latest_only = !self.latest_only;
        self.on_filter_changed();
        self.set_status(if self.latest_only {
            "Latest only: on"
        } else {
            "Latest only: off"
        });
    }

    /// Open the year picker.
    pub fn open_year_picker(&mut self) {
        if self.screen != Screen::Browse {
            return;
        }

        let years = self.available_years();
        self.year_cursor = self.year_filter.map_or(0, |year| {
            years
                .iter()
                .position(|candidate| *candidate == year)
                .map_or(0, |index| index + 1)
        });
        self.year_picker_open = true;
    }

    /// Move inside the year picker.
    pub fn year_move(&mut self, delta: i64) {
        let len = self.available_years().len() + 1;
        self.year_cursor = shift_cursor(self.year_cursor, len, delta);
    }

    /// Apply the highlighted year and close the picker.
    pub fn confirm_year(&mut self) {
        let years = self.available_years();
        self.year_filter = if self.year_cursor == 0 {
            None
        } else {
            years.get(self.year_cursor - 1).copied()
        };
        self.year_picker_open = false;
        self.on_filter_changed();
    }

    /// Close the year picker without changing the filter.
    pub fn close_year_picker(&mut self) {
        self.year_picker_open = false;
    }

    // -----------------------------------------------------------------
    // Downloads
    // -----------------------------------------------------------------

    /// Queue the toggled PDFs, or the row under the cursor when nothing is
    /// selected. Switches to the queue screen when at least one job is queued.
    pub fn queue_selection(&mut self) {
        let download_dir = self.config.resolved_download_dir();
        let specs = {
            let filtered = self.filtered();
            let chosen: Vec<CatalogEntry<'_>> = if self.selected.is_empty() {
                filtered
                    .get(self.browse_cursor)
                    .copied()
                    .into_iter()
                    .collect()
            } else {
                filtered
                    .iter()
                    .copied()
                    .filter(|item| self.selected.contains(&item.entry.url))
                    .collect()
            };

            chosen.into_iter().map(|item| {
                DownloadSpec::from_entry(
                    item,
                    &download_dir,
                    &self.config.folder_template,
                    ConflictPolicy::Ask,
                )
            })
        };

        let specs: Vec<DownloadSpec> = match specs.collect() {
            Ok(specs) => specs,
            Err(error) => {
                self.set_error(format!("Could not build the destination path: {error}"));
                return;
            }
        };

        if specs.is_empty() {
            self.set_error("No PDF selected. Move to a row or press Space to select one.");
            return;
        }

        let Some(handle) = self.downloads.clone() else {
            self.set_error("The download manager is not running.");
            return;
        };

        let first_new = self.queue.jobs.len();
        let mut queued = 0;
        let mut failure = None;

        for spec in specs {
            let destination = spec.destination.clone();
            let duplicate = self.queue.jobs.iter().any(|job| {
                job.status.is_active() && job.destination.as_deref() == Some(destination.as_path())
            });
            if duplicate {
                continue;
            }

            let title = spec.title.clone();
            match handle.enqueue(spec) {
                Ok(id) => {
                    self.queue
                        .jobs
                        .push(JobView::new(id, title, Some(destination)));
                    queued += 1;
                }
                Err(error) => {
                    failure = Some(error.to_string());
                    break;
                }
            }
        }

        let had_failure = failure.is_some();
        if let Some(error) = failure {
            self.set_error(format!("Could not queue a download: {error}"));
        }

        if queued == 0 {
            if !had_failure {
                self.set_status("Those files are already in the queue.");
            }
            return;
        }

        self.queue.cursor = first_new;
        self.queue_return = Screen::Browse;
        self.screen = Screen::Queue;
        self.set_status(format!("Queued {queued} download(s)."));
    }

    /// Open the queue without starting a new download.
    pub fn open_queue(&mut self) {
        if self.screen != Screen::Queue {
            self.queue_return = self.screen;
            self.screen = Screen::Queue;
        }
    }

    /// Move the queue cursor.
    pub fn queue_move(&mut self, delta: i64) {
        self.queue.cursor = shift_cursor(self.queue.cursor, self.queue.jobs.len(), delta);
    }

    /// Pause or resume the selected job.
    pub fn toggle_pause(&mut self) {
        let Some(job) = self.queue.jobs.get(self.queue.cursor) else {
            return;
        };
        let id = job.id;
        let status = job.status;

        let Some(handle) = self.downloads.clone() else {
            self.set_error("The download manager is not running.");
            return;
        };

        let result = match status {
            JobStatus::Paused => {
                self.set_status("Resuming download...");
                handle.resume(id)
            }
            JobStatus::Queued | JobStatus::Running | JobStatus::Retrying => {
                self.set_status("Pausing download...");
                handle.pause(id)
            }
            JobStatus::Conflict => {
                self.set_error("Resolve the file conflict first.");
                return;
            }
            JobStatus::Completed
            | JobStatus::Failed
            | JobStatus::Skipped
            | JobStatus::Cancelled => {
                self.set_status("That download has already finished.");
                return;
            }
        };

        if let Err(error) = result {
            self.set_error(format!("Could not control the download: {error}"));
        }
    }

    /// Retry a failed download.
    pub fn queue_retry(&mut self) {
        let Some(job) = self.queue.jobs.get(self.queue.cursor) else {
            return;
        };
        let id = job.id;

        if job.status != JobStatus::Failed {
            self.set_status("Only failed downloads can be retried.");
            return;
        }

        let Some(handle) = self.downloads.clone() else {
            self.set_error("The download manager is not running.");
            return;
        };

        match handle.retry(id) {
            Ok(()) => self.set_status("Retrying download..."),
            Err(error) => self.set_error(format!("Could not retry the download: {error}")),
        }
    }

    /// Cancel the selected job.
    pub fn queue_cancel(&mut self) {
        let Some(job) = self.queue.jobs.get(self.queue.cursor) else {
            return;
        };
        let id = job.id;

        if job.status.is_finished() {
            self.set_status("That download has already finished.");
            return;
        }

        let Some(handle) = self.downloads.clone() else {
            self.set_error("The download manager is not running.");
            return;
        };

        match handle.cancel(id) {
            Ok(()) => self.set_status("Cancelling download..."),
            Err(error) => self.set_error(format!("Could not cancel the download: {error}")),
        }
    }

    /// Answer the oldest pending conflict.
    pub fn resolve_conflict(&mut self, resolution: ConflictResolution) {
        let Some(handle) = self.downloads.clone() else {
            self.set_error("The download manager is not running.");
            return;
        };

        let Some(prompt) = self.queue.conflicts.pop_front() else {
            return;
        };

        match handle.resolve_conflict(prompt.id, resolution) {
            Ok(()) => self.set_status(match resolution {
                ConflictResolution::Skip => "Keeping the existing file.",
                ConflictResolution::Rename => "Downloading beside the existing file.",
                ConflictResolution::Overwrite => "Replacing the existing file.",
                ConflictResolution::Cancel => "Download cancelled.",
            }),
            Err(error) => {
                self.queue.conflicts.push_front(prompt);
                self.set_error(format!("Could not resolve the conflict: {error}"));
            }
        }
    }

    /// Apply an event from the download manager.
    pub fn handle_download_event(&mut self, event: DownloadEvent) {
        match event {
            DownloadEvent::Queued { id, title } => {
                if let Some(job) = self.job_mut(id) {
                    job.title = title;
                    job.status = JobStatus::Queued;
                } else {
                    self.queue.jobs.push(JobView::new(id, title, None));
                }
            }
            DownloadEvent::Started {
                id,
                destination,
                resumed_from,
                total,
            } => {
                if let Some(job) = self.job_mut(id) {
                    job.status = JobStatus::Running;
                    job.destination = Some(destination);
                    job.progress.total = total.or(job.progress.total);
                    job.progress.downloaded = job.progress.downloaded.max(resumed_from);
                    job.note = None;
                }
            }
            DownloadEvent::Progress { id, progress } => {
                if let Some(job) = self.job_mut(id) {
                    job.status = JobStatus::Running;
                    job.progress = progress;
                }
            }
            DownloadEvent::Retrying {
                id,
                attempt,
                max_attempts,
                delay,
                error,
            } => {
                if let Some(job) = self.job_mut(id) {
                    job.status = JobStatus::Retrying;
                    job.note = Some(format!(
                        "{}  ·  retry {attempt}/{max_attempts} in {}s",
                        short_error(&error),
                        delay.as_secs().max(1)
                    ));
                }
            }
            DownloadEvent::Paused { id, downloaded } => {
                if let Some(job) = self.job_mut(id) {
                    job.status = JobStatus::Paused;
                    job.progress.downloaded = downloaded;
                    job.progress.bytes_per_second = 0.0;
                    job.progress.eta = None;
                    job.note = None;
                }
            }
            DownloadEvent::Conflict {
                id,
                path,
                existing_size,
                expected_size,
            } => {
                if let Some(job) = self.job_mut(id) {
                    job.status = JobStatus::Conflict;
                    job.destination = Some(path.clone());
                    job.note = None;
                }
                self.queue.conflicts.push_back(ConflictPrompt {
                    id,
                    path,
                    existing_size,
                    expected_size,
                });

                if matches!(self.screen, Screen::Browse | Screen::Queue) {
                    self.screen = Screen::Queue;
                }
                self.set_error("A download needs a decision: the destination already exists.");
            }
            DownloadEvent::Skipped { id, path, reason } => {
                if let Some(job) = self.job_mut(id) {
                    job.status = JobStatus::Skipped;
                    job.destination = Some(path);
                    job.note = Some(reason.message().to_owned());
                }
            }
            DownloadEvent::Completed { id, path, bytes } => {
                let mut title = None;
                if let Some(job) = self.job_mut(id) {
                    job.status = JobStatus::Completed;
                    job.destination = Some(path);
                    job.progress.downloaded = bytes;
                    job.progress.total = Some(bytes);
                    job.progress.bytes_per_second = 0.0;
                    job.progress.eta = Some(Duration::ZERO);
                    job.note = None;
                    title = Some(job.title.clone());
                }

                if let Some(title) = title {
                    self.set_status(format!("Downloaded {title}"));
                }
            }
            DownloadEvent::Failed { id, error } => {
                let message = friendly_download_error(&error);
                if let Some(job) = self.job_mut(id) {
                    job.status = JobStatus::Failed;
                    job.note = Some(message.clone());
                }
                self.set_error(format!("Download failed: {message}"));
            }
            DownloadEvent::Cancelled { id } => {
                if let Some(job) = self.job_mut(id) {
                    job.status = JobStatus::Cancelled;
                    job.note = None;
                }
            }
        }
    }

    fn job_mut(&mut self, id: JobId) -> Option<&mut JobView> {
        self.queue.jobs.iter_mut().find(|job| job.id == id)
    }

    /// Number of jobs that still need attention or are making progress.
    #[must_use]
    pub fn active_downloads(&self) -> usize {
        self.queue
            .jobs
            .iter()
            .filter(|job| job.status.is_active())
            .count()
    }

    /// Number of jobs that reached a terminal state.
    #[must_use]
    pub fn finished_downloads(&self) -> usize {
        self.queue
            .jobs
            .iter()
            .filter(|job| job.status.is_finished())
            .count()
    }

    // -----------------------------------------------------------------
    // Help, settings and queue access
    // -----------------------------------------------------------------

    /// Open the help overlay.
    pub fn open_help(&mut self) {
        if self.screen != Screen::Help {
            self.help_return = self.screen;
            self.screen = Screen::Help;
        }
    }

    /// Close the help overlay and return to the previous screen.
    pub fn close_help(&mut self) {
        self.screen = self.help_return;
    }

    /// Open the settings screen.
    pub fn open_settings(&mut self) {
        if matches!(
            self.screen,
            Screen::Settings | Screen::Help | Screen::Splash
        ) {
            return;
        }
        self.settings_return = self.screen;
        self.settings.editing = false;
        self.settings.buffer.clear();
        self.settings.cursor = 0;
        self.screen = Screen::Settings;
    }

    /// Leave settings, cancelling an active text edit first.
    pub fn close_settings(&mut self) {
        if self.settings.editing {
            self.cancel_setting_editor();
        } else {
            self.screen = self.settings_return;
        }
    }

    /// The selected settings field.
    #[must_use]
    pub fn settings_field(&self) -> SettingField {
        let index = self.settings.cursor.min(SettingField::ALL.len() - 1);
        SettingField::ALL[index]
    }

    /// Move the settings cursor.
    pub fn settings_move(&mut self, delta: i64) {
        self.settings.cursor = shift_cursor(self.settings.cursor, SettingField::ALL.len(), delta);
    }

    /// Enter edit mode or cycle/toggle the selected setting.
    pub fn settings_activate(&mut self) {
        let field = self.settings_field();
        match field.kind() {
            SettingKind::Text => {
                self.settings.buffer = field.value(&self.config);
                self.settings.editing = true;
            }
            SettingKind::Cycle => {
                let next = language_step(self.config.language, 1);
                self.apply_setting(field, next.code());
            }
            SettingKind::Number => {
                let next = if self.config.concurrency >= MAX_CONCURRENCY {
                    MIN_CONCURRENCY
                } else {
                    self.config.concurrency + 1
                };
                self.apply_setting(field, &next.to_string());
            }
            SettingKind::Toggle => {
                let next = !self.config.latest_only;
                self.apply_setting(field, if next { "true" } else { "false" });
            }
        }
    }

    /// Adjust the selected setting with Left/Right.
    pub fn settings_adjust(&mut self, delta: i64) {
        let field = self.settings_field();
        match field.kind() {
            SettingKind::Cycle => {
                let next = language_step(self.config.language, delta);
                self.apply_setting(field, next.code());
            }
            SettingKind::Number => {
                let next = if delta < 0 {
                    self.config
                        .concurrency
                        .saturating_sub(1)
                        .max(MIN_CONCURRENCY)
                } else {
                    (self.config.concurrency + 1).min(MAX_CONCURRENCY)
                };
                self.apply_setting(field, &next.to_string());
            }
            SettingKind::Toggle => {
                let next = !self.config.latest_only;
                self.apply_setting(field, if next { "true" } else { "false" });
            }
            SettingKind::Text => {}
        }
    }

    /// Append a character while editing a text setting.
    pub fn push_editor_char(&mut self, character: char) {
        if self.settings.editing {
            self.settings.buffer.push(character);
        }
    }

    /// Delete the last character while editing a text setting.
    pub fn pop_editor_char(&mut self) {
        if self.settings.editing {
            self.settings.buffer.pop();
        }
    }

    /// Clear the text editor buffer.
    pub fn clear_editor(&mut self) {
        if self.settings.editing {
            self.settings.buffer.clear();
        }
    }

    /// Save the text editor buffer.
    pub fn commit_setting_editor(&mut self) {
        if !self.settings.editing {
            return;
        }

        let field = self.settings_field();
        let value = self.settings.buffer.clone();
        if self.apply_setting(field, &value) {
            self.settings.editing = false;
            self.settings.buffer.clear();
        }
    }

    /// Discard the text editor buffer.
    pub fn cancel_setting_editor(&mut self) {
        self.settings.editing = false;
        self.settings.buffer.clear();
    }

    fn apply_setting(&mut self, field: SettingField, value: &str) -> bool {
        let mut candidate = self.config.clone();
        let result = candidate
            .set(field.key(), value)
            .and_then(|()| candidate.save(&self.paths));

        match result {
            Ok(()) => {
                self.config = candidate;
                if field == SettingField::LatestOnly {
                    self.latest_only = self.config.latest_only;
                    self.selected.clear();
                    self.browse_cursor = 0;
                }
                self.set_status(format!("{} saved.", field.label()));
                true
            }
            Err(error) => {
                self.set_error(format!("{}: {error}", field.label()));
                false
            }
        }
    }

    // -----------------------------------------------------------------
    // Header and status helpers
    // -----------------------------------------------------------------

    /// Breadcrumb components for the current selection.
    #[must_use]
    pub fn breadcrumb(&self) -> Vec<String> {
        let mut parts = Vec::new();

        if let Some(board_id) = self.selection.board.as_deref() {
            if let Some(board) = self.catalog.find_board(board_id) {
                parts.push(board.board.name.clone());
            } else {
                parts.push(board_id.to_owned());
            }
        }

        if let Some(level) = self.selection.level.as_deref() {
            parts.push(breadcrumb_level(level));
        }
        if let Some(stream) = self.selection.stream.as_deref() {
            parts.push(stream.to_owned());
        }
        if let Some(subject) = self.selection.subject.as_deref() {
            parts.push(subject.to_owned());
        }
        if let Some(kind) = self.selection.kind {
            parts.push(kind.display_name().to_owned());
        }

        parts
    }

    /// Default status-bar text for the current screen.
    #[must_use]
    pub fn status_summary(&self) -> String {
        match self.screen {
            Screen::Splash => String::new(),
            Screen::Board => {
                if self.catalog.is_empty() {
                    "No catalogs loaded.".to_owned()
                } else {
                    format!(
                        "{} · {} PDFs",
                        count_label(self.boards().len(), "board", "boards"),
                        format_count(self.catalog.len())
                    )
                }
            }
            Screen::Level => count_label(self.menu_len(), "level", "levels"),
            Screen::Stream => count_label(self.menu_len(), "stream", "streams"),
            Screen::Subject => count_label(self.menu_len(), "subject", "subjects"),
            Screen::Material => count_label(self.menu_len(), "material type", "material types"),
            Screen::Browse => {
                let count = count_label(self.filtered().len(), "PDF", "PDFs");
                if self.selected.is_empty() {
                    count
                } else {
                    format!("{count} · {} selected", self.selected.len())
                }
            }
            Screen::Queue => format!(
                "{} · {} finished",
                count_label(self.active_downloads(), "active", "active"),
                self.finished_downloads()
            ),
            Screen::Settings => "Changes save automatically.".to_owned(),
            Screen::Help => format!("pu-get v{}", env!("CARGO_PKG_VERSION")),
        }
    }
}

fn count_label(count: usize, singular: &str, plural: &str) -> String {
    format!("{count} {}", if count == 1 { singular } else { plural })
}

fn format_count(value: usize) -> String {
    let digits = value.to_string();
    let mut formatted = String::with_capacity(digits.len() + digits.len() / 3);

    for (index, character) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            formatted.push(',');
        }
        formatted.push(character);
    }

    formatted
}

fn friendly_download_error(error: &DownloadError) -> String {
    match error {
        DownloadError::Timeout { seconds } => format!("Timed out after {seconds}s"),
        DownloadError::Http { status, .. } => format!("HTTP {status}"),
        DownloadError::Network { .. } => "Network error".to_owned(),
        DownloadError::PermissionDenied { .. } => "Permission denied".to_owned(),
        DownloadError::DiskFull { .. } => "Disk full".to_owned(),
        DownloadError::InvalidContentType { .. } | DownloadError::NotPdf { .. } => {
            "Not a PDF".to_owned()
        }
        DownloadError::AlreadyExists { .. } => "File already exists".to_owned(),
        DownloadError::Io { .. } => "File error".to_owned(),
        DownloadError::InvalidUrl { .. } => "Invalid URL".to_owned(),
        DownloadError::ManagerStopped => "Manager stopped".to_owned(),
        DownloadError::InvalidConfiguration { .. } => "Configuration error".to_owned(),
        DownloadError::Client { .. } => "HTTP client error".to_owned(),
    }
}

fn short_error(message: &str) -> String {
    let lower = message.to_ascii_lowercase();

    for (needle, label) in [
        ("timed out", "Timed out"),
        ("permission denied", "Permission denied"),
        ("disk space", "Disk full"),
        ("not a pdf", "Not a PDF"),
        ("instead of a pdf", "Not a PDF"),
        ("connection", "Network error"),
        ("dns", "Network error"),
    ] {
        if lower.contains(needle) {
            return label.to_owned();
        }
    }

    if let Some(index) = lower.find("http ") {
        let rest = &message[index + 5..];
        let status: String = rest.chars().take_while(char::is_ascii_digit).collect();
        if !status.is_empty() {
            return format!("HTTP {status}");
        }
    }

    let text = message.split([':', ';']).next().unwrap_or(message).trim();
    text.chars().take(56).collect()
}

fn board_rank(id: &str) -> u8 {
    match id {
        "kseeb" => 0,
        "pue" => 1,
        "cbse" => 2,
        "ncert" => 3,
        _ => 4,
    }
}

fn level_sort_key(level: &str) -> (u8, u16, String) {
    let canonical = canonical_level(level);
    match canonical.as_str() {
        "sslc" => (0, 0, String::new()),
        "puc1" => (1, 0, String::new()),
        "puc2" => (2, 0, String::new()),
        _ => match canonical.parse::<u16>() {
            Ok(number) => (3, number, String::new()),
            Err(_) => (4, 0, canonical),
        },
    }
}

fn breadcrumb_level(level: &str) -> String {
    let canonical = canonical_level(level);
    match canonical.parse::<u16>() {
        Ok(number) => format!("Class {number}"),
        Err(_) => level.trim().to_owned(),
    }
}

fn level_label(level: &str) -> String {
    let canonical = canonical_level(level);
    match canonical.as_str() {
        "sslc" => "SSLC (Class 10)".to_owned(),
        "puc1" => "PUC 1 (Class 11)".to_owned(),
        "puc2" => "PUC 2 (Class 12)".to_owned(),
        _ => match canonical.parse::<u16>() {
            Ok(number) => format!("Class {number}"),
            Err(_) => level.trim().to_owned(),
        },
    }
}

fn language_step(current: Language, delta: i64) -> Language {
    let languages = Language::ALL;
    let index = languages
        .iter()
        .position(|language| *language == current)
        .unwrap_or(0);
    let len = languages.len();

    let next = if delta < 0 {
        (index + len - 1) % len
    } else {
        (index + 1) % len
    };
    languages[next]
}

fn shift_cursor(cursor: usize, len: usize, delta: i64) -> usize {
    if len == 0 {
        return 0;
    }

    let last = len - 1;
    let current = cursor.min(last);
    let magnitude = usize::try_from(delta.unsigned_abs()).unwrap_or(usize::MAX);

    if delta < 0 {
        current.saturating_sub(magnitude)
    } else {
        current.saturating_add(magnitude).min(last)
    }
}

fn entry_matches_search(item: CatalogEntry<'_>, query: &str) -> bool {
    let haystack = format!(
        "{} {} {} {} {} {}",
        item.entry.title,
        item.entry.subject,
        item.entry.source,
        item.entry.level,
        item.entry.stream.as_deref().unwrap_or(""),
        item.board.name
    );

    query
        .split_whitespace()
        .all(|term| fuzzy_score(&haystack, term).is_some())
}

/// Case-insensitive subsequence score. Each query term must appear in order;
/// consecutive and word-boundary matches score better.
fn fuzzy_score(haystack: &str, needle: &str) -> Option<u32> {
    let haystack: Vec<char> = haystack.to_lowercase().chars().collect();
    let needle = needle.to_lowercase();
    if needle.is_empty() {
        return Some(0);
    }

    let mut cursor = 0;
    let mut score = 0u32;
    let mut previous: Option<usize> = None;

    for wanted in needle.chars() {
        let mut found = None;
        for (index, candidate) in haystack.iter().enumerate().skip(cursor) {
            if *candidate == wanted {
                found = Some(index);
                break;
            }
        }

        let index = found?;
        score = score.saturating_add(10);

        if previous.is_some_and(|last| index == last + 1) {
            score = score.saturating_add(8);
        }

        let boundary = index == 0
            || haystack.get(index - 1).is_some_and(|character| {
                character.is_whitespace() || matches!(*character, '-' | '_' | '/' | '(')
            });
        if boundary {
            score = score.saturating_add(12);
        }

        previous = Some(index);
        cursor = index + 1;
    }

    Some(score)
}

#[cfg(test)]
mod tests {
    use super::*;

    const KSEEB: &str = r#"
[board]
id = "kseeb"
name = "Karnataka State Board"

[[entries]]
level = "SSLC"
subject = "Mathematics"
type = "previous_year_paper"
title = "SSLC Mathematics Previous Year Paper 2025"
language = "kn"
year = 2025
url = "https://example.com/kseeb/maths-2025.pdf"
source = "KSEAB"

[[entries]]
level = "SSLC"
subject = "Mathematics"
type = "previous_year_paper"
title = "SSLC Mathematics Previous Year Paper 2024"
language = "kn"
year = 2024
url = "https://example.com/kseeb/maths-2024.pdf"
source = "KSEAB"

[[entries]]
level = "SSLC"
subject = "Mathematics"
type = "model_paper"
title = "SSLC Mathematics Model Paper 2025"
language = "en"
year = 2025
url = "https://example.com/kseeb/maths-model-2025.pdf"
source = "KSEAB"
"#;

    const PUE: &str = r#"
[board]
id = "pue"
name = "Karnataka Pre-University"

[[entries]]
level = "PUC 1"
stream = "Science"
subject = "Physics"
type = "textbook"
title = "PUC 1 Physics Textbook"
language = "en"
year = 2025
url = "https://example.com/pue/puc1-physics.pdf"
source = "PUE Karnataka"

[[entries]]
level = "PUC 1"
stream = "Commerce"
subject = "Accountancy"
type = "textbook"
title = "PUC 1 Accountancy Textbook"
language = "en"
year = 2025
url = "https://example.com/pue/puc1-accountancy.pdf"
source = "PUE Karnataka"

[[entries]]
level = "PUC 1"
subject = "Kannada"
type = "textbook"
title = "PUC 1 Kannada Textbook"
language = "kn"
year = 2025
url = "https://example.com/pue/puc1-kannada.pdf"
source = "PUE Karnataka"

[[entries]]
level = "PUC 2"
stream = "Science"
subject = "Chemistry"
type = "textbook"
title = "PUC 2 Chemistry Textbook"
language = "en"
year = 2024
url = "https://example.com/pue/puc2-chemistry.pdf"
source = "PUE Karnataka"
"#;

    fn catalog() -> Catalog {
        let mut catalog = Catalog::from_toml("kseeb-test", KSEEB).unwrap();
        catalog.merge(Catalog::from_toml("pue-test", PUE).unwrap());
        catalog
    }

    fn app() -> App {
        let paths = AppPaths::new(
            std::env::temp_dir().join("pu-get-test/config.toml"),
            std::env::temp_dir().join("pu-get-test/cache"),
        );
        let mut app = App::new(catalog(), Config::default(), paths, false);
        app.leave_splash();
        app
    }

    fn browse_app() -> App {
        let mut app = app();
        app.selection.board = Some("kseeb".to_owned());
        app.selection.level = Some("SSLC".to_owned());
        app.selection.subject = Some("Mathematics".to_owned());
        app.selection.kind = Some(MaterialType::PreviousYearPaper);
        app.screen = Screen::Browse;
        app
    }

    #[test]
    fn boards_are_curated_karnataka_first() {
        let app = app();
        let boards = app.boards();
        let ids: Vec<&str> = boards.iter().map(|board| board.value.as_str()).collect();
        assert_eq!(ids, ["kseeb", "pue"]);
    }

    #[test]
    fn levels_and_streams_are_derived_from_the_catalog() {
        let mut app = app();
        app.selection.board = Some("pue".to_owned());
        let levels = app.levels();
        let level_values: Vec<&str> = levels.iter().map(|item| item.value.as_str()).collect();
        assert_eq!(level_values, ["PUC 1", "PUC 2"]);

        app.selection.level = Some("PUC 1".to_owned());
        let streams = app.stream_options();
        assert_eq!(streams[0].label, "All streams");
        assert_eq!(streams[0].count, 3);
        assert!(streams.iter().any(|stream| stream.value == "Science"));
        assert!(streams.iter().any(|stream| stream.value == "Commerce"));
    }

    #[test]
    fn stream_less_subjects_belong_to_every_stream() {
        let mut app = app();
        app.selection.board = Some("pue".to_owned());
        app.selection.level = Some("PUC 1".to_owned());
        app.selection.stream = Some("Science".to_owned());
        let subjects = app.subjects();
        let subject_values: Vec<&str> = subjects.iter().map(|item| item.value.as_str()).collect();
        assert_eq!(subject_values, ["Kannada", "Physics"]);
    }

    #[test]
    fn choosing_a_board_resets_lower_selections() {
        let mut app = app();
        app.selection.level = Some("SSLC".to_owned());
        app.selection.subject = Some("Mathematics".to_owned());

        app.screen = Screen::Board;
        app.board_cursor = 0;
        app.choose_board();

        assert_eq!(app.selection.board.as_deref(), Some("kseeb"));
        assert!(app.selection.level.is_none());
        assert!(app.selection.subject.is_none());
        assert_eq!(app.screen, Screen::Level);
    }

    #[test]
    fn streams_are_skipped_for_boards_without_them() {
        let mut app = app();
        app.selection.board = Some("kseeb".to_owned());
        app.screen = Screen::Level;
        app.level_cursor = 0;
        app.choose_level();
        assert_eq!(app.screen, Screen::Subject);
        assert!(app.stream_options().is_empty());
    }

    #[test]
    fn latest_only_is_on_by_default_and_keeps_the_newest_paper() {
        let app = browse_app();
        assert!(app.latest_only);

        let titles: Vec<&str> = app
            .filtered()
            .iter()
            .map(|item| item.entry.title.as_str())
            .collect();
        assert_eq!(titles, ["SSLC Mathematics Previous Year Paper 2025"]);

        let years = app.available_years();
        assert_eq!(years, [2025, 2024]);
    }

    #[test]
    fn turning_latest_off_reveals_older_editions() {
        let mut app = browse_app();
        app.toggle_latest();
        assert!(!app.latest_only);
        assert_eq!(app.filtered().len(), 2);
    }

    #[test]
    fn year_filter_narrows_results() {
        let mut app = browse_app();
        app.latest_only = false;
        app.year_filter = Some(2024);
        let filtered = app.filtered();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].entry.year, Some(2024));
    }

    #[test]
    fn fuzzy_search_matches_terms_in_any_field() {
        assert!(fuzzy_score("Physics Textbook 2025", "phys").is_some());
        assert!(fuzzy_score("Physics Textbook 2025", "ptb").is_some());
        assert!(fuzzy_score("Physics Textbook", "chemistry").is_none());

        assert!(super::entry_matches_search(
            Catalog::from_toml("search", KSEEB)
                .unwrap()
                .entries()
                .next()
                .unwrap(),
            "maths"
        ));
    }

    #[test]
    fn preferred_language_is_listed_first() {
        let mut app = app();
        app.selection.board = Some("kseeb".to_owned());
        app.selection.level = Some("SSLC".to_owned());
        app.selection.subject = Some("Mathematics".to_owned());
        app.latest_only = false;

        app.config.language = Language::En;
        let first = app.filtered().first().copied().unwrap();
        assert_eq!(first.entry.language, Language::En);

        app.config.language = Language::Kn;
        let first = app.filtered().first().copied().unwrap();
        assert_eq!(first.entry.language, Language::Kn);
    }

    #[test]
    fn toggle_and_select_all_use_catalog_urls() {
        let mut app = browse_app();
        app.latest_only = false;
        app.browse_cursor = 0;

        app.toggle_selected();
        assert_eq!(app.selected.len(), 1);

        app.toggle_select_all();
        assert_eq!(app.selected.len(), 2);

        app.toggle_select_all();
        assert!(app.selected.is_empty());
    }

    #[test]
    fn search_changes_clear_the_selection() {
        let mut app = browse_app();
        app.latest_only = false;
        app.toggle_selected();
        assert_eq!(app.selected.len(), 1);

        app.open_search();
        app.push_search_char('2');
        app.push_search_char('0');
        app.push_search_char('2');
        app.push_search_char('4');
        assert!(app.selected.is_empty());
        assert_eq!(app.filtered().len(), 1);

        app.cancel_search();
        assert!(app.search.is_empty());
        assert_eq!(app.filtered().len(), 2);
    }

    #[test]
    fn breadcrumb_follows_the_selection() {
        let app = browse_app();
        assert_eq!(
            app.breadcrumb(),
            [
                "Karnataka State Board",
                "SSLC",
                "Mathematics",
                "Previous Year Papers"
            ]
        );
    }

    #[test]
    fn queue_events_update_rows_without_a_manager() {
        let mut app = app();
        let id = JobId(7);
        app.handle_download_event(DownloadEvent::Queued {
            id,
            title: "Physics".to_owned(),
        });
        app.handle_download_event(DownloadEvent::Progress {
            id,
            progress: ProgressSnapshot {
                downloaded: 50,
                total: Some(100),
                bytes_per_second: 25.0,
                eta: Some(Duration::from_secs(2)),
            },
        });
        app.handle_download_event(DownloadEvent::Completed {
            id,
            path: PathBuf::from("/tmp/physics.pdf"),
            bytes: 100,
        });

        let job = &app.queue.jobs[0];
        assert_eq!(job.status, JobStatus::Completed);
        assert_eq!(job.progress.fraction(), 1.0);
        assert_eq!(app.finished_downloads(), 1);
    }

    #[test]
    fn settings_change_config_without_touching_other_fields() {
        let mut app = app();
        app.screen = Screen::Settings;
        app.settings.cursor = 2; // concurrency
        let before = app.config.concurrency;
        app.settings_activate();
        assert_eq!(
            app.config.concurrency,
            if before >= MAX_CONCURRENCY {
                MIN_CONCURRENCY
            } else {
                before + 1
            }
        );

        app.settings.cursor = 3; // language
        let before = app.config.language;
        app.settings_adjust(1);
        assert_ne!(app.config.language, before);
    }

    #[test]
    fn back_navigation_preserves_the_chosen_item() {
        let mut app = app();
        app.selection.board = Some("pue".to_owned());
        app.selection.level = Some("PUC 1".to_owned());
        app.selection.stream = Some("Science".to_owned());
        app.selection.subject = Some("Physics".to_owned());
        app.selection.kind = Some(MaterialType::Textbook);
        app.screen = Screen::Browse;

        app.go_back();
        assert_eq!(app.screen, Screen::Material);
        assert_eq!(app.material_cursor, 0);

        app.go_back();
        assert_eq!(app.screen, Screen::Subject);

        app.go_back();
        assert_eq!(app.screen, Screen::Stream);
        assert_eq!(app.stream_cursor, 2);
    }

    #[test]
    fn year_picker_round_trips_and_clears_when_all_years_is_chosen() {
        let mut app = browse_app();
        app.open_year_picker();
        assert!(app.year_picker_open);

        app.year_cursor = 1;
        app.confirm_year();
        assert_eq!(app.year_filter, Some(2025));
        assert!(!app.year_picker_open);

        app.open_year_picker();
        app.year_cursor = 0;
        app.confirm_year();
        assert_eq!(app.year_filter, None);
    }
}
