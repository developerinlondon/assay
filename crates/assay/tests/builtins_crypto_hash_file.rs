mod common;

use common::create_vm;

#[tokio::test]
async fn test_hash_file_sha256_known_content() {
    // sha256("hello world\n") == a948904f2f0f479b8f8197694b30184b0d2ed1c1cd2a1ec0fb85d299a192a447
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("hello.txt");
    std::fs::write(&path, b"hello world\n").unwrap();
    let path_str = path.to_str().unwrap().to_string();

    let vm = create_vm();
    vm.globals().set("p", path_str).unwrap();
    let hash: String = vm
        .load(r#"return crypto.hash_file(p, "sha256")"#)
        .eval_async()
        .await
        .unwrap();
    assert_eq!(
        hash,
        "a948904f2f0f479b8f8197694b30184b0d2ed1c1cd2a1ec0fb85d299a192a447"
    );
}

#[tokio::test]
async fn test_hash_file_default_algo_is_sha256() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("hello.txt");
    std::fs::write(&path, b"hello world\n").unwrap();
    let path_str = path.to_str().unwrap().to_string();

    let vm = create_vm();
    vm.globals().set("p", path_str).unwrap();
    let hash: String = vm
        .load(r#"return crypto.hash_file(p)"#)
        .eval_async()
        .await
        .unwrap();
    assert_eq!(
        hash,
        "a948904f2f0f479b8f8197694b30184b0d2ed1c1cd2a1ec0fb85d299a192a447"
    );
}

#[tokio::test]
async fn test_hash_file_missing_path_errors() {
    let vm = create_vm();
    let result: mlua::Result<String> = vm
        .load(r#"return crypto.hash_file("/no/such/file", "sha256")"#)
        .eval_async()
        .await;
    assert!(result.is_err());
    let msg = format!("{:?}", result.unwrap_err());
    assert!(
        msg.contains("hash_file"),
        "error should mention hash_file: {msg}"
    );
}

#[tokio::test]
async fn test_hash_file_sha3_256() {
    // sha3-256("hello world\n") == a8009a7a528d87778c356da3a55d964719e818666a04e4f960c9e2439e35f138
    // (Verified via: printf 'hello world\n' | openssl dgst -sha3-256)
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("hello.txt");
    std::fs::write(&path, b"hello world\n").unwrap();
    let path_str = path.to_str().unwrap().to_string();

    let vm = create_vm();
    vm.globals().set("p", path_str).unwrap();
    let hash: String = vm
        .load(r#"return crypto.hash_file(p, "sha3-256")"#)
        .eval_async()
        .await
        .unwrap();
    assert_eq!(
        hash,
        "a8009a7a528d87778c356da3a55d964719e818666a04e4f960c9e2439e35f138"
    );
}

#[tokio::test]
async fn test_hash_file_unsupported_algo_errors() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("x");
    std::fs::write(&path, b"x").unwrap();
    let path_str = path.to_str().unwrap().to_string();

    let vm = create_vm();
    vm.globals().set("p", path_str).unwrap();
    let result: mlua::Result<String> = vm
        .load(r#"return crypto.hash_file(p, "md5")"#)
        .eval_async()
        .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_hash_file_large_file() {
    // Sanity-check that hashing works on a non-trivial file (5 MiB of zero bytes).
    // sha256 verified via `dd if=/dev/zero bs=1M count=5 | sha256sum`.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("zeros");
    std::fs::write(&path, vec![0u8; 5 * 1024 * 1024]).unwrap();
    let path_str = path.to_str().unwrap().to_string();

    let vm = create_vm();
    vm.globals().set("p", path_str).unwrap();
    let hash: String = vm
        .load(r#"return crypto.hash_file(p, "sha256")"#)
        .eval_async()
        .await
        .unwrap();
    assert_eq!(
        hash,
        "c036cbb7553a909f8b8877d4461924307f27ecb66cff928eeeafd569c3887e29"
    );
}
