//! Fetch + cache + sha256-verify tests for `assay install`.
//!
//! Phase 2 of plan 21. Drives the fetch path against a `wiremock`
//! server to cover happy-path / cache-hit / mismatch / 404 / offline
//! scenarios end-to-end.

use std::collections::HashMap;
use std::path::Path;

use assay::install::fetch::{FetchError, FetchPlan, fetch};
use assay::install::manifest::{Extension, Lib};
use data_encoding::HEXLOWER;
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

// --- helpers ----------------------------------------------------------

fn sha(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    HEXLOWER.encode(&h.finalize())
}

fn lib(name: &str, version: &str, sha256: &str, source: Option<&str>) -> Lib {
    Lib {
        name: name.into(),
        version: version.into(),
        sha256: sha256.into(),
        source: source.map(str::to_string),
    }
}

fn ext(
    name: &str,
    version: &str,
    arch_hashes: &[(&str, &str)],
    source: Option<&str>,
) -> Extension {
    let mut sha256 = HashMap::new();
    for (a, h) in arch_hashes {
        sha256.insert((*a).to_string(), (*h).to_string());
    }
    Extension {
        name: name.into(),
        version: version.into(),
        sha256,
        source: source.map(str::to_string),
    }
}

async fn mock_serving(body: &[u8], path_str: &str) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(path_str))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(body.to_vec()))
        .mount(&server)
        .await;
    server
}

fn pre_populate(cache_dir: &Path, name: &str, bytes: &[u8]) -> std::path::PathBuf {
    let p = cache_dir.join(name);
    std::fs::write(&p, bytes).unwrap();
    p
}

// --- FetchPlan construction ------------------------------------------

#[test]
fn extension_plan_builds_url_and_cache_path() {
    let e = ext(
        "assay-engine",
        "0.4.1",
        &[("x86_64", "aaaa"), ("aarch64", "bbbb")],
        None,
    );
    let cache = TempDir::new().unwrap();
    let plan = FetchPlan::for_extension(&e, "x86_64", cache.path()).unwrap();

    assert_eq!(plan.display_name, "assay-engine 0.4.1");
    assert_eq!(plan.expected_sha256, "aaaa");
    assert_eq!(
        plan.url,
        "https://github.com/developerinlondon/assay/releases/download/v0.4.1/assay-engine-0.4.1-x86_64.tar.gz"
    );
    assert_eq!(
        plan.cache_path,
        cache.path().join("assay-engine-0.4.1-x86_64.tar.gz")
    );
}

#[test]
fn extension_plan_errors_when_arch_sha_missing() {
    let e = ext("assay-engine", "0.4.1", &[("x86_64", "aaaa")], None);
    let cache = TempDir::new().unwrap();
    let err = FetchPlan::for_extension(&e, "aarch64", cache.path()).unwrap_err();
    assert!(matches!(err, FetchError::NoArchHash { .. }));
    let msg = err.to_string();
    assert!(msg.contains("aarch64"));
    assert!(msg.contains("x86_64")); // available archs in error
}

#[test]
fn extension_plan_uses_source_override() {
    let e = ext(
        "assay-engine",
        "0.4.1",
        &[("x86_64", "aaaa")],
        Some("https://mirror.example/eng.tar.gz"),
    );
    let cache = TempDir::new().unwrap();
    let plan = FetchPlan::for_extension(&e, "x86_64", cache.path()).unwrap();
    assert_eq!(plan.url, "https://mirror.example/eng.tar.gz");
}

#[test]
fn lib_plan_builds_url_and_cache_path() {
    let l = lib("hostops", "0.1.0", "cccc", None);
    let cache = TempDir::new().unwrap();
    let plan = FetchPlan::for_lib(&l, cache.path());

    assert_eq!(plan.display_name, "hostops 0.1.0");
    assert_eq!(plan.expected_sha256, "cccc");
    assert_eq!(
        plan.url,
        "https://github.com/developerinlondon/assay/releases/download/v0.1.0/assay-lib-hostops-0.1.0.tar.gz"
    );
    assert_eq!(
        plan.cache_path,
        cache.path().join("assay-lib-hostops-0.1.0.tar.gz")
    );
}

#[test]
fn lib_plan_uses_source_override() {
    let l = lib(
        "hostops",
        "0.1.0",
        "cccc",
        Some("https://mirror.example/h.tar.gz"),
    );
    let cache = TempDir::new().unwrap();
    let plan = FetchPlan::for_lib(&l, cache.path());
    assert_eq!(plan.url, "https://mirror.example/h.tar.gz");
}

// --- fetch() flows ---------------------------------------------------

#[tokio::test]
async fn happy_path_downloads_verifies_and_caches() {
    let body = b"library tarball bytes";
    let server = mock_serving(body, "/lib.tar.gz").await;

    let cache = TempDir::new().unwrap();
    let l = lib(
        "hostops",
        "0.1.0",
        &sha(body),
        Some(&format!("{}/lib.tar.gz", server.uri())),
    );
    let plan = FetchPlan::for_lib(&l, cache.path());

    fetch(&plan, &reqwest::Client::new(), false).await.unwrap();

    assert!(plan.cache_path.exists());
    assert_eq!(std::fs::read(&plan.cache_path).unwrap(), body);
}

