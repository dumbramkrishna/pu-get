//! Catalog URL validation shared by `pu-get verify` and `pu-get crawl`.
//!
//! Validation prefers HEAD. Portals that reject HEAD fall back to a one-byte
//! ranged GET and check the PDF magic marker, so HTML error pages served with
//! `200 OK` are rejected.

use std::time::Duration;

use futures_util::stream::{self, StreamExt};
use reqwest::header::{ACCEPT, CONTENT_TYPE, RANGE};
use reqwest::{Client, StatusCode};
use url::Url;

use crate::download::{content_type_allows_pdf, is_pdf_magic};

const ACCEPT_HEADER: &str = "application/pdf,application/octet-stream;q=0.9,*/*;q=0.8";

/// Outcome of probing one URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckStatus {
    /// The URL is reachable and appears to be a PDF.
    Ok {
        /// HTTP status returned by the successful probe.
        http_status: u16,
    },
    /// The URL failed validation.
    Failed {
        /// Human-readable reason.
        reason: String,
    },
}

impl CheckStatus {
    /// Whether the URL passed validation.
    #[must_use]
    pub fn is_ok(&self) -> bool {
        matches!(self, Self::Ok { .. })
    }

    /// One-line label for CLI output.
    #[must_use]
    pub fn label(&self) -> String {
        match self {
            Self::Ok { http_status } => format!("OK ({http_status})"),
            Self::Failed { reason } => format!("FAIL ({reason})"),
        }
    }
}

/// One URL to validate, with a label for the report.
#[derive(Debug, Clone)]
pub struct VerifyTarget {
    /// Entry description shown in reports.
    pub label: String,
    /// Direct PDF URL.
    pub url: String,
}

/// A target and its validation outcome.
#[derive(Debug, Clone)]
pub struct VerifyResult {
    /// Original target.
    pub target: VerifyTarget,
    /// Probe outcome.
    pub status: CheckStatus,
}

/// Check many URLs with bounded concurrency.
pub async fn check_many(
    client: &Client,
    urls: &[String],
    concurrency: usize,
) -> Vec<(String, CheckStatus)> {
    stream::iter(urls.iter().cloned())
        .map(|url| {
            let client = client.clone();
            async move {
                let status = check_url(&client, &url).await;
                (url, status)
            }
        })
        .buffer_unordered(concurrency.max(1))
        .collect()
        .await
}

/// Validate one URL.
pub async fn check_url(client: &Client, url: &str) -> CheckStatus {
    let Ok(parsed) = Url::parse(url) else {
        return failed("invalid URL");
    };

    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return failed("URL must be absolute http(s)");
    }

    match client.head(url).header(ACCEPT, ACCEPT_HEADER).send().await {
        Ok(response) => {
            let status = response.status();

            if status.is_success() {
                let content_type = response
                    .headers()
                    .get(CONTENT_TYPE)
                    .and_then(|value| value.to_str().ok());

                match content_type {
                    Some(content_type) if !content_type_allows_pdf(content_type) => {
                        return failed(format!("unexpected content type `{content_type}`"));
                    }
                    Some(_) => {
                        return CheckStatus::Ok {
                            http_status: status.as_u16(),
                        };
                    }
                    None => {}
                }
            } else if !matches!(
                status,
                StatusCode::FORBIDDEN
                    | StatusCode::METHOD_NOT_ALLOWED
                    | StatusCode::NOT_IMPLEMENTED
            ) {
                return failed(format!("HTTP {}", status.as_u16()));
            }
        }
        Err(error) if error.is_connect() || error.is_timeout() => {
            return failed(crate::http::error_chain(&error));
        }
        Err(_) => {}
    }

    check_with_range_get(client, url).await
}

async fn check_with_range_get(client: &Client, url: &str) -> CheckStatus {
    let mut response = match client
        .get(url)
        .header(ACCEPT, ACCEPT_HEADER)
        .header(RANGE, "bytes=0-4")
        .send()
        .await
    {
        Ok(response) => response,
        Err(error) => return failed(crate::http::error_chain(&error)),
    };

    if !response.status().is_success() {
        return failed(format!("HTTP {}", response.status().as_u16()));
    }

    if let Some(content_type) = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
    {
        if !content_type_allows_pdf(content_type) {
            return failed(format!("unexpected content type `{content_type}`"));
        }
    }

    let mut prefix = Vec::with_capacity(5);
    while prefix.len() < 5 {
        match response.chunk().await {
            Ok(Some(chunk)) => prefix.extend_from_slice(&chunk),
            Ok(None) => break,
            Err(error) => return failed(crate::http::error_chain(&error)),
        }
    }

    if is_pdf_magic(&prefix) {
        // Reuse the status that produced the magic marker; some servers answer
        // Range requests with 200, which is still valid for validation.
        CheckStatus::Ok {
            http_status: response.status().as_u16(),
        }
    } else {
        failed("response does not start with %PDF-")
    }
}

/// Render a stable text report.
#[must_use]
pub fn render_report(results: &[VerifyResult]) -> String {
    let mut output = String::new();
    let mut failed = 0_usize;

    for result in results {
        if !result.status.is_ok() {
            failed += 1;
        }
        output.push_str(&result.status.label());
        output.push('\t');
        output.push_str(&result.target.label);
        output.push('\t');
        output.push_str(&result.target.url);
        output.push('\n');
    }

    output.push_str(&format!(
        "\n{} checked, {} ok, {} failed\n",
        results.len(),
        results.len() - failed,
        failed
    ));
    output
}

/// Build the HTTP client used by verification and crawling.
pub fn build_client(timeout: Duration) -> Result<Client, reqwest::Error> {
    crate::http::client(timeout)
}

fn failed(reason: impl Into<String>) -> CheckStatus {
    CheckStatus::Failed {
        reason: reason.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_summary() {
        let results = vec![
            VerifyResult {
                target: VerifyTarget {
                    label: "a".to_owned(),
                    url: "https://example.com/a.pdf".to_owned(),
                },
                status: CheckStatus::Ok { http_status: 200 },
            },
            VerifyResult {
                target: VerifyTarget {
                    label: "b".to_owned(),
                    url: "https://example.com/b.pdf".to_owned(),
                },
                status: CheckStatus::Failed {
                    reason: "HTTP 404".to_owned(),
                },
            },
        ];

        let report = render_report(&results);
        assert!(report.contains("OK (200)"));
        assert!(report.contains("FAIL (HTTP 404)"));
        assert!(report.contains("2 checked, 1 ok, 1 failed"));
    }
}
