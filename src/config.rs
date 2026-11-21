//! Persistent configuration and platform directories.
//!
//! `pu-get` keeps a single TOML file in the platform configuration directory
//! (`~/.config/pu-get/config.toml` on Linux, `~/Library/Application Support`
//! on macOS and `%APPDATA%` on Windows). The file is optional: when it is
//! absent the defaults below apply. Writes are atomic so an interrupted save
//! can never leave a truncated configuration behind.

use std::ffi::OsStr;
use std::fmt;
use std::fs;
use std::io::Write as _;
use std::path::{Component, Path, PathBuf};
use std::str::FromStr;

use directories::{BaseDirs, UserDirs};
use serde::{Deserialize, Serialize};

use crate::error::{ConfigError, ConfigResult};

/// Directory name used under the platform configuration/cache roots.
pub const APP_DIR_NAME: &str = "pu-get";

/// Configuration file name inside [`APP_DIR_NAME`].
pub const CONFIG_FILE_NAME: &str = "config.toml";

/// Optional cache-directory override (used by tests and portable installs).
pub const CACHE_DIR_ENV: &str = "PU_GET_CACHE_DIR";

/// Current configuration schema version.
pub const CONFIG_SCHEMA_VERSION: u32 = 1;

/// Default folder layout under the download directory.
pub const DEFAULT_FOLDER_TEMPLATE: &str = "{board}/{level}/{subject}/{type}";

/// Default number of simultaneous downloads.
pub const DEFAULT_CONCURRENCY: usize = 3;

/// Lowest accepted download concurrency.
pub const MIN_CONCURRENCY: usize = 1;

/// Highest accepted download concurrency.
pub const MAX_CONCURRENCY: usize = 16;

/// Default HTTP timeout for a single request attempt.
pub const DEFAULT_REQUEST_TIMEOUT_SECS: u64 = 60;

/// Largest accepted HTTP timeout.
pub const MAX_REQUEST_TIMEOUT_SECS: u64 = 600;

/// Default number of retries after the initial failed attempt.
pub const DEFAULT_RETRIES: u32 = 3;

/// Largest accepted retry count.
pub const MAX_RETRIES: u32 = 10;

/// Longest accepted folder template (bytes).
const MAX_FOLDER_TEMPLATE_LEN: usize = 1_024;

/// Placeholders accepted inside [`Config::folder_template`].
pub const FOLDER_TEMPLATE_PLACEHOLDERS: &[&str] = &[
    "board", "board_id", "level", "stream", "subject", "type", "year", "language", "source",
    "title",
];

/// Languages pu-get can label study material with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    /// English.
    #[default]
    En,
    /// Kannada.
    Kn,
    /// Hindi.
    Hi,
    /// Urdu.
    Ur,
}

impl Language {
    /// Every supported language, in display order.
    pub const ALL: [Self; 4] = [Self::En, Self::Kn, Self::Hi, Self::Ur];

    /// Stable two-letter code stored in catalogs and configuration.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::En => "en",
            Self::Kn => "kn",
            Self::Hi => "hi",
            Self::Ur => "ur",
        }
    }

    /// Human-readable English name.
    #[must_use]
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::En => "English",
            Self::Kn => "Kannada",
            Self::Hi => "Hindi",
            Self::Ur => "Urdu",
        }
    }
}

impl fmt::Display for Language {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl FromStr for Language {
    type Err = ConfigError;

    fn from_str(value: &str) -> ConfigResult<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "en" | "eng" | "english" => Ok(Self::En),
            "kn" | "kan" | "kannada" => Ok(Self::Kn),
            "hi" | "hin" | "hindi" => Ok(Self::Hi),
            "ur" | "urd" | "urdu" => Ok(Self::Ur),
            other => Err(ConfigError::InvalidValue {
                key: "language",
                message: format!("expected one of en, kn, hi, ur (got `{other}`)"),
            }),
        }
    }
}

