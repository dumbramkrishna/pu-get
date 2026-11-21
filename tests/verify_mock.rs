//! Integration tests for catalog URL verification against a mock HTTP server.

use std::time::Duration;

use pu_get::verify::{build_client, check_url, CheckStatus};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const PDF_PREFIX: &[u8] = b"%PDF-1.7";

#[tokio::test]
async fn accepts_head_with_pdf_content_type() {
    let server = MockServer::start().await;
    Mock::given(method("HEAD"))
        .and(path("/paper.pdf"))
        .respond_with(ResponseTemplate::new(200).insert_header("Content-Type", "application/pdf"))
        .mount(&server)
        .await;

    let client = build_client(Duration::from_secs(5)).unwrap();
    let status = check_url(&client, &format!("{}/paper.pdf", server.uri())).await;
    assert!(status.is_ok(), "{status:?}");
}

#[tokio::test]
async fn rejects_html_head_responses() {
    let server = MockServer::start().await;
    Mock::given(method("HEAD"))
        .and(path("/paper.pdf"))
        .respond_with(
            ResponseTemplate::new(200).insert_header("Content-Type", "text/html; charset=utf-8"),
        )
        .mount(&server)
        .await;

    let client = build_client(Duration::from_secs(5)).unwrap();
    let status = check_url(&client, &format!("{}/paper.pdf", server.uri())).await;
    assert!(matches!(status, CheckStatus::Failed { .. }), "{status:?}");
}

#[tokio::test]
async fn falls_back_to_range_get_and_checks_magic() {
    let server = MockServer::start().await;
    Mock::given(method("HEAD"))
        .and(path("/range.pdf"))
        .respond_with(ResponseTemplate::new(405))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/range.pdf"))
        .respond_with(
            ResponseTemplate::new(206)
                .insert_header("Content-Type", "application/pdf")
                .insert_header("Content-Range", "bytes 0-4/100")
                .set_body_bytes(PDF_PREFIX.to_vec()),
        )
        .mount(&server)
        .await;

    let client = build_client(Duration::from_secs(5)).unwrap();
    let status = check_url(&client, &format!("{}/range.pdf", server.uri())).await;
    assert!(status.is_ok(), "{status:?}");
}

#[tokio::test]
async fn rejects_range_get_without_pdf_magic() {
    let server = MockServer::start().await;
    Mock::given(method("HEAD"))
        .and(path("/fake.pdf"))
        .respond_with(ResponseTemplate::new(405))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/fake.pdf"))
        .respond_with(
            ResponseTemplate::new(206)
                .insert_header("Content-Type", "application/pdf")
                .set_body_bytes(b"<html>".to_vec()),
        )
        .mount(&server)
        .await;

    let client = build_client(Duration::from_secs(5)).unwrap();
    let status = check_url(&client, &format!("{}/fake.pdf", server.uri())).await;
    assert!(matches!(status, CheckStatus::Failed { .. }), "{status:?}");
}

#[tokio::test]
async fn reports_http_errors() {
    let server = MockServer::start().await;
    Mock::given(method("HEAD"))
        .and(path("/missing.pdf"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let client = build_client(Duration::from_secs(5)).unwrap();
    let status = check_url(&client, &format!("{}/missing.pdf", server.uri())).await;

    match status {
        CheckStatus::Failed { reason } => assert!(reason.contains("404"), "{reason}"),
        CheckStatus::Ok { .. } => panic!("404 must fail"),
    }
}
