//! PDF validation helpers.
//!
//! Content-Type alone is not trusted: official portals sometimes serve HTML
//! error pages with a `200 OK`. Every completed `.part` file must also begin
//! with the `%PDF-` magic marker before it is published.

use std::path::Path;

use tokio::io::AsyncReadExt as _;

use crate::error::DownloadError;

/// Bytes at the start of every valid PDF.
pub const PDF_MAGIC: &[u8] = b"%PDF-";

/// Whether a `Content-Type` value identifies a PDF or a generic binary stream.
///
/// `text/html`, `text/plain` and similar values are rejected. Missing headers
/// are accepted and fall back to magic-byte validation.
#[must_use]
pub fn content_type_allows_pdf(value: &str) -> bool {
    let mime = value
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();

    matches!(
        mime.as_str(),
        "application/pdf"
            | "application/x-pdf"
            | "application/octet-stream"
            | "binary/octet-stream"
    )
}

/// Whether a byte slice starts with the PDF magic marker.
#[must_use]
pub fn is_pdf_magic(bytes: &[u8]) -> bool {
    bytes.starts_with(PDF_MAGIC)
}

/// Read the first bytes of `path` and require the PDF magic marker.
pub async fn validate_pdf(path: &Path) -> Result<(), DownloadError> {
    let mut file = tokio::fs::File::open(path)
        .await
        .map_err(|error| DownloadError::Io {
            path: path.to_path_buf(),
            message: error.to_string(),
        })?;

    let mut header = [0_u8; PDF_MAGIC.len()];
    match file.read_exact(&mut header).await {
        Ok(_) if is_pdf_magic(&header) => Ok(()),
        Ok(_) => Err(DownloadError::NotPdf {
            path: path.to_path_buf(),
        }),
        Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => {
            Err(DownloadError::NotPdf {
                path: path.to_path_buf(),
            })
        }
        Err(error) => Err(DownloadError::Io {
            path: path.to_path_buf(),
            message: error.to_string(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_pdf_content_types() {
        assert!(content_type_allows_pdf("application/pdf"));
        assert!(content_type_allows_pdf("Application/PDF; charset=binary"));
        assert!(content_type_allows_pdf("application/octet-stream"));
        assert!(!content_type_allows_pdf("text/html; charset=utf-8"));
        assert!(!content_type_allows_pdf("text/plain"));
        assert!(!content_type_allows_pdf(""));
    }

    #[test]
    fn recognises_magic_bytes() {
        assert!(is_pdf_magic(b"%PDF-1.7\n"));
        assert!(is_pdf_magic(b"%PDF-"));
        assert!(!is_pdf_magic(b"%PD"));
        assert!(!is_pdf_magic(b"<html>"));
    }

    #[tokio::test]
    async fn validates_files_on_disk() {
        let directory = tempfile::tempdir().unwrap();

        let good = directory.path().join("good.pdf");
        tokio::fs::write(&good, b"%PDF-1.7\nmock\n").await.unwrap();
        validate_pdf(&good).await.unwrap();

        let bad = directory.path().join("bad.pdf");
        tokio::fs::write(&bad, b"<html>nope</html>").await.unwrap();
        assert!(matches!(
            validate_pdf(&bad).await,
            Err(DownloadError::NotPdf { .. })
        ));

        let short = directory.path().join("short.pdf");
        tokio::fs::write(&short, b"%PD").await.unwrap();
        assert!(matches!(
            validate_pdf(&short).await,
            Err(DownloadError::NotPdf { .. })
        ));
    }
}