/// Keys accepted by `pu-get config get|set`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConfigKey {
    /// Root directory for downloaded files.
    DownloadDir,
    /// Folder layout; see [`FOLDER_TEMPLATE_PLACEHOLDERS`].
    FolderTemplate,
    /// Preferred resource language.
    Language,
    /// Simultaneous downloads.
    Concurrency,
    /// Show only the newest material per subject/type by default.
    LatestOnly,
    /// Manifest URL used by `pu-get update`.
    UpdateUrl,
    /// HTTP timeout per request attempt.
    RequestTimeoutSecs,
    /// Number of retries for transient failures.
    Retries,
}

impl ConfigKey {
    /// Every configuration key, in display order.
    pub const ALL: [Self; 8] = [
        Self::DownloadDir,
        Self::FolderTemplate,
        Self::Language,
        Self::Concurrency,
        Self::LatestOnly,
        Self::UpdateUrl,
        Self::RequestTimeoutSecs,
        Self::Retries,
    ];

    /// TOML / CLI name of the key.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::DownloadDir => "download_dir",
            Self::FolderTemplate => "folder_template",
            Self::Language => "language",
            Self::Concurrency => "concurrency",
            Self::LatestOnly => "latest_only",
            Self::UpdateUrl => "update_url",
            Self::RequestTimeoutSecs => "request_timeout_secs",
            Self::Retries => "retries",
        }
    }

    /// Short explanation shown by `pu-get config list`.
    #[must_use]
    pub const fn description(self) -> &'static str {
        match self {
            Self::DownloadDir => "root directory for downloads",
            Self::FolderTemplate => "sub-folder layout below download_dir",
            Self::Language => "preferred language: en, kn, hi or ur",
            Self::Concurrency => "simultaneous downloads (1-16)",
            Self::LatestOnly => "hide older editions by default (true/false)",
            Self::UpdateUrl => "remote catalog manifest URL (empty disables)",
            Self::RequestTimeoutSecs => "HTTP timeout per attempt (1-600 seconds)",
            Self::Retries => "retries for transient network errors (0-10)",
        }
    }

    /// Parse a key from its canonical name (dashes are accepted as a
    /// convenience and normalised to underscores).
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        let normalised = value.trim().to_ascii_lowercase().replace('-', "_");
        Self::ALL
            .iter()
            .copied()
            .find(|key| key.as_str() == normalised.as_str())
    }
}

impl fmt::Display for ConfigKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for ConfigKey {
    type Err = ConfigError;

    fn from_str(value: &str) -> ConfigResult<Self> {
        Self::parse(value).ok_or_else(|| ConfigError::UnknownKey(value.to_owned()))
    }
}

/// Persisted application configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// On-disk schema version.
    pub schema_version: u32,
    /// Root directory for downloads. May contain `~`.
    pub download_dir: PathBuf,
    /// Folder layout below `download_dir`.
    pub folder_template: String,
    /// Preferred language when several editions exist.
    pub language: Language,
    /// Maximum number of simultaneous downloads.
    pub concurrency: usize,
    /// Whether browsers start with the "latest only" filter enabled.
    pub latest_only: bool,
    /// Remote manifest URL consulted by `pu-get update`.
    pub update_url: Option<String>,
    /// HTTP timeout per request attempt.
    pub request_timeout_secs: u64,
    /// Number of retries after the initial failure.
    pub retries: u32,
    /// Path the configuration was loaded from (not serialised).
    #[serde(skip)]
    path: Option<PathBuf>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            schema_version: CONFIG_SCHEMA_VERSION,
            download_dir: default_download_dir(),
            folder_template: DEFAULT_FOLDER_TEMPLATE.to_owned(),
            language: Language::En,
            concurrency: DEFAULT_CONCURRENCY,
            latest_only: true,
            update_url: None,
            request_timeout_secs: DEFAULT_REQUEST_TIMEOUT_SECS,
            retries: DEFAULT_RETRIES,
            path: None,
        }
    }
}

