//! Streaming download engine.
//!
//! [`spawn`] starts a background manager on the current Tokio runtime. The
//! returned [`DownloadHandle`] is cheap to clone and sends commands over an
//! unbounded channel; the accompanying [`EventReceiver`] receives queued,
//! progress, conflict and terminal events so the TUI never blocks on I/O.
//!
//! Files are written to `<destination>.part`, validated as PDFs, then published
//! without clobbering an existing destination.

pub mod manager;
pub mod path;
pub mod progress;
pub mod validate;

use std::fmt;
use std::path::{Path, PathBuf};

use crate::catalog::CatalogEntry;
use crate::error::PathError;

pub use manager::{
    spawn, ConflictResolution, DownloadEvent, DownloadHandle, ManagerConfig, SkipReason,
};
pub use path::{destination_path, part_path, PathContext};
pub use progress::ProgressSnapshot;
pub use validate::{content_type_allows_pdf, is_pdf_magic, validate_pdf};

/// Channel carrying [`DownloadEvent`] values to the UI or CLI.
pub type EventReceiver = tokio::sync::mpsc::Receiver<DownloadEvent>;

/// How to handle an existing destination file.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ConflictPolicy {
    /// Ask the user (the manager emits [`DownloadEvent::Conflict`]).
    #[default]
    Ask,
    /// Leave the existing file untouched.
    Skip,
    /// Download beside it using a `name (1).pdf` suffix.
    Rename,
    /// Replace the existing file (explicit opt-in only).
    Overwrite,
}

impl ConflictPolicy {
    /// Stable label for CLI help and logs.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ask => "ask",
            Self::Skip => "skip",
            Self::Rename => "rename",
            Self::Overwrite => "overwrite",
        }
    }
}

impl fmt::Display for ConflictPolicy {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Identifier assigned to each queued download.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct JobId(pub u64);

impl fmt::Display for JobId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

/// Everything the manager needs to download one file.
#[derive(Debug, Clone)]
pub struct DownloadSpec {
    /// Direct PDF URL.
    pub url: String,
    /// Final destination path (without the `.part` suffix).
    pub destination: PathBuf,
    /// Display title used in progress messages.
    pub title: String,
    /// Official source label.
    pub source: String,
    /// Known size, used for progress and same-size skips.
    pub expected_size: Option<u64>,
    /// Existing-file policy.
    pub conflict: ConflictPolicy,
}

impl DownloadSpec {
    /// Build a spec and resolve its destination from a catalog entry.
    pub fn from_entry(
        item: CatalogEntry<'_>,
        download_dir: &Path,
        folder_template: &str,
        conflict: ConflictPolicy,
    ) -> Result<Self, PathError> {
        let context = PathContext::from_entry(item);
        let destination = destination_path(download_dir, folder_template, &context)?;

        Ok(Self {
            url: item.entry.url.clone(),
            destination,
            title: item.entry.title.clone(),
            source: item.entry.source.clone(),
            expected_size: item.entry.size_bytes,
            conflict,
        })
    }
}
