mod common;

use common::create_vm;

/// Build a gzip-compressed tar archive in memory with one member.
fn build_tar_gz(member_name: &str, member_bytes: &[u8]) -> Vec<u8> {
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    {
        let mut builder = tar::Builder::new(&mut gz);
        let mut header = tar::Header::new_gnu();
        header.set_path(member_name).unwrap();
        header.set_size(member_bytes.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        builder.append(&header, member_bytes).unwrap();
        builder.finish().unwrap();
    }
    gz.finish().unwrap()
}

/// Build a plain (uncompressed) tar archive in memory.
fn build_tar(member_name: &str, member_bytes: &[u8]) -> Vec<u8> {
    let mut buf = Vec::new();
    {
        let mut builder = tar::Builder::new(&mut buf);
        let mut header = tar::Header::new_gnu();
        header.set_path(member_name).unwrap();
        header.set_size(member_bytes.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        builder.append(&header, member_bytes).unwrap();
        builder.finish().unwrap();
    }
    buf
}

#[tokio::test]
async fn test_untar_extracts_named_member_from_tar_gz() {
    let dir = tempfile::tempdir().unwrap();
    let archive = dir.path().join("x.tar.gz");
    let dest = dir.path().join("extracted.bin");
    let bytes = build_tar_gz("rustic", b"FAKE-RUSTIC-BINARY");
    std::fs::write(&archive, &bytes).unwrap();

    let archive_str = archive.to_str().unwrap().to_string();
    let dest_str = dest.to_str().unwrap().to_string();

    let vm = create_vm();
    vm.globals().set("a", archive_str).unwrap();
    vm.globals().set("d", dest_str).unwrap();
    let n: i64 = vm
        .load(r#"return compress.untar(a, d, { member = "rustic" })"#)
        .eval_async()
        .await
        .unwrap();
    assert_eq!(n, 18);
    let got = std::fs::read(&dest).unwrap();
    assert_eq!(got, b"FAKE-RUSTIC-BINARY");
}

#[tokio::test]
async fn test_untar_plain_tar_no_compression() {
    let dir = tempfile::tempdir().unwrap();
    let archive = dir.path().join("x.tar");
    let dest = dir.path().join("extracted.bin");
    std::fs::write(&archive, build_tar("hello", b"hi")).unwrap();

    let vm = create_vm();
    vm.globals().set("a", archive.to_str().unwrap().to_string()).unwrap();
    vm.globals().set("d", dest.to_str().unwrap().to_string()).unwrap();
    let n: i64 = vm
        .load(r#"return compress.untar(a, d, { member = "hello" })"#)
        .eval_async()
        .await
        .unwrap();
    assert_eq!(n, 2);
    assert_eq!(std::fs::read(&dest).unwrap(), b"hi");
}

#[tokio::test]
async fn test_untar_member_not_found_errors() {
    let dir = tempfile::tempdir().unwrap();
    let archive = dir.path().join("x.tar.gz");
    std::fs::write(&archive, build_tar_gz("alpha", b"a")).unwrap();
    let dest = dir.path().join("out");

    let vm = create_vm();
    vm.globals().set("a", archive.to_str().unwrap().to_string()).unwrap();
    vm.globals().set("d", dest.to_str().unwrap().to_string()).unwrap();
    let result: mlua::Result<i64> = vm
        .load(r#"return compress.untar(a, d, { member = "beta" })"#)
        .eval_async()
        .await;
    assert!(result.is_err());
    let msg = format!("{:?}", result.unwrap_err());
    assert!(msg.contains("beta"), "error should name missing member: {msg}");
}

#[tokio::test]
async fn test_untar_nested_member_path() {
    // Mimics tailscale-style asset: tailscale_1.78.1_amd64/tailscale
    let dir = tempfile::tempdir().unwrap();
    let archive = dir.path().join("x.tar.gz");
    let dest = dir.path().join("extracted");
    std::fs::write(
        &archive,
        build_tar_gz("tailscale_1.78.1_amd64/tailscale", b"NESTED"),
    )
    .unwrap();

    let vm = create_vm();
    vm.globals().set("a", archive.to_str().unwrap().to_string()).unwrap();
    vm.globals().set("d", dest.to_str().unwrap().to_string()).unwrap();
    let n: i64 = vm
        .load(r#"return compress.untar(a, d, { member = "tailscale_1.78.1_amd64/tailscale" })"#)
        .eval_async()
        .await
        .unwrap();
    assert_eq!(n, 6);
    assert_eq!(std::fs::read(&dest).unwrap(), b"NESTED");
}