impl Config {
    /// Load configuration from `paths`, returning defaults when the file does
    /// not exist yet. A file that exists but cannot be parsed is an error.
    pub fn load(paths: &AppPaths) -> ConfigResult<Self> {
        let mut config = match fs::read_to_string(&paths.config_file) {
            Ok(contents) => toml::from_str(&contents).map_err(|source| ConfigError::Parse {
                path: paths.config_file.clone(),
                source,
            })?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(source) => {
                return Err(ConfigError::Read {
                    path: paths.config_file.clone(),
                    source,
                });
            }
        };

        config.path = Some(paths.config_file.clone());
        config.validate()?;
        Ok(config)
    }

    /// Validate and atomically persist the configuration.
    pub fn save(&self, paths: &AppPaths) -> ConfigResult<()> {
        self.validate()?;
        let contents = toml::to_string_pretty(self)?;
        atomic_write(&paths.config_file, contents.as_bytes())
    }

    /// Path the configuration was loaded from, if any.
    #[must_use]
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Absolute download directory with `~` expanded and relative paths
    /// resolved against the current working directory.
    #[must_use]
    pub fn resolved_download_dir(&self) -> PathBuf {
        absolutize(expand_tilde(&self.download_dir))
    }

    /// Current value of `key`, formatted for display.
    #[must_use]
    pub fn get(&self, key: ConfigKey) -> String {
        match key {
            ConfigKey::DownloadDir => self.download_dir.display().to_string(),
            ConfigKey::FolderTemplate => self.folder_template.clone(),
            ConfigKey::Language => self.language.code().to_owned(),
            ConfigKey::Concurrency => self.concurrency.to_string(),
            ConfigKey::LatestOnly => self.latest_only.to_string(),
            ConfigKey::UpdateUrl => self.update_url.clone().unwrap_or_default(),
            ConfigKey::RequestTimeoutSecs => self.request_timeout_secs.to_string(),
            ConfigKey::Retries => self.retries.to_string(),
        }
    }

    /// Parse and apply a new value for `key`, then validate the whole
    /// configuration. Callers are responsible for calling [`Self::save`].
    pub fn set(&mut self, key: ConfigKey, raw_value: &str) -> ConfigResult<()> {
        let value = raw_value.trim();
        match key {
            ConfigKey::DownloadDir => {
                if value.is_empty() {
                    return Err(invalid(key, "path must not be empty"));
                }
                self.download_dir = absolutize(expand_tilde(value));
            }
            ConfigKey::FolderTemplate => {
                self.folder_template = value.to_owned();
            }
            ConfigKey::Language => {
                self.language = value.parse()?;
            }
            ConfigKey::Concurrency => {
                self.concurrency = value.parse().map_err(|_| {
                    invalid(
                        key,
                        format!(
                            "expected a whole number between {MIN_CONCURRENCY} and {MAX_CONCURRENCY}"
                        ),
                    )
                })?;
            }
            ConfigKey::LatestOnly => {
                self.latest_only = parse_bool(value, key)?;
            }
            ConfigKey::UpdateUrl => {
                if value.is_empty() {
                    self.update_url = None;
                } else {
                    validate_update_url(value)?;
                    self.update_url = Some(value.to_owned());
                }
            }
            ConfigKey::RequestTimeoutSecs => {
                self.request_timeout_secs = value.parse().map_err(|_| {
                    invalid(
                        key,
                        format!("expected a whole number between 1 and {MAX_REQUEST_TIMEOUT_SECS}"),
                    )
                })?;
            }
            ConfigKey::Retries => {
                self.retries = value.parse().map_err(|_| {
                    invalid(
                        key,
                        format!("expected a whole number between 0 and {MAX_RETRIES}"),
                    )
                })?;
            }
        }

        self.validate()
    }

