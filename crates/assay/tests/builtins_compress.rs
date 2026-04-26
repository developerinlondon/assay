mod common;

use std::io::Write;

use common::{eval_lua, run_lua};

fn gzip_bytes(input: &[u8]) -> Vec<u8> {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(input).unwrap();
    encoder.finish().unwrap()
}

fn xz_bytes(input: &[u8]) -> Vec<u8> {
    let mut encoder = xz2::write::XzEncoder::new(Vec::new(), 6);
    encoder.write_all(input).unwrap();
    encoder.finish().unwrap()
}

fn zstd_bytes(input: &[u8]) -> Vec<u8> {
    zstd::stream::encode_all(input, 0).unwrap()
}

fn lua_string_literal(bytes: &[u8]) -> String {
    let mut out = String::from("\"");
    for b in bytes {
        out.push_str(&format!("\\{}", b));
    }
    out.push('"');
    out
}

#[tokio::test]
async fn test_gunzip_round_trip() {
    let payload = b"hello compress world\nline 2\n";
    let compressed = gzip_bytes(payload);
    let lit = lua_string_literal(&compressed);
    let script = format!(
        r#"
        local out = compress.gunzip({lit})
        assert.eq(out, "hello compress world\nline 2\n")
        "#
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_unxz_round_trip() {
    let payload = b"xz compressed payload bytes 12345";
    let compressed = xz_bytes(payload);
    let lit = lua_string_literal(&compressed);
    let script = format!(
        r#"
        local out = compress.unxz({lit})
        assert.eq(out, "xz compressed payload bytes 12345")
        "#
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_unzstd_round_trip() {
    let payload = b"zstd compressed payload bytes ABCDE";
    let compressed = zstd_bytes(payload);
    let lit = lua_string_literal(&compressed);
    let script = format!(
        r#"
        local out = compress.unzstd({lit})
        assert.eq(out, "zstd compressed payload bytes ABCDE")
        "#
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_gunzip_garbage_errors() {
    let script = r#"compress.gunzip("not actually gzip data")"#;
    let err = run_lua(script).await.unwrap_err();
    assert!(
        err.to_string().contains("compress.gunzip"),
        "expected compress.gunzip error, got: {err}"
    );
}

#[tokio::test]
async fn test_unxz_garbage_errors() {
    let script = r#"compress.unxz("not actually xz data")"#;
    let err = run_lua(script).await.unwrap_err();
    assert!(
        err.to_string().contains("compress.unxz"),
        "expected compress.unxz error, got: {err}"
    );
}

#[tokio::test]
async fn test_unzstd_garbage_errors() {
    let script = r#"compress.unzstd("not actually zstd data")"#;
    let err = run_lua(script).await.unwrap_err();
    assert!(
        err.to_string().contains("compress.unzstd"),
        "expected compress.unzstd error, got: {err}"
    );
}

#[tokio::test]
async fn test_gunzip_returns_bytes_with_zero() {
    let payload: Vec<u8> = vec![0x00, 0x01, 0x02, 0xFF, 0x00, 0xAB];
    let compressed = gzip_bytes(&payload);
    let lit = lua_string_literal(&compressed);
    let script = format!(
        r#"
        local out = compress.gunzip({lit})
        return #out
        "#
    );
    let len: i64 = eval_lua(&script).await;
    assert_eq!(len, payload.len() as i64);
}
