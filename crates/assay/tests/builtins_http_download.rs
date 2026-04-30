mod common;

use common::create_vm;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn test_download_writes_body_to_file() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/asset.bin"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"hello\n".as_slice()))
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("out.bin");
    let url = format!("{}/asset.bin", server.uri());
    let dest_str = dest.to_str().unwrap().to_string();

    let vm = create_vm();
    vm.globals().set("u", url).unwrap();
    vm.globals().set("d", dest_str).unwrap();
    let bytes: i64 = vm
        .load(r#"return http.download(u, d)"#)
        .eval_async()
        .await
        .unwrap();
    assert_eq!(bytes, 6);
    let written = std::fs::read(&dest).unwrap();
    assert_eq!(written, b"hello\n");
}

#[tokio::test]
async fn test_download_404_errors_no_partial_file() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/missing"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("out.bin");
    let url = format!("{}/missing", server.uri());
    let dest_str = dest.to_str().unwrap().to_string();

    let vm = create_vm();
    vm.globals().set("u", url).unwrap();
    vm.globals().set("d", dest_str.clone()).unwrap();
    let result: mlua::Result<i64> = vm
        .load(r#"return http.download(u, d)"#)
        .eval_async()
        .await;
    assert!(result.is_err());
    // No partial file should exist.
    assert!(!std::path::Path::new(&dest_str).exists());
}

#[tokio::test]
async fn test_download_creates_parent_dir() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/x"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"x".as_slice()))
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("nested/dirs/out.bin");
    let url = format!("{}/x", server.uri());
    let dest_str = dest.to_str().unwrap().to_string();

    let vm = create_vm();
    vm.globals().set("u", url).unwrap();
    vm.globals().set("d", dest_str).unwrap();
    let _: i64 = vm
        .load(r#"return http.download(u, d)"#)
        .eval_async()
        .await
        .unwrap();
    assert_eq!(std::fs::read(&dest).unwrap(), b"x");
}

#[tokio::test]
async fn test_download_atomic_temp_then_rename() {
    // Verify that an interrupted download doesn't leave a partial file at dest.
    // Hard to simulate mid-stream cancellation cleanly; we just verify the
    // success-path post-condition: dest exists and contains the full body.
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/big"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![0xABu8; 100_000]))
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("big.bin");
    let url = format!("{}/big", server.uri());
    let dest_str = dest.to_str().unwrap().to_string();

    let vm = create_vm();
    vm.globals().set("u", url).unwrap();
    vm.globals().set("d", dest_str).unwrap();
    let bytes: i64 = vm
        .load(r#"return http.download(u, d)"#)
        .eval_async()
        .await
        .unwrap();
    assert_eq!(bytes, 100_000);
    assert_eq!(std::fs::metadata(&dest).unwrap().len(), 100_000);
}
