//! End-to-end test for `pu-get update` using a mock manifest host.

use std::process::Command;

use sha2::{Digest, Sha256};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const CATALOG: &str = r#"
[board]
id = "smoke"
name = "Smoke Test Board"

[[entries]]
level = "10"
subject = "Science"
type = "textbook"
title = "Smoke Textbook"
language = "en"
url = "https://example.com/smoke.pdf"
source = "Smoke"
"#;

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

#[tokio::test]
async fn installs_checks_and_reports_an_up_to_date_catalog() {
    let server = MockServer::start().await;
    let checksum = sha256_hex(CATALOG.as_bytes());
    let manifest = format!(
        "schema_version = 1\n\
         catalog_version = \"2026.11.21\"\n\
         catalog_url = \"catalog.toml\"\n\
         sha256 = \"{checksum}\"\n"
    );

    Mock::given(method("GET"))
        .and(path("/manifest.toml"))
        .respond_with(ResponseTemplate::new(200).set_body_string(manifest))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/catalog.toml"))
        .respond_with(ResponseTemplate::new(200).set_body_string(CATALOG))
        .mount(&server)
        .await;

    let directory = tempfile::tempdir().unwrap();
    let config_path = directory.path().join("config.toml");
    let cache_path = directory.path().join("cache");
    let manifest_url = format!("{}/manifest.toml", server.uri());

    let output = run_update(&config_path, &cache_path, &manifest_url, false).await;
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let catalog_path = directory.path().join("catalogs").join("remote.toml");
    assert_eq!(
        std::fs::read_to_string(&catalog_path).unwrap(),
        CATALOG,
        "installed catalog differs"
    );
    assert!(
        cache_path.join("state").join("update-state.json").exists(),
        "update state missing"
    );

    let check = run_update(&config_path, &cache_path, &manifest_url, true).await;
    assert!(check.status.success());
    let stdout = String::from_utf8_lossy(&check.stdout);
    assert!(stdout.contains("up to date"), "{stdout}");
}

async fn run_update(
    config: &std::path::Path,
    cache: &std::path::Path,
    manifest_url: &str,
    check: bool,
) -> std::process::Output {
    let binary = env!("CARGO_BIN_EXE_pu-get");
    let mut command = Command::new(binary);
    command
        .arg("--config")
        .arg(config)
        .arg("update")
        .arg("--remote")
        .arg(manifest_url)
        .env("PU_GET_CACHE_DIR", cache);

    if check {
        command.arg("--check");
    }

    tokio::task::spawn_blocking(move || command.output().unwrap())
        .await
        .unwrap()
}
