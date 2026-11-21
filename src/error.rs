//! Error types shared across `pu-get`.
//!
//! The binary reports errors through [`anyhow`], but domain modules expose
//! precise [`thiserror`] enums so callers can react to individual failures
//! (a missing config file is not the same thing as a rejected download).

use std::path::PathBuf;

use thiserror::Error;

/// Errors raised while locating, parsing, validating or persisting
/// `pu-get` configuration.
#[derive(Debug, Error)]
pub enum ConfigError {
    /// The platform home directory could not be determined.
    #[error("could not locate the user home directory; pass --config <PATH> or set HOME")]
    HomeDirectoryNotFound,

    /// The configuration file exists but could not be read.
    #[error("could not read configuration file {}: {source}", path.display())]
    Read {
        /// Path that failed to read.
        path: PathBuf,
        /// Underlying I/O failure.
        #[source]
        source: std::io::Error,
    },

    /// The configuration file could not be written atomically.
    #[error("could not write configuration file {}: {source}", path.display())]
    Write {
        /// Path that failed to write.
        path: PathBuf,
        /// Underlying I/O failure.
        #[source]
        source: std::io::Error,
    },

    /// A required directory could not be created.
    #[error("could not create directory {}: {source}", path.display())]
    CreateDir {
        /// Directory that could not be created.
        path: PathBuf,
        /// Underlying I/O failure.
        #[source]
        source: std::io::Error,
    },

    /// The configuration file is not valid TOML.
    #[error("configuration file {} is not valid TOML: {source}", path.display())]
    Parse {
        /// Path that failed to parse.
        path: PathBuf,
        /// TOML parser diagnostic.
        #[source]
        source: toml::de::Error,
    },

    /// The in-memory configuration could not be serialised.
    #[error("could not serialise configuration as TOML: {0}")]
    Serialize(#[from] toml::ser::Error),

    /// A value supplied by the user is out of range or malformed.
    #[error("invalid value for `{key}`: {message}")]
    InvalidValue {
        /// Configuration key (`download_dir`, `concurrency`, ...).
        key: &'static str,
        /// Human-readable explanation of the expected value.
        message: String,
    },

    /// The key is not part of pu-get's configuration schema.
    #[error("unknown configuration key `{0}`")]
    UnknownKey(String),

    /// The file was written by a newer pu-get than this binary understands.
    #[error(
        "unsupported configuration schema version {found} (this build supports up to {supported})"
    )]
    UnsupportedSchema {
        /// Version found in the file.
        found: u32,
        /// Highest version supported by this binary.
        supported: u32,
    },
}

/// Convenience alias for configuration operations.
pub type ConfigResult<T> = Result<T, ConfigError>;

/// Errors raised while reading, parsing, validating or merging catalogs.
#[derive(Debug, Error)]
pub enum CatalogError {
    /// A catalog file is not valid TOML.
    #[error("catalog `{origin}` is not valid TOML: {source}")]
    Toml {
        /// Where the catalog came from (path or embedded name).
        origin: String,
        /// TOML parser diagnostic.
        #[source]
        source: toml::de::Error,
    },

    /// A catalog file is not valid JSON.
    #[error("catalog `{origin}` is not valid JSON: {source}")]
    Json {
        /// Where the catalog came from (path or embedded name).
        origin: String,
        /// JSON parser diagnostic.
        #[source]
        source: serde_json::Error,
    },

    /// A catalog file could not be read.
    #[error("could not read catalog file {}: {source}", path.display())]
    Read {
        /// File that failed to read.
        path: PathBuf,
        /// Underlying I/O failure.
        #[source]
        source: std::io::Error,
    },

    /// A catalog directory could not be enumerated.
    #[error("could not read catalog directory {}: {source}", path.display())]
    ReadDir {
        /// Directory that failed to read.
        path: PathBuf,
        /// Underlying I/O failure.
        #[source]
        source: std::io::Error,
    },

    /// A catalog file is not UTF-8.
    #[error("catalog `{origin}` is not valid UTF-8")]
    Utf8 {
        /// Where the catalog came from.
        origin: String,
    },

    /// A catalog file has an extension pu-get does not understand.
    #[error("unsupported catalog format for {}; expected .toml or .json", path.display())]
    UnsupportedFormat {
        /// File with the unsupported extension.
        path: PathBuf,
    },

    /// The board id is empty.
    #[error("catalog `{origin}` has an empty board id")]
    EmptyBoardId {
        /// Where the catalog came from.
        origin: String,
    },

    /// The board id contains characters that are unsafe in paths and filters.
    #[error(
        "catalog `{origin}` has invalid board id `{id}` (use ASCII letters, digits, '-' or '_')"
    )]
    InvalidBoardId {
        /// Where the catalog came from.
        origin: String,
        /// The rejected id.
        id: String,
    },

    /// The board display name is empty.
    #[error("catalog `{origin}` has an empty board name")]
    EmptyBoardName {
        /// Where the catalog came from.
        origin: String,
    },

    /// A required entry field is empty.
    #[error("entry #{index} in `{origin}` is missing `{field}`")]
    MissingField {
        /// Where the catalog came from.
        origin: String,
        /// Zero-based entry index inside the file.
        index: usize,
        /// Field that is empty.
        field: &'static str,
    },

    /// A field contains a value that failed validation.
    #[error("entry #{index} in `{origin}` has an invalid {field} `{value}`")]
    InvalidField {
        /// Where the catalog came from.
        origin: String,
        /// Zero-based entry index inside the file.
        index: usize,
        /// Field that failed validation.
        field: &'static str,
        /// Rejected value.
        value: String,
    },

    /// A year is outside the supported range.
    #[error("entry #{index} in `{origin}` has an invalid year {year} (expected 1900-2100)")]
    InvalidYear {
        /// Where the catalog came from.
        origin: String,
        /// Zero-based entry index inside the file.
        index: usize,
        /// Rejected year.
        year: u16,
    },

    /// A material type string is not recognised.
    #[error(
        "unknown material type `{0}`; expected textbook, model_paper, previous_year_paper, \
         sample_paper, marking_scheme or syllabus"
    )]
    UnknownMaterialType(String),

    /// An embedded catalog disappeared from the compiled binary.
    #[error("embedded catalog `{0}` is missing from the binary")]
    MissingEmbedded(String),
}

