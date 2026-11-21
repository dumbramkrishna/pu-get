//! Integration tests for the download manager against a local mock HTTP server.
//!
//! Covers streaming, reset-safe publishing, resume with Range, retry/backoff,
//! PDF validation, 404 handling, same-size skips, conflict resolution and
//! queued pause/resume.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use pu_get::download::{
    part_path, spawn, ConflictPolicy, ConflictResolution, DownloadEvent, DownloadSpec, JobId,
    ManagerConfig,
};
use pu_get::error::DownloadError;
use tokio::sync::mpsc::Receiver;
use tokio::time::timeout;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

const PDF: &[u8] = b"%PDF-1.7\nmock pdf payload\n";

fn manager_config(concurrency: usize, retries: u32) -> ManagerConfig {
    ManagerConfig {
        concurrency,
        retries,
        request_timeout: Duration::from_secs(5),
    }
}

fn spec(url: String, destination: PathBuf, conflict: ConflictPolicy) -> DownloadSpec {
    DownloadSpec {
        url,
        destination,
        title: "Mock paper".to_owned(),
        source: "Test".to_owned(),
        expected_size: None,
        conflict,
    }
}

async fn next_event(events: &mut Receiver<DownloadEvent>) -> DownloadEvent {
    timeout(Duration::from_secs(10), events.recv())
        .await
        .expect("timed out waiting for download event")
        .expect("download manager closed the event channel")
}

async fn wait_for(
    events: &mut Receiver<DownloadEvent>,
    predicate: impl Fn(&DownloadEvent) -> bool,
) -> DownloadEvent {
    loop {
        let event = next_event(events).await;
        if predicate(&event) {
            return event;
        }
    }
}

fn completed(id: JobId) -> impl Fn(&DownloadEvent) -> bool {
    move |event| matches!(event, DownloadEvent::Completed { id: event_id, .. } if *event_id == id)
}

fn failed(id: JobId) -> impl Fn(&DownloadEvent) -> bool {
    move |event| matches!(event, DownloadEvent::Failed { id: event_id, .. } if *event_id == id)
}

async fn mount_pdf(server: &MockServer, route: &str) {
    Mock::given(method("GET"))
        .and(path(route))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("Content-Type", "application/pdf")
                .set_body_bytes(PDF.to_vec()),
        )
        .mount(server)
        .await;
}

#[tokio::test]
async fn streams_and_publishes_a_validated_pdf() {
    let server = MockServer::start().await;
    mount_pdf(&server, "/file.pdf").await;

    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("nested").join("file.pdf");
    let (handle, mut events) = spawn(manager_config(2, 0)).unwrap();
    let id = handle
        .enqueue(spec(
            format!("{}/file.pdf", server.uri()),
            destination.clone(),
            ConflictPolicy::Skip,
        ))
        .unwrap();

    let event = wait_for(&mut events, completed(id)).await;
    let DownloadEvent::Completed { path, bytes, .. } = event else {
        unreachable!()
    };

    assert_eq!(path, destination);
    assert_eq!(bytes, PDF.len() as u64);
    assert_eq!(tokio::fs::read(&destination).await.unwrap().as_slice(), PDF);
    assert!(!part_path(&destination).exists());
    handle.shutdown().unwrap();
}

#[tokio::test]
async fn resumes_a_partial_file_with_range_requests() {
    let server = MockServer::start().await;
    let body = PDF.to_vec();
    let responder_body = body.clone();

    Mock::given(method("GET"))
        .and(path("/resume.pdf"))
        .respond_with(move |request: &Request| {
            let range = request
                .headers
                .get("range")
                .and_then(|value| value.to_str().ok());

            if let Some(range) = range {
                let start: usize = range
                    .trim_start_matches("bytes=")
                    .split('-')
                    .next()
                    .expect("range start")
                    .parse()
                    .expect("numeric range start");
                let remaining = responder_body[start..].to_vec();

                ResponseTemplate::new(206)
                    .insert_header("Content-Type", "application/pdf")
                    .insert_header(
                        "Content-Range",
                        format!(
                            "bytes {start}-{}/{}",
                            responder_body.len() - 1,
                            responder_body.len()
                        ),
                    )
                    .set_body_bytes(remaining)
            } else {
                ResponseTemplate::new(200)
                    .insert_header("Content-Type", "application/pdf")
                    .set_body_bytes(responder_body.clone())
            }
        })
        .mount(&server)
        .await;

    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("resume.pdf");
    tokio::fs::create_dir_all(destination.parent().unwrap())
        .await
        .unwrap();

    let split = PDF.len() / 2;
    tokio::fs::write(part_path(&destination), &PDF[..split])
        .await
        .unwrap();

    let (handle, mut events) = spawn(manager_config(2, 0)).unwrap();
    let id = handle
        .enqueue(spec(
            format!("{}/resume.pdf", server.uri()),
            destination.clone(),
            ConflictPolicy::Skip,
        ))
        .unwrap();

    wait_for(&mut events, completed(id)).await;

    assert_eq!(tokio::fs::read(&destination).await.unwrap().as_slice(), PDF);
    assert!(!part_path(&destination).exists());

    let requests = server.received_requests().await.unwrap();
    assert!(
        requests
            .iter()
            .any(|request| request.headers.contains_key("range")),
        "expected at least one Range request"
    );

    handle.shutdown().unwrap();
}