#[tokio::test]
async fn cache_hit_skips_http() {
    let body = b"already-cached lib bytes";
    // MockServer with no mounted mock: any HTTP request fails the test
    // because wiremock returns 404 by default and we'd see a Status error.
    let server = MockServer::start().await;

    let cache = TempDir::new().unwrap();
    pre_populate(cache.path(), "assay-lib-hostops-0.1.0.tar.gz", body);

    let l = lib(
        "hostops",
        "0.1.0",
        &sha(body),
        Some(&format!("{}/lib.tar.gz", server.uri())),
    );
    let plan = FetchPlan::for_lib(&l, cache.path());

    fetch(&plan, &reqwest::Client::new(), false).await.unwrap();
    // file unchanged
    assert_eq!(std::fs::read(&plan.cache_path).unwrap(), body);
    // wiremock: zero requests received
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn bad_cache_is_replaced_when_online() {
    let good_body = b"good content";
    let bad_body = b"stale wrong content";
    let server = mock_serving(good_body, "/lib.tar.gz").await;

    let cache = TempDir::new().unwrap();
    pre_populate(cache.path(), "assay-lib-hostops-0.1.0.tar.gz", bad_body);

    let l = lib(
        "hostops",
        "0.1.0",
        &sha(good_body),
        Some(&format!("{}/lib.tar.gz", server.uri())),
    );
    let plan = FetchPlan::for_lib(&l, cache.path());

    fetch(&plan, &reqwest::Client::new(), false).await.unwrap();

    // bad cache replaced with good bytes
    assert_eq!(std::fs::read(&plan.cache_path).unwrap(), good_body);
}

#[tokio::test]
async fn server_body_with_wrong_sha256_aborts_and_leaves_cache_empty() {
    let body = b"server gives wrong bytes";
    let server = mock_serving(body, "/lib.tar.gz").await;

    let cache = TempDir::new().unwrap();
    let l = lib(
        "hostops",
        "0.1.0",
        // expected sha doesn't match what server returns
        &sha(b"different bytes that we hash"),
        Some(&format!("{}/lib.tar.gz", server.uri())),
    );
    let plan = FetchPlan::for_lib(&l, cache.path());

    let err = fetch(&plan, &reqwest::Client::new(), false).await.unwrap_err();
    assert!(matches!(err, FetchError::Sha256Mismatch { .. }));
    // cache file not written
    assert!(!plan.cache_path.exists());
    // tmp file cleaned up too
    let tmp = cache.path().join("assay-lib-hostops-0.1.0.tar.gz.tmp");
    assert!(!tmp.exists());
}

#[tokio::test]
async fn http_404_aborts_with_status_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/missing.tar.gz"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let cache = TempDir::new().unwrap();
    let l = lib(
        "hostops",
        "0.1.0",
        "0000",
        Some(&format!("{}/missing.tar.gz", server.uri())),
    );
    let plan = FetchPlan::for_lib(&l, cache.path());

    let err = fetch(&plan, &reqwest::Client::new(), false).await.unwrap_err();
    match err {
        FetchError::Status { status, .. } => assert_eq!(status, 404),
        other => panic!("expected Status(404), got {other:?}"),
    }
    assert!(!plan.cache_path.exists());
}

// --- --offline behaviour ---------------------------------------------

#[tokio::test]
async fn offline_with_cache_hit_succeeds_without_http() {
    let body = b"cached lib bytes";
    let server = MockServer::start().await; // no mocks mounted

    let cache = TempDir::new().unwrap();
    pre_populate(cache.path(), "assay-lib-hostops-0.1.0.tar.gz", body);

    let l = lib(
        "hostops",
        "0.1.0",
        &sha(body),
        Some(&format!("{}/never-called.tar.gz", server.uri())),
    );
    let plan = FetchPlan::for_lib(&l, cache.path());

    fetch(&plan, &reqwest::Client::new(), true).await.unwrap();
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn offline_with_cache_miss_errors_with_offline_missing() {
    let cache = TempDir::new().unwrap();
    let l = lib(
        "hostops",
        "0.1.0",
        "abcdef",
        Some("https://example.invalid/never-reached.tar.gz"),
    );
    let plan = FetchPlan::for_lib(&l, cache.path());

    let err = fetch(&plan, &reqwest::Client::new(), true).await.unwrap_err();
    assert!(matches!(err, FetchError::OfflineMissing { .. }));
    let msg = err.to_string();
    assert!(msg.contains("hostops"));
    assert!(msg.contains("--offline"));
}

#[tokio::test]
async fn offline_with_bad_cache_drops_it_and_errors() {
    let cache = TempDir::new().unwrap();
    let cache_file = pre_populate(
        cache.path(),
        "assay-lib-hostops-0.1.0.tar.gz",
        b"stale wrong content",
    );

    let l = lib(
        "hostops",
        "0.1.0",
        &sha(b"different correct content"),
        Some("https://example.invalid/never-reached.tar.gz"),
    );
    let plan = FetchPlan::for_lib(&l, cache.path());

    let err = fetch(&plan, &reqwest::Client::new(), true).await.unwrap_err();
    assert!(matches!(err, FetchError::OfflineMissing { .. }));
    // bad cache file dropped during the probe
    assert!(!cache_file.exists());
}