/// Convenience alias for catalog operations.
pub type CatalogResult<T> = Result<T, CatalogError>;

/// Errors raised while loading the resolver registry or crawling official sources.
#[derive(Debug, Error)]
pub enum ResolverError {
    /// A local registry file could not be read.
    #[error("could not read resolver registry {}: {source}", path.display())]
    Read {
        /// Registry path.
        path: PathBuf,
        /// Underlying I/O failure.
        #[source]
        source: std::io::Error,
    },

    /// A registry file is not valid TOML.
    #[error("resolver registry `{origin}` is not valid TOML: {source}")]
    Toml {
        /// Registry origin (path or embedded name).
        origin: String,
        /// Parser diagnostic.
        #[source]
        source: toml::de::Error,
    },

    /// The embedded registry is missing (build error).
    #[error("the compiled-in resolver registry is missing")]
    EmbeddedMissing,

    /// The registry failed validation.
    #[error("invalid resolver registry: {0}")]
    Invalid(String),

    /// The HTTP client could not be created.
    #[error("could not create the resolver HTTP client: {0}")]
    Client(String),

    /// A page or resource could not be fetched.
    #[error("could not fetch {url}: {message}")]
    Http {
        /// Requested URL.
        url: String,
        /// Transport or TLS message.
        message: String,
    },

    /// The server returned a non-success status.
    #[error("HTTP {status} while fetching {url}")]
    Status {
        /// Requested URL.
        url: String,
        /// HTTP status.
        status: u16,
    },

    /// A fetched page could not be interpreted.
    #[error("could not parse {url}: {message}")]
    Parse {
        /// Requested URL.
        url: String,
        /// Parser diagnostic.
        message: String,
    },
}

/// Convenience alias for resolver operations.
pub type ResolverResult<T> = Result<T, ResolverError>;

/// Errors raised while rendering a download destination path.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PathError {
    /// The configured folder template is invalid.
    #[error("invalid folder template: {0}")]
    Template(String),

    /// The template rendered to no path segments at all.
    #[error("folder template produced an empty destination path")]
    EmptyPath,
}

/// Errors raised by the download engine.
///
/// IO and HTTP errors are normalised into strings so events can be cloned and
/// rendered by both the TUI and the plain CLI.
#[derive(Debug, Clone, Error)]
pub enum DownloadError {
    /// The manager's command channel has been closed.
    #[error("the download manager is no longer running")]
    ManagerStopped,

    /// The manager configuration is invalid.
    #[error("invalid download configuration: {message}")]
    InvalidConfiguration {
        /// Explanation of the invalid setting.
        message: String,
    },

    /// The HTTP client could not be created.
    #[error("could not create the HTTP client: {message}")]
    Client {
        /// Underlying builder error.
        message: String,
    },

    /// A download URL is malformed or uses a non-HTTP scheme.
    #[error("invalid download URL `{url}`")]
    InvalidUrl {
        /// Rejected URL.
        url: String,
    },

    /// A file operation failed.
    #[error("file operation failed for {}: {message}", path.display())]
    Io {
        /// Path involved in the failure.
        path: PathBuf,
        /// Operating-system message.
        message: String,
    },

    /// The process lacks permission to read, create or write a path.
    #[error("permission denied for {}: {message}", path.display())]
    PermissionDenied {
        /// Path involved in the failure.
        path: PathBuf,
        /// Operating-system message.
        message: String,
    },

    /// The filesystem or quota is full.
    #[error("not enough disk space while writing {}: {message}", path.display())]
    DiskFull {
        /// Path involved in the failure.
        path: PathBuf,
        /// Operating-system message.
        message: String,
    },

    /// A network error occurred.
    #[error("network error: {message}")]
    Network {
        /// Transport-level message.
        message: String,
    },

    /// An HTTP request timed out.
    #[error("request timed out after {seconds} second(s)")]
    Timeout {
        /// Configured timeout.
        seconds: u64,
    },

    /// The server returned a non-success status.
    #[error("server returned HTTP {status} for {url}")]
    Http {
        /// HTTP status code.
        status: u16,
        /// Requested URL.
        url: String,
    },

    /// The server returned something that is clearly not a PDF.
    #[error("server returned `{content_type}` instead of a PDF for {url}")]
    InvalidContentType {
        /// Requested URL.
        url: String,
        /// Rejected Content-Type header.
        content_type: String,
    },

    /// The downloaded bytes do not start with the PDF magic marker.
    #[error("downloaded file is not a PDF: {}", path.display())]
    NotPdf {
        /// Temporary file that failed validation.
        path: PathBuf,
    },

    /// A final destination already exists.
    #[error("destination already exists: {}", path.display())]
    AlreadyExists {
        /// Colliding destination path.
        path: PathBuf,
    },
}

impl DownloadError {
    /// Whether retrying the request may succeed.
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::Network { .. } | Self::Timeout { .. } => true,
            Self::Http { status, .. } => matches!(*status, 408 | 425 | 429 | 500..=599),
            _ => false,
        }
    }
}
