//! Destination path rendering, filename sanitisation and collision handling.

use std::borrow::Cow;
use std::path::{Path, PathBuf};

use crate::catalog::CatalogEntry;
use crate::config::{expand_tilde, validate_folder_template};
use crate::error::PathError;

/// Maximum characters in one rendered folder component.
const MAX_COMPONENT_CHARS: usize = 120;

/// Maximum characters in the PDF filename stem.
const MAX_FILENAME_STEM_CHARS: usize = 150;

/// Windows device names that must never be used as a filename stem.
const RESERVED_FILENAMES: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$", "COM1", "COM2", "COM3", "COM4", "COM5",
    "COM6", "COM7", "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8",
    "LPT9",
];

/// Values substituted into folder templates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathContext {
    /// Board display name (`{board}`).
    pub board: String,
    /// Board id (`{board_id}`).
    pub board_id: String,
    /// Level (`{level}`).
    pub level: String,
    /// Stream, or `None` for boards without streams.
    pub stream: Option<String>,
    /// Subject (`{subject}`).
    pub subject: String,
    /// Material type id (`{type}`).
    pub kind: String,
    /// Year (`{year}`), rendered as `undated` when absent.
    pub year: Option<u16>,
    /// Language code (`{language}`).
    pub language: String,
    /// Source label (`{source}`).
    pub source: String,
    /// Entry title (`{title}`).
    pub title: String,
}

impl PathContext {
    /// Build path tokens from a catalog entry.
    #[must_use]
    pub fn from_entry(item: CatalogEntry<'_>) -> Self {
        Self {
            board: item.board.name.clone(),
            board_id: item.board.id.clone(),
            level: item.entry.level.clone(),
            stream: item.entry.stream.clone(),
            subject: item.entry.subject.clone(),
            kind: item.entry.kind.as_str().to_owned(),
            year: item.entry.year,
            language: item.entry.language.code().to_owned(),
            source: item.entry.source.clone(),
            title: item.entry.title.clone(),
        }
    }
}

/// Render a folder template and append a sanitised PDF filename.
///
/// `{stream}` becomes `general` when absent and `{year}` becomes `undated`, so
/// templates never produce empty path segments.
pub fn destination_path(
    root: &Path,
    template: &str,
    context: &PathContext,
) -> Result<PathBuf, PathError> {
    validate_folder_template(template).map_err(|error| PathError::Template(error.to_string()))?;

    let rendered = render_template(template, context);
    let mut destination = expand_tilde(root);
    let mut segment_count = 0;

    for segment in rendered.split(['/', '\\']) {
        if segment.trim().is_empty() {
            continue;
        }

        let segment = sanitize_with_limit(segment, MAX_COMPONENT_CHARS);
        destination.push(segment);
        segment_count += 1;
    }

    if segment_count == 0 {
        return Err(PathError::EmptyPath);
    }

    destination.push(sanitize_filename(&context.title));
    Ok(destination)
}

/// Sanitise one path component (folder name).
#[must_use]
pub fn sanitize_component(value: &str) -> String {
    sanitize_with_limit(value, MAX_COMPONENT_CHARS)
}

/// Produce a safe PDF filename from a title.
///
/// Invalid characters are replaced with `_`, trailing dots/spaces are removed,
/// Windows device names are prefixed with `_`, and the stem is truncated to
/// keep paths well below platform limits.
#[must_use]
pub fn sanitize_filename(title: &str) -> String {
    let trimmed = title.trim();
    let lowercase = trimmed.to_ascii_lowercase();
    let raw_stem = if lowercase.ends_with(".pdf") {
        &trimmed[..trimmed.len() - 4]
    } else {
        trimmed
    };

    let mut stem = sanitize_with_limit(raw_stem, MAX_FILENAME_STEM_CHARS);
    if RESERVED_FILENAMES.contains(&stem.to_ascii_uppercase().as_str()) {
        stem.insert(0, '_');
    }

    format!("{stem}.pdf")
}

/// Path of the partial file used while downloading `destination`.
#[must_use]
pub fn part_path(destination: &Path) -> PathBuf {
    let mut path = destination.as_os_str().to_os_string();
    path.push(".part");
    PathBuf::from(path)
}

/// Build `name (index).ext` beside `destination`.
#[must_use]
pub fn numbered_path(destination: &Path, index: u32) -> PathBuf {
    let stem = destination.file_stem().map_or_else(
        || "download".to_owned(),
        |stem| stem.to_string_lossy().into_owned(),
    );
    let extension = destination
        .extension()
        .map(|extension| extension.to_string_lossy().into_owned());

    let filename = match extension {
        Some(extension) => format!("{stem} ({index}).{extension}"),
        None => format!("{stem} ({index})"),
    };

    match destination
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        Some(parent) => parent.join(filename),
        None => PathBuf::from(filename),
    }
}

/// Find a numbered destination that neither exists nor has a `.part` file.
#[must_use]
pub fn next_available_path(destination: &Path) -> PathBuf {
    next_available_path_with(destination, |candidate| {
        candidate.exists() || part_path(candidate).exists()
    })
}

/// Find a numbered destination for which `taken` returns `false`.
#[must_use]
pub fn next_available_path_with(
    destination: &Path,
    mut taken: impl FnMut(&Path) -> bool,
) -> PathBuf {
    for index in 1..=9_999 {
        let candidate = numbered_path(destination, index);
        if !taken(&candidate) {
            return candidate;
        }
    }

    numbered_path(destination, 9_999)
}