#[tokio::test]
async fn retries_transient_server_errors() {
    let server = MockServer::start().await;
    let calls = Arc::new(AtomicUsize::new(0));
    let responder_calls = Arc::clone(&calls);

    Mock::given(method("GET"))
        .and(path("/flaky.pdf"))
        .respond_with(move |_request: &Request| {
            if responder_calls.fetch_add(1, Ordering::SeqCst) == 0 {
                ResponseTemplate::new(500)
            } else {
                ResponseTemplate::new(200)
                    .insert_header("Content-Type", "application/pdf")
                    .set_body_bytes(PDF.to_vec())
            }
        })
        .mount(&server)
        .await;

    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("flaky.pdf");
    let (handle, mut events) = spawn(manager_config(2, 2)).unwrap();
    let id = handle
        .enqueue(spec(
            format!("{}/flaky.pdf", server.uri()),
            destination.clone(),
            ConflictPolicy::Skip,
        ))
        .unwrap();

    let event = wait_for(&mut events, |event| completed(id)(event)).await;
    assert!(matches!(event, DownloadEvent::Completed { .. }));
    assert!(calls.load(Ordering::SeqCst) >= 2);
    assert_eq!(tokio::fs::read(&destination).await.unwrap().as_slice(), PDF);
    handle.shutdown().unwrap();
}

#[tokio::test]
async fn rejects_html_content_types() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/not-a-pdf.pdf"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("Content-Type", "text/html; charset=utf-8")
                .set_body_bytes(b"<html>error</html>".to_vec()),
        )
        .mount(&server)
        .await;

    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("not-a-pdf.pdf");
    let (handle, mut events) = spawn(manager_config(2, 0)).unwrap();
    let id = handle
        .enqueue(spec(
            format!("{}/not-a-pdf.pdf", server.uri()),
            destination.clone(),
            ConflictPolicy::Skip,
        ))
        .unwrap();

    let event = wait_for(&mut events, failed(id)).await;
    let DownloadEvent::Failed { error, .. } = event else {
        unreachable!()
    };
    assert!(matches!(error, DownloadError::InvalidContentType { .. }));
    assert!(!destination.exists());
    assert!(!part_path(&destination).exists());
    handle.shutdown().unwrap();
}

#[tokio::test]
async fn rejects_pdf_content_without_magic_bytes() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/fake.pdf"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("Content-Type", "application/pdf")
                .set_body_bytes(b"this is definitely not a pdf".to_vec()),
        )
        .mount(&server)
        .await;

    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("fake.pdf");
    let (handle, mut events) = spawn(manager_config(2, 0)).unwrap();
    let id = handle
        .enqueue(spec(
            format!("{}/fake.pdf", server.uri()),
            destination.clone(),
            ConflictPolicy::Skip,
        ))
        .unwrap();

    let event = wait_for(&mut events, failed(id)).await;
    let DownloadEvent::Failed { error, .. } = event else {
        unreachable!()
    };
    assert!(matches!(error, DownloadError::NotPdf { .. }));
    assert!(!destination.exists());
    assert!(!part_path(&destination).exists());
    handle.shutdown().unwrap();
}

#[tokio::test]
async fn reports_http_errors_without_retrying() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/missing.pdf"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("missing.pdf");
    let (handle, mut events) = spawn(manager_config(2, 3)).unwrap();
    let id = handle
        .enqueue(spec(
            format!("{}/missing.pdf", server.uri()),
            destination,
            ConflictPolicy::Skip,
        ))
        .unwrap();

    let event = wait_for(&mut events, failed(id)).await;
    let DownloadEvent::Failed { error, .. } = event else {
        unreachable!()
    };
    assert!(matches!(error, DownloadError::Http { status: 404, .. }));

    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1, "404 must not be retried");
    handle.shutdown().unwrap();
}

#[tokio::test]
async fn skips_an_existing_file_with_the_expected_size() {
    let server = MockServer::start().await;
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("already.pdf");
    tokio::fs::write(&destination, PDF).await.unwrap();

    let mut request = spec(
        format!("{}/already.pdf", server.uri()),
        destination.clone(),
        ConflictPolicy::Ask,
    );
    request.expected_size = Some(PDF.len() as u64);

    let (handle, mut events) = spawn(manager_config(2, 0)).unwrap();
    let id = handle.enqueue(request).unwrap();

    let event = wait_for(
        &mut events,
        |event| matches!(event, DownloadEvent::Skipped { id: event_id, .. } if *event_id == id),
    )
    .await;
    let DownloadEvent::Skipped { path, reason, .. } = event else {
        unreachable!()
    };
    assert_eq!(path, destination);
    assert_eq!(reason.message(), "already downloaded (matching size)");
    assert_eq!(tokio::fs::read(&destination).await.unwrap().as_slice(), PDF);
    assert!(
        server.received_requests().await.unwrap().is_empty(),
        "a same-size skip must not contact the server"
    );
    handle.shutdown().unwrap();
}

