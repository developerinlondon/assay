//! Sealing the vault's master KEK with a key from the environment.
//!
//! Without a seal key the KEK is stored as raw bytes, so a database dump
//! is a plaintext copy of every secret. These drive a real engine and
//! read `vault.kek_metadata` off the disk it wrote.

mod common;

use common::{ADMIN_KEY, EngineProcess, client};
use serde_json::json;
use std::path::Path;

const SECRET: &str = "sealed-secret-value";
const SEAL_KEY_A: &str = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";
const SEAL_KEY_B: &str = "/v79/Pv6+fj39vX08/Lx8O/u7ezr6uno5+bl5OPi4eA=";

fn backend(data_dir: &Path) -> String {
    format!(
        "[backend]\ntype = \"sqlite\"\ndata_dir = \"{}\"",
        data_dir.display()
    )
}

/// Boot an engine with the given seal key, write a secret, stop.
async fn write_secret(dir: &Path, data_dir: &Path, tag: &str, seal: Option<&str>) {
    let mut engine = EngineProcess::spawn_with_env(dir, tag, &backend(data_dir), seal);
    let http = client();
    engine.wait_ready(&http).await.expect("engine ready");
    let r = http
        .put(engine.url("/api/v1/vault/kv/seal/secret"))
        .header("Authorization", format!("Bearer {ADMIN_KEY}"))
        .json(&json!({"data": SECRET}))
        .send()
        .await
        .expect("write secret");
    assert_eq!(r.status(), 201, "write secret: {}", engine.log());
    engine.stop();
}

/// Read the one `vault.kek_metadata` row straight off the SQLite file.
async fn kek_row(data_dir: &Path) -> (String, Vec<u8>) {
    let url = format!("sqlite://{}/vault.db", data_dir.display());
    let pool = sqlx::SqlitePool::connect(&url).await.expect("open vault.db");
    let row: (String, Vec<u8>) =
        sqlx::query_as("SELECT sealing_method, sealed_blob FROM kek_metadata LIMIT 1")
            .fetch_one(&pool)
            .await
            .expect("read kek_metadata");
    pool.close().await;
    row
}

/// Boot with a seal key and read the secret back.
async fn read_secret(dir: &Path, data_dir: &Path, tag: &str, seal: Option<&str>) -> String {
    let mut engine = EngineProcess::spawn_with_env(dir, tag, &backend(data_dir), seal);
    let http = client();
    engine.wait_ready(&http).await.expect("engine ready");
    let r = http
        .get(engine.url("/api/v1/vault/kv/seal/secret"))
        .header("Authorization", format!("Bearer {ADMIN_KEY}"))
        .send()
        .await
        .expect("read secret");
    assert_eq!(r.status(), 200, "read secret: {}", engine.log());
    let body: serde_json::Value = r.json().await.unwrap();
    let value = body["data"].as_str().expect("data is a string").to_string();
    engine.stop();
    value
}

/// With no seal key the KEK stays raw in the store, as it always has.
#[tokio::test(flavor = "multi_thread")]
async fn without_a_seal_key_the_kek_is_stored_in_the_clear() {
    let dir = tempfile::tempdir().expect("tempdir");
    let data = dir.path().join("data");
    write_secret(dir.path(), &data, "plain", None).await;

    let (method, blob) = kek_row(&data).await;
    assert_eq!(method, "plaintext");
    assert_eq!(blob.len(), 32, "a plaintext KEK is the raw key");
}

/// With a seal key the stored blob is ciphertext, and the same key
/// opens it again on the next boot.
#[tokio::test(flavor = "multi_thread")]
async fn a_seal_key_encrypts_the_kek_and_still_opens_it() {
    let dir = tempfile::tempdir().expect("tempdir");
    let data = dir.path().join("data");
    write_secret(dir.path(), &data, "sealed", Some(SEAL_KEY_A)).await;

    let (method, blob) = kek_row(&data).await;
    assert_eq!(method, "env-aes-gcm");
    assert_eq!(
        blob.len(),
        61,
        "version byte, 12-byte nonce, 32 bytes of key and a 16-byte tag"
    );

    let value = read_secret(dir.path(), &data, "sealed-2", Some(SEAL_KEY_A)).await;
    assert_eq!(value, SECRET, "the secret must survive a sealed restart");
}

/// A store written in the clear is sealed on the first boot that has a
/// key, and its existing secrets still decrypt.
#[tokio::test(flavor = "multi_thread")]
async fn an_existing_plaintext_store_is_resealed_and_keeps_its_secrets() {
    let dir = tempfile::tempdir().expect("tempdir");
    let data = dir.path().join("data");
    write_secret(dir.path(), &data, "before", None).await;
    let (method, _) = kek_row(&data).await;
    assert_eq!(method, "plaintext", "precondition: stored in the clear");

    let value = read_secret(dir.path(), &data, "reseal", Some(SEAL_KEY_A)).await;
    assert_eq!(value, SECRET, "secrets written before sealing must still read");

    let (method, blob) = kek_row(&data).await;
    assert_eq!(method, "env-aes-gcm", "the KEK should have been re-sealed");
    assert_eq!(blob.len(), 61);

    // Re-running with the same key changes nothing and still works.
    let again = read_secret(dir.path(), &data, "reseal-2", Some(SEAL_KEY_A)).await;
    assert_eq!(again, SECRET);
    assert_eq!(kek_row(&data).await.0, "env-aes-gcm");
}

/// The wrong key, or none at all, must fail loudly rather than mint a
/// second KEK and orphan every secret the first one wraps.
#[tokio::test(flavor = "multi_thread")]
async fn a_sealed_store_refuses_to_boot_without_the_right_key() {
    let dir = tempfile::tempdir().expect("tempdir");
    let data = dir.path().join("data");
    write_secret(dir.path(), &data, "locked", Some(SEAL_KEY_A)).await;

    for (tag, key, expected) in [
        ("no-key", None, "is not set"),
        ("wrong-key", Some(SEAL_KEY_B), "does not decrypt"),
    ] {
        let mut engine = EngineProcess::spawn_with_env(dir.path(), tag, &backend(&data), key);
        let err = engine
            .wait_ready(&client())
            .await
            .expect_err("the engine must refuse to start");
        assert!(
            err.contains(expected),
            "expected {expected:?} in the failure, got: {err}"
        );
    }
}