    /// Check all invariants that must hold before the configuration is used.
    pub fn validate(&self) -> ConfigResult<()> {
        if self.schema_version != CONFIG_SCHEMA_VERSION {
            return Err(ConfigError::UnsupportedSchema {
                found: self.schema_version,
                supported: CONFIG_SCHEMA_VERSION,
            });
        }

        if self.download_dir.as_os_str().is_empty() {
            return Err(invalid(ConfigKey::DownloadDir, "path must not be empty"));
        }

        if !(MIN_CONCURRENCY..=MAX_CONCURRENCY).contains(&self.concurrency) {
            return Err(invalid(
                ConfigKey::Concurrency,
                format!("must be between {MIN_CONCURRENCY} and {MAX_CONCURRENCY}"),
            ));
        }

        if self.request_timeout_secs == 0 || self.request_timeout_secs > MAX_REQUEST_TIMEOUT_SECS {
            return Err(invalid(
                ConfigKey::RequestTimeoutSecs,
                format!("must be between 1 and {MAX_REQUEST_TIMEOUT_SECS}"),
            ));
        }

        if self.retries > MAX_RETRIES {
            return Err(invalid(
                ConfigKey::Retries,
                format!("must be between 0 and {MAX_RETRIES}"),
            ));
        }

        validate_folder_template(&self.folder_template)?;

        if let Some(url) = &self.update_url {
            validate_update_url(url)?;
        }

        Ok(())
    }
}

/// Platform locations used by pu-get.
#[derive(Debug, Clone)]
pub struct AppPaths {
    /// Configuration file (`.../pu-get/config.toml`).
    pub config_file: PathBuf,
    /// Application configuration directory.
    pub config_dir: PathBuf,
    /// User-supplied catalogs that override or extend the embedded ones.
    pub catalog_dir: PathBuf,
    /// Cache directory for HTTP and update metadata.
    pub cache_dir: PathBuf,
    /// Mutable state derived from remote catalogs.
    pub state_dir: PathBuf,
}

impl AppPaths {
    /// Resolve platform directories, honouring an explicit `--config` path.
    ///
    /// An explicit path only overrides the configuration file; user catalogs
    /// live next to it and cached data still uses the platform cache root when
    /// one can be determined.
    pub fn discover(config_override: Option<&Path>) -> ConfigResult<Self> {
        if let Some(raw) = config_override {
            if raw.as_os_str().is_empty() {
                return Err(ConfigError::InvalidValue {
                    key: "config",
                    message: "path must not be empty".to_owned(),
                });
            }

            let config_file = absolutize(expand_tilde(raw));
            let cache_dir = platform_cache_dir().unwrap_or_else(|| {
                config_file
                    .parent()
                    .unwrap_or_else(|| Path::new("."))
                    .join(".pu-get-cache")
            });
            return Ok(Self::new(config_file, cache_dir));
        }

        let base = BaseDirs::new().ok_or(ConfigError::HomeDirectoryNotFound)?;
        let config_dir = base.config_dir().join(APP_DIR_NAME);
        let cache_dir = platform_cache_dir().unwrap_or_else(|| base.cache_dir().join(APP_DIR_NAME));
        Ok(Self::from_parts(
            config_dir.join(CONFIG_FILE_NAME),
            config_dir,
            cache_dir,
        ))
    }

    /// Build paths for an explicit configuration file and cache directory.
    /// Useful for tests and embedders.
    #[must_use]
    pub fn new(config_file: PathBuf, cache_dir: PathBuf) -> Self {
        let config_dir = config_file
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        Self::from_parts(config_file, config_dir, cache_dir)
    }

    fn from_parts(config_file: PathBuf, config_dir: PathBuf, cache_dir: PathBuf) -> Self {
        Self {
            catalog_dir: config_dir.join("catalogs"),
            state_dir: cache_dir.join("state"),
            config_file,
            config_dir,
            cache_dir,
        }
    }