fn render_template(template: &str, context: &PathContext) -> String {
    let mut rendered = String::with_capacity(template.len() + 32);
    let mut remaining = template;

    while let Some(open) = remaining.find('{') {
        rendered.push_str(&remaining[..open]);
        let after_open = &remaining[open + 1..];

        let Some(close) = after_open.find('}') else {
            rendered.push_str(&remaining[open..]);
            return rendered;
        };

        let placeholder = &after_open[..close];
        let value = value_for(placeholder, context);
        rendered.push_str(&sanitize_with_limit(&value, MAX_COMPONENT_CHARS));
        remaining = &after_open[close + 1..];
    }

    rendered.push_str(remaining);
    rendered
}

fn value_for<'a>(placeholder: &str, context: &'a PathContext) -> Cow<'a, str> {
    match placeholder {
        "board" => Cow::Borrowed(context.board.as_str()),
        "board_id" => Cow::Borrowed(context.board_id.as_str()),
        "level" => Cow::Borrowed(context.level.as_str()),
        "stream" => Cow::Borrowed(context.stream.as_deref().unwrap_or("general")),
        "subject" => Cow::Borrowed(context.subject.as_str()),
        "type" => Cow::Borrowed(context.kind.as_str()),
        "year" => context.year.map_or_else(
            || Cow::Borrowed("undated"),
            |year| Cow::Owned(year.to_string()),
        ),
        "language" => Cow::Borrowed(context.language.as_str()),
        "source" => Cow::Borrowed(context.source.as_str()),
        "title" => Cow::Borrowed(context.title.as_str()),
        _ => Cow::Borrowed(""),
    }
}

fn sanitize_with_limit(value: &str, max_chars: usize) -> String {
    let mut replaced = String::with_capacity(value.len());

    for character in value.chars() {
        if character.is_control() {
            continue;
        }

        if matches!(
            character,
            '/' | '\\' | '<' | '>' | ':' | '"' | '|' | '?' | '*'
        ) {
            replaced.push('_');
        } else {
            replaced.push(character);
        }
    }

    let mut cleaned: String = replaced.chars().take(max_chars).collect();
    cleaned = cleaned.trim().to_owned();

    while cleaned.ends_with(' ') || cleaned.ends_with('.') {
        cleaned.pop();
    }

    if cleaned.is_empty() || cleaned == "." || cleaned == ".." {
        "download".to_owned()
    } else {
        cleaned
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context() -> PathContext {
        PathContext {
            board: "Karnataka State Board".to_owned(),
            board_id: "kseeb".to_owned(),
            level: "SSLC".to_owned(),
            stream: None,
            subject: "Mathematics".to_owned(),
            kind: "previous_year_paper".to_owned(),
            year: Some(2025),
            language: "kn".to_owned(),
            source: "KSEAB".to_owned(),
            title: "SSLC Mathematics Previous Year Paper 2025".to_owned(),
        }
    }

    #[test]
    fn sanitises_path_components() {
        assert_eq!(sanitize_component("a/b\\c"), "a_b_c");
        assert_eq!(sanitize_component("a:b?c*d"), "a_b_c_d");
        assert_eq!(sanitize_component(".."), "download");
        assert_eq!(sanitize_component("  ..  "), "download");
        assert_eq!(sanitize_component("trailing..."), "trailing");
    }

    #[test]
    fn sanitises_filenames() {
        assert_eq!(
            sanitize_filename("SSLC / Maths: 2025.pdf"),
            "SSLC _ Maths_ 2025.pdf"
        );
        assert_eq!(sanitize_filename("report.PDF"), "report.pdf");
        assert_eq!(sanitize_filename("CON"), "_CON.pdf");
        assert_eq!(sanitize_filename(""), "download.pdf");
        assert!(sanitize_filename(&"a".repeat(500)).chars().count() <= 154);
    }

    #[test]
    fn renders_template_and_filename() {
        let directory = tempfile::tempdir().unwrap();
        let path = destination_path(
            directory.path(),
            "{board_id}/{level}/{subject}/{type}",
            &context(),
        )
        .unwrap();

        let expected = directory
            .path()
            .join("kseeb")
            .join("SSLC")
            .join("Mathematics")
            .join("previous_year_paper")
            .join("SSLC Mathematics Previous Year Paper 2025.pdf");
        assert_eq!(path, expected);
    }

    #[test]
    fn missing_stream_and_year_have_stable_defaults() {
        let directory = tempfile::tempdir().unwrap();
        let path = destination_path(
            directory.path(),
            "{stream}/{year}",
            &PathContext {
                stream: None,
                year: None,
                ..context()
            },
        )
        .unwrap();

        assert_eq!(
            path,
            directory
                .path()
                .join("general")
                .join("undated")
                .join("SSLC Mathematics Previous Year Paper 2025.pdf")
        );
    }

    #[test]
    fn rejects_invalid_templates() {
        let directory = tempfile::tempdir().unwrap();
        assert!(matches!(
            destination_path(directory.path(), "/absolute/{board}", &context()),
            Err(PathError::Template(_))
        ));
        assert!(matches!(
            destination_path(directory.path(), "{nope}", &context()),
            Err(PathError::Template(_))
        ));
    }

    #[test]
    fn numbering_avoids_existing_files_and_parts() {
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("paper.pdf");
        std::fs::write(&destination, b"existing").unwrap();
        std::fs::write(part_path(&destination), b"partial").unwrap();

        assert_eq!(
            next_available_path(&destination),
            directory.path().join("paper (1).pdf")
        );

        std::fs::write(directory.path().join("paper (1).pdf"), b"existing").unwrap();
        assert_eq!(
            next_available_path(&destination),
            directory.path().join("paper (2).pdf")
        );
    }

    #[test]
    fn part_path_appends_suffix() {
        let path = Path::new("/tmp/books/paper.pdf");
        assert_eq!(part_path(path), Path::new("/tmp/books/paper.pdf.part"));
    }
}