#[tokio::test]
async fn skips_an_existing_file_when_head_reports_the_same_size() {
    let server = MockServer::start().await;
    Mock::given(method("HEAD"))
        .and(path("/head.pdf"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("Content-Length", PDF.len().to_string())
                .insert_header("Content-Type", "application/pdf"),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/head.pdf"))
        .respond_with(
            ResponseTemplate::new(206)
                .insert_header("Content-Type", "application/pdf")
                .insert_header("Content-Range", format!("bytes 0-0/{}", PDF.len()))
                .set_body_bytes(vec![PDF[0]]),
        )
        .mount(&server)
        .await;

    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("head.pdf");
    tokio::fs::write(&destination, PDF).await.unwrap();

    let (handle, mut events) = spawn(manager_config(2, 0)).unwrap();
    let id = handle
        .enqueue(spec(
            format!("{}/head.pdf", server.uri()),
            destination.clone(),
            ConflictPolicy::Ask,
        ))
        .unwrap();

    let event = wait_for(
        &mut events,
        |event| matches!(event, DownloadEvent::Skipped { id: event_id, .. } if *event_id == id),
    )
    .await;
    let DownloadEvent::Skipped { reason, .. } = event else {
        unreachable!()
    };
    assert_eq!(reason.message(), "already downloaded (matching size)");

    let requests = server.received_requests().await.unwrap();
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.method.as_str() == "GET")
            .count(),
        1,
        "only the one-byte range probe should use GET"
    );
    assert!(
        requests
            .iter()
            .any(|request| request.method.as_str() == "HEAD"),
        "the HEAD probe should be attempted first"
    );
    handle.shutdown().unwrap();
}

#[tokio::test]
async fn asks_before_renaming_an_existing_conflict() {
    let server = MockServer::start().await;
    mount_pdf(&server, "/conflict.pdf").await;

    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("conflict.pdf");
    tokio::fs::write(&destination, b"old file").await.unwrap();

    let (handle, mut events) = spawn(manager_config(2, 0)).unwrap();
    let id = handle
        .enqueue(spec(
            format!("{}/conflict.pdf", server.uri()),
            destination.clone(),
            ConflictPolicy::Ask,
        ))
        .unwrap();

    wait_for(
        &mut events,
        |event| matches!(event, DownloadEvent::Conflict { id: event_id, .. } if *event_id == id),
    )
    .await;

    handle
        .resolve_conflict(id, ConflictResolution::Rename)
        .unwrap();

    let event = wait_for(&mut events, completed(id)).await;
    let DownloadEvent::Completed { path: renamed, .. } = event else {
        unreachable!()
    };
    assert_eq!(renamed, directory.path().join("conflict (1).pdf"));
    assert_eq!(
        tokio::fs::read(&destination).await.unwrap().as_slice(),
        b"old file"
    );
    assert_eq!(tokio::fs::read(&renamed).await.unwrap().as_slice(), PDF);
    handle.shutdown().unwrap();
}

#[tokio::test]
async fn pauses_and_resumes_a_queued_download() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/slow.pdf"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("Content-Type", "application/pdf")
                .set_body_bytes(PDF.to_vec())
                .set_delay(Duration::from_millis(250)),
        )
        .mount(&server)
        .await;
    mount_pdf(&server, "/fast.pdf").await;

    let directory = tempfile::tempdir().unwrap();
    let (handle, mut events) = spawn(manager_config(1, 0)).unwrap();

    let slow = handle
        .enqueue(spec(
            format!("{}/slow.pdf", server.uri()),
            directory.path().join("slow.pdf"),
            ConflictPolicy::Skip,
        ))
        .unwrap();
    let fast = handle
        .enqueue(spec(
            format!("{}/fast.pdf", server.uri()),
            directory.path().join("fast.pdf"),
            ConflictPolicy::Skip,
        ))
        .unwrap();

    handle.pause(fast).unwrap();

    wait_for(
        &mut events,
        |event| matches!(event, DownloadEvent::Paused { id: event_id, .. } if *event_id == fast),
    )
    .await;

    handle.resume(fast).unwrap();

    let mut finished = std::collections::HashSet::new();
    while finished.len() < 2 {
        let event = next_event(&mut events).await;
        if let DownloadEvent::Completed { id, .. } = event {
            finished.insert(id);
        }
    }

    assert!(finished.contains(&slow));
    assert!(finished.contains(&fast));
    assert!(directory.path().join("slow.pdf").exists());
    assert!(directory.path().join("fast.pdf").exists());
    handle.shutdown().unwrap();
}