    /// Create the configuration and user-catalog directories.
    pub fn ensure_config_dirs(&self) -> ConfigResult<()> {
        create_dir_all(&self.config_dir)?;
        create_dir_all(&self.catalog_dir)
    }

    /// Create the cache and state directories.
    pub fn ensure_cache_dirs(&self) -> ConfigResult<()> {
        create_dir_all(&self.cache_dir)?;
        create_dir_all(&self.state_dir)
    }
}

fn platform_cache_dir() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os(CACHE_DIR_ENV).filter(|value| !value.is_empty()) {
        return Some(expand_tilde(PathBuf::from(path)));
    }

    BaseDirs::new().map(|base| base.cache_dir().join(APP_DIR_NAME))
}

/// Default download directory: the platform Downloads folder plus `pu-get`.
#[must_use]
pub fn default_download_dir() -> PathBuf {
    if let Some(user_dirs) = UserDirs::new() {
        if let Some(downloads) = user_dirs.download_dir() {
            return downloads.join(APP_DIR_NAME);
        }
        return user_dirs.home_dir().join("Downloads").join(APP_DIR_NAME);
    }

    PathBuf::from("~").join("Downloads").join(APP_DIR_NAME)
}

/// Expand a leading `~` to the platform home directory.
///
/// Only a bare `~` or a `~/...` prefix is expanded; `~user` is left untouched.
#[must_use]
pub fn expand_tilde(path: impl AsRef<Path>) -> PathBuf {
    let path = path.as_ref();
    let mut components = path.components();

    match components.next() {
        Some(Component::Normal(first)) if first == OsStr::new("~") => BaseDirs::new().map_or_else(
            || path.to_path_buf(),
            |base| {
                let mut expanded = base.home_dir().to_path_buf();
                expanded.extend(components);
                expanded
            },
        ),
        _ => path.to_path_buf(),
    }
}

/// Validate a folder template, including placeholder names and traversal
/// safety. Download-path rendering relies on this validation.
pub fn validate_folder_template(template: &str) -> ConfigResult<()> {
    if template.trim().is_empty() {
        return Err(invalid(
            ConfigKey::FolderTemplate,
            "template must not be empty",
        ));
    }

    if template.len() > MAX_FOLDER_TEMPLATE_LEN {
        return Err(invalid(
            ConfigKey::FolderTemplate,
            format!("template must be at most {MAX_FOLDER_TEMPLATE_LEN} bytes"),
        ));
    }

    if template.contains('\0') {
        return Err(invalid(
            ConfigKey::FolderTemplate,
            "template must not contain NUL bytes",
        ));
    }

    let is_windows_absolute = template.starts_with('/')
        || template.starts_with('\\')
        || (template.as_bytes().get(1) == Some(&b':') && template.is_char_boundary(2));
    if Path::new(template).is_absolute() || is_windows_absolute {
        return Err(invalid(
            ConfigKey::FolderTemplate,
            "template must be relative to download_dir",
        ));
    }

    if template.split(['/', '\\']).any(|segment| segment == "..") {
        return Err(invalid(
            ConfigKey::FolderTemplate,
            "template must not contain `..` segments",
        ));
    }

    let bytes = template.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'{' => {
                let rest = &template[index + 1..];
                let Some(end) = rest.find('}') else {
                    return Err(invalid(
                        ConfigKey::FolderTemplate,
                        "unclosed `{` in template",
                    ));
                };
                let placeholder = &rest[..end];
                if placeholder.is_empty() {
                    return Err(invalid(
                        ConfigKey::FolderTemplate,
                        "empty `{}` placeholder in template",
                    ));
                }
                if !FOLDER_TEMPLATE_PLACEHOLDERS.contains(&placeholder) {
                    return Err(invalid(
                        ConfigKey::FolderTemplate,
                        format!(
                            "unknown placeholder `{{{placeholder}}}`; supported placeholders: {}",
                            FOLDER_TEMPLATE_PLACEHOLDERS.join(", ")
                        ),
                    ));
                }
                index += 1 + end + 1;
            }
            b'}' => {
                return Err(invalid(
                    ConfigKey::FolderTemplate,
                    "unmatched `}` in template",
                ));
            }
            _ => index += 1,
        }
    }

    Ok(())
}

fn parse_bool(value: &str, key: ConfigKey) -> ConfigResult<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => Err(invalid(key, "expected true or false")),
    }
}

fn validate_update_url(value: &str) -> ConfigResult<()> {
    let parsed = url::Url::parse(value)
        .map_err(|error| invalid(ConfigKey::UpdateUrl, format!("invalid URL: {error}")))?;

    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err(invalid(
            ConfigKey::UpdateUrl,
            "expected an absolute http(s) URL",
        ));
    }

    Ok(())
}

fn invalid(key: ConfigKey, message: impl Into<String>) -> ConfigError {
    ConfigError::InvalidValue {
        key: key.as_str(),
        message: message.into(),
    }
}

fn absolutize(path: PathBuf) -> PathBuf {
    if path.is_absolute() {
        return path;
    }

    std::env::current_dir().map_or(path.clone(), |current| current.join(path))
}

fn create_dir_all(path: &Path) -> ConfigResult<()> {
    fs::create_dir_all(path).map_err(|source| ConfigError::CreateDir {
        path: path.to_path_buf(),
        source,
    })
}

fn atomic_write(path: &Path, contents: &[u8]) -> ConfigResult<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));

    create_dir_all(parent)?;

    let mut temporary =
        tempfile::NamedTempFile::new_in(parent).map_err(|source| ConfigError::Write {
            path: path.to_path_buf(),
            source,
        })?;

    temporary
        .write_all(contents)
        .and_then(|()| temporary.as_file().sync_all())
        .map_err(|source| ConfigError::Write {
            path: path.to_path_buf(),
            source,
        })?;

    temporary
        .persist(path)
        .map_err(|error| ConfigError::Write {
            path: path.to_path_buf(),
            source: error.error,
        })?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_paths(directory: &Path) -> AppPaths {
        AppPaths::new(directory.join(CONFIG_FILE_NAME), directory.join("cache"))
    }

    #[test]
    fn language_accepts_codes_and_names() {
        assert_eq!("en".parse::<Language>().unwrap(), Language::En);
        assert_eq!("Kannada".parse::<Language>().unwrap(), Language::Kn);
        assert_eq!("hi".parse::<Language>().unwrap(), Language::Hi);
        assert_eq!("URDU".parse::<Language>().unwrap(), Language::Ur);
    }

    #[test]
    fn language_rejects_unknown_values() {
        let error = "klingon".parse::<Language>().unwrap_err();
        assert!(error.to_string().contains("expected one of en, kn, hi, ur"));
    }

    #[test]
    fn config_key_round_trips() {
        for key in ConfigKey::ALL {
            assert_eq!(ConfigKey::parse(key.as_str()), Some(key));
            assert_eq!(key.as_str().parse::<ConfigKey>().unwrap(), key);
        }
        assert_eq!(
            ConfigKey::parse("download-dir"),
            Some(ConfigKey::DownloadDir)
        );
        assert!(ConfigKey::parse("nope").is_none());
    }

    #[test]
    fn default_config_is_valid() {
        Config::default().validate().unwrap();
    }

    #[test]
    fn default_template_is_valid() {
        validate_folder_template(DEFAULT_FOLDER_TEMPLATE).unwrap();
    }

    #[test]
    fn template_rejects_unknown_placeholder() {
        let error = validate_folder_template("{board}/{nope}").unwrap_err();
        assert!(error.to_string().contains("unknown placeholder"));
    }

    #[test]
    fn template_rejects_traversal_and_absolute_paths() {
        assert!(validate_folder_template("{board}/../escape").is_err());
        assert!(validate_folder_template("/absolute/{board}").is_err());
        assert!(validate_folder_template("{board").is_err());
        assert!(validate_folder_template("{board}/}").is_err());
    }

    #[test]
    fn concurrency_bounds_are_enforced() {
        let mut config = Config::default();
        assert!(config
            .set(ConfigKey::Concurrency, &(MAX_CONCURRENCY + 1).to_string())
            .is_err());
        config
            .set(ConfigKey::Concurrency, &MIN_CONCURRENCY.to_string())
            .unwrap();
        assert_eq!(config.concurrency, MIN_CONCURRENCY);
    }

    #[test]
    fn booleans_accept_common_spellings() {
        let mut config = Config::default();
        config.set(ConfigKey::LatestOnly, "off").unwrap();
        assert!(!config.latest_only);
        config.set(ConfigKey::LatestOnly, "YES").unwrap();
        assert!(config.latest_only);
        assert!(config.set(ConfigKey::LatestOnly, "maybe").is_err());
    }

    #[test]
    fn download_dir_becomes_absolute() {
        let mut config = Config::default();
        config
            .set(ConfigKey::DownloadDir, "relative-study-material")
            .unwrap();
        assert!(config.download_dir.is_absolute());
    }

    #[test]
    fn update_url_must_be_http() {
        let mut config = Config::default();
        assert!(config
            .set(ConfigKey::UpdateUrl, "ftp://example.com/catalog.toml")
            .is_err());
        config
            .set(ConfigKey::UpdateUrl, "https://example.com/catalog.toml")
            .unwrap();
        assert_eq!(
            config.update_url.as_deref(),
            Some("https://example.com/catalog.toml")
        );
        config.set(ConfigKey::UpdateUrl, "").unwrap();
        assert!(config.update_url.is_none());
    }

    #[test]
    fn config_round_trips_through_disk() {
        let directory = tempfile::tempdir().unwrap();
        let paths = test_paths(directory.path());

        let mut config = Config::default();
        config
            .set(
                ConfigKey::DownloadDir,
                directory.path().join("Study").to_string_lossy().as_ref(),
            )
            .unwrap();
        config.set(ConfigKey::Concurrency, "5").unwrap();
        config.set(ConfigKey::Language, "kn").unwrap();
        config.save(&paths).unwrap();

        let loaded = Config::load(&paths).unwrap();
        assert_eq!(loaded.concurrency, 5);
        assert_eq!(loaded.language, Language::Kn);
        assert_eq!(loaded.download_dir, config.download_dir);
        assert_eq!(loaded.path(), Some(paths.config_file.as_path()));
    }

    #[test]
    fn missing_config_file_uses_defaults() {
        let directory = tempfile::tempdir().unwrap();
        let paths = test_paths(directory.path());
        let config = Config::load(&paths).unwrap();
        assert_eq!(config.schema_version, CONFIG_SCHEMA_VERSION);
        assert_eq!(config.folder_template, DEFAULT_FOLDER_TEMPLATE);
    }

    #[test]
    fn unknown_toml_fields_are_rejected() {
        let directory = tempfile::tempdir().unwrap();
        let paths = test_paths(directory.path());
        fs::write(&paths.config_file, "bogus_field = 1\n").unwrap();
        assert!(Config::load(&paths).is_err());
    }

    #[test]
    fn language_serialises_as_code() {
        let config = Config {
            language: Language::Kn,
            ..Config::default()
        };
        let text = toml::to_string(&config).unwrap();
        assert!(text.contains("language = \"kn\""));
    }

    #[test]
    fn expand_tilde_leaves_regular_paths_alone() {
        let path = PathBuf::from("/tmp/pu-get-test");
        assert_eq!(expand_tilde(&path), path);
    }
}
