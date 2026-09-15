//! Sealing the vault's master KEK, and what boot does without material.
//!
//! Without unseal material the KEK would be stored as raw bytes, so a
//! database dump would be a plaintext copy of every secret. Boot refuses
//! rather than do that. These drive a real engine and read
//! `vault.kek_metadata` off the disk it wrote.
//!
//! The engine process is the right level for this: the decision is split
//! across `[vault.sealing]` parsing, the policy that resolves it, and
//! the loader that acts on it, and only a real boot exercises all three
//! against a store that persists between runs.

mod common;

use common::{ADMIN_KEY, DEV_SEALING, EngineProcess, client};
use serde_json::json;
use std::path::Path;

const SECRET: &str = "sealed-secret-value";
const SEAL_KEY_A: &str = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";
const SEAL_KEY_B: &str = "/v79/Pv6+fj39vX08/Lx8O/u7ezr6uno5+bl5OPi4eA=";
const PASSPHRASE: &str = "correct horse battery staple correct horse";
const SALT: &str = "an-example-public-salt";

/// Permit the one-way re-seal of a store that holds a plaintext key.
const ALLOW_MIGRATION: &str = "[vault.sealing]\nallow_plaintext_migration = true\n";

fn backend(data_dir: &Path) -> String {
    format!(
        "[backend]\ntype = \"sqlite\"\ndata_dir = \"{}\"",
        data_dir.display()
    )
}

/// Boot an engine, write a secret, stop.
async fn write_secret(dir: &Path, data_dir: &Path, tag: &str, sealing: &str, seal: Option<&str>) {
    let mut engine = EngineProcess::spawn_with_vault(dir, tag, &backend(data_dir), sealing, seal);
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

/// Boot and read the secret back.
async fn read_secret(
    dir: &Path,
    data_dir: &Path,
    tag: &str,
    sealing: &str,
    seal: Option<&str>,
) -> String {
    let mut engine = EngineProcess::spawn_with_vault(dir, tag, &backend(data_dir), sealing, seal);
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

/// Boot and expect the engine to refuse, returning its failure output.
///
/// Every refusal here happens before the engine binds anything, so the
/// verdict rests on this child's own exit status and never on a socket.
/// Asserting that once, here, keeps each caller free to check only the
/// message it cares about.
async fn boot_failure(
    dir: &Path,
    data_dir: &Path,
    tag: &str,
    sealing: &str,
    seal: Option<&str>,
) -> String {
    let mut engine = EngineProcess::spawn_with_vault(dir, tag, &backend(data_dir), sealing, seal);
    let err = engine
        .wait_ready(&client())
        .await
        .expect_err("the engine must refuse to start");
    // The engine has to die before it ever binds, so readiness here
    // rests on this child's own exit status and never touches a socket
    // — no sibling engine can answer in its place.
    assert!(
        err.starts_with("exited "),
        "the engine should have exited, not stalled or served: {err}"
    );
    err
}

/// Read the one `vault.kek_metadata` row straight off the SQLite file.
async fn kek_row(data_dir: &Path) -> (String, Vec<u8>) {
    let url = format!("sqlite://{}/vault.db", data_dir.display());
    let pool = sqlx::SqlitePool::connect(&url)
        .await
        .expect("open vault.db");
    let row: (String, Vec<u8>) =
        sqlx::query_as("SELECT sealing_method, sealed_blob FROM kek_metadata LIMIT 1")
            .fetch_one(&pool)
            .await
            .expect("read kek_metadata");
    pool.close().await;
    row
}

async fn kek_row_count(data_dir: &Path) -> i64 {
    let url = format!("sqlite://{}/vault.db", data_dir.display());
    let pool = sqlx::SqlitePool::connect(&url)
        .await
        .expect("open vault.db");
    let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM kek_metadata")
        .fetch_one(&pool)
        .await
        .expect("count kek_metadata");
    pool.close().await;
    row.0
}

/// Write a seal key to a file only its owner can read.
fn key_file(dir: &Path, name: &str, body: &str, mode: u32) -> std::path::PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, body).expect("write seal key file");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode))
            .expect("chmod seal key file");
    }
    let _ = mode;
    path
}

/// The gap this all exists to close: a deployment that never configured
/// unseal material used to boot happily with its master key in the
/// clear, and say so in one line of log nobody reads.
#[tokio::test(flavor = "multi_thread")]
async fn without_unseal_material_the_engine_refuses_to_start() {
    let dir = tempfile::tempdir().expect("tempdir");
    let data = dir.path().join("data");

    let err = boot_failure(dir.path(), &data, "no-material", "", None).await;
    assert!(
        err.contains("ASSAY_VAULT_SEAL_KEY"),
        "the failure must name what to set: {err}"
    );
    assert!(
        err.contains("allow_plaintext_kek"),
        "and the way past it for local development: {err}"
    );
}

/// The escape hatch is config-only and loud, and it leaves the key in
/// the clear exactly as before — which is the point of it being loud.
#[tokio::test(flavor = "multi_thread")]
async fn the_escape_hatch_boots_with_the_kek_in_the_clear() {
    let dir = tempfile::tempdir().expect("tempdir");
    let data = dir.path().join("data");
    write_secret(dir.path(), &data, "plain", DEV_SEALING, None).await;

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
    write_secret(dir.path(), &data, "sealed", "", Some(SEAL_KEY_A)).await;

    let (method, blob) = kek_row(&data).await;
    assert_eq!(method, "env-aes-gcm");
    assert_eq!(
        blob.len(),
        61,
        "version byte, 12-byte nonce, 32 bytes of key and a 16-byte tag"
    );

    let value = read_secret(dir.path(), &data, "sealed-2", "", Some(SEAL_KEY_A)).await;
    assert_eq!(value, SECRET, "the secret must survive a sealed restart");
}

/// A key from a file is the same key as the one from the variable, so an
/// operator can move a secret out of the process environment — where it
/// is readable from `/proc/<pid>/environ` and carried into core dumps —
/// without re-sealing the store.
#[tokio::test(flavor = "multi_thread")]
async fn a_seal_key_from_a_file_opens_a_store_sealed_from_the_variable() {
    let dir = tempfile::tempdir().expect("tempdir");
    let data = dir.path().join("data");
    write_secret(dir.path(), &data, "by-var", "", Some(SEAL_KEY_A)).await;

    let path = key_file(dir.path(), "seal.key", &format!("{SEAL_KEY_A}\n"), 0o600);
    let sealing = format!(
        "[vault.sealing]\nsource = \"file\"\npath = \"{}\"\n",
        path.display()
    );

    let value = read_secret(dir.path(), &data, "by-file", &sealing, None).await;
    assert_eq!(value, SECRET);
    assert_eq!(kek_row(&data).await.0, "env-aes-gcm");
}

/// A key file anyone on the box can read is not a secret.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn a_world_readable_seal_key_file_is_refused() {
    let dir = tempfile::tempdir().expect("tempdir");
    let data = dir.path().join("data");
    let path = key_file(dir.path(), "loose.key", SEAL_KEY_A, 0o644);
    let sealing = format!(
        "[vault.sealing]\nsource = \"file\"\npath = \"{}\"\n",
        path.display()
    );

    let err = boot_failure(dir.path(), &data, "loose-file", &sealing, None).await;
    assert!(err.contains("loose.key"), "name the file: {err}");
    assert!(err.contains("0644"), "and the mode it is: {err}");
}

/// A `path` with no `source = "file"` is a forgotten line, not a
/// request to read the environment. Booting on material the operator
/// did not configure is how a vault ends up unsealed by accident.
#[tokio::test(flavor = "multi_thread")]
async fn a_key_file_without_its_source_line_is_refused_rather_than_ignored() {
    let dir = tempfile::tempdir().expect("tempdir");
    let data = dir.path().join("data");
    let path = key_file(dir.path(), "seal.key", SEAL_KEY_A, 0o600);
    let sealing = format!(
        "[vault.sealing]\npath = \"{}\"\nallow_plaintext_kek = true\n",
        path.display()
    );

    let err = boot_failure(dir.path(), &data, "orphan-path", &sealing, None).await;
    assert!(err.contains("`path`"), "name the stray field: {err}");
    assert!(err.contains("source"), "{err}");
}

/// A passphrase goes through a memory-hard KDF rather than a plain hash,
/// and the salt is what makes it survive a restart.
#[tokio::test(flavor = "multi_thread")]
async fn a_passphrase_and_its_salt_reopen_the_store() {
    let dir = tempfile::tempdir().expect("tempdir");
    let data = dir.path().join("data");
    let sealing = format!(
        "[vault.sealing]\n\
         source = \"passphrase\"\n\
         passphrase = \"{PASSPHRASE}\"\n\
         salt = \"{SALT}\"\n"
    );

    write_secret(dir.path(), &data, "kdf", &sealing, None).await;
    assert_eq!(kek_row(&data).await.0, "env-aes-gcm");

    let value = read_secret(dir.path(), &data, "kdf-2", &sealing, None).await;
    assert_eq!(value, SECRET, "the same passphrase must reopen the store");
}

/// A random per-boot salt would lock the operator out on restart, so the
/// salt is required rather than defaulted.
#[tokio::test(flavor = "multi_thread")]
async fn a_passphrase_without_a_salt_is_refused() {
    let dir = tempfile::tempdir().expect("tempdir");
    let data = dir.path().join("data");
    let sealing =
        format!("[vault.sealing]\nsource = \"passphrase\"\npassphrase = \"{PASSPHRASE}\"\n");

    let err = boot_failure(dir.path(), &data, "no-salt", &sealing, None).await;
    assert!(err.contains("salt"), "name the missing field: {err}");
}

/// Re-sealing destroys the only plaintext copy of the key. Nobody used
/// to be asked; now the refusal names the backup to take first.
#[tokio::test(flavor = "multi_thread")]
async fn an_existing_plaintext_store_is_not_resealed_without_consent() {
    let dir = tempfile::tempdir().expect("tempdir");
    let data = dir.path().join("data");
    write_secret(dir.path(), &data, "before", DEV_SEALING, None).await;
    assert_eq!(kek_row(&data).await.0, "plaintext", "precondition");

    let err = boot_failure(dir.path(), &data, "unasked", "", Some(SEAL_KEY_A)).await;
    assert!(
        err.contains("vault.kek_metadata"),
        "name the backup to take: {err}"
    );
    assert!(err.contains("allow_plaintext_migration"), "{err}");

    assert_eq!(
        kek_row(&data).await.0,
        "plaintext",
        "the refusal must leave the store exactly as it was"
    );
}

/// With consent the store is sealed in place, and its existing secrets
/// still decrypt.
#[tokio::test(flavor = "multi_thread")]
async fn a_consented_migration_reseals_and_keeps_its_secrets() {
    let dir = tempfile::tempdir().expect("tempdir");
    let data = dir.path().join("data");
    write_secret(dir.path(), &data, "before", DEV_SEALING, None).await;

    let value = read_secret(
        dir.path(),
        &data,
        "reseal",
        ALLOW_MIGRATION,
        Some(SEAL_KEY_A),
    )
    .await;
    assert_eq!(
        value, SECRET,
        "secrets written before sealing must still read"
    );

    let (method, blob) = kek_row(&data).await;
    assert_eq!(method, "env-aes-gcm", "the KEK should have been re-sealed");
    assert_eq!(blob.len(), 61);

    // Re-running changes nothing and still works — the migration gate
    // must not turn a no-op boot into a second rewrite.
    let again = read_secret(dir.path(), &data, "reseal-2", "", Some(SEAL_KEY_A)).await;
    assert_eq!(again, SECRET);
    assert_eq!(kek_row(&data).await.0, "env-aes-gcm");
}

/// The wrong key, or none at all, must fail loudly rather than mint a
/// second KEK and orphan every secret the first one wraps.
#[tokio::test(flavor = "multi_thread")]
async fn a_sealed_store_refuses_to_boot_without_the_right_key() {
    let dir = tempfile::tempdir().expect("tempdir");
    let data = dir.path().join("data");
    write_secret(dir.path(), &data, "locked", "", Some(SEAL_KEY_A)).await;

    // With no material at all the refusal comes from the policy, before
    // the store is opened; with the wrong material it comes from the
    // store failing to open. Both must stop the boot.
    for (tag, key, expected) in [
        ("no-key", None, "no unseal material is configured"),
        ("wrong-key", Some(SEAL_KEY_B), "does not decrypt"),
    ] {
        let err = boot_failure(dir.path(), &data, tag, "", key).await;
        assert!(
            err.contains(expected),
            "expected {expected:?} in the failure, got: {err}"
        );
    }
}

/// The escape hatch gates *minting* a key in the clear. It must never
/// become a way past a store that is already sealed — that would mint a
/// second KEK and orphan every secret the first one wraps.
#[tokio::test(flavor = "multi_thread")]
async fn the_escape_hatch_does_not_open_a_sealed_store() {
    let dir = tempfile::tempdir().expect("tempdir");
    let data = dir.path().join("data");
    write_secret(dir.path(), &data, "locked", "", Some(SEAL_KEY_A)).await;

    let err = boot_failure(dir.path(), &data, "hatch", DEV_SEALING, None).await;
    assert!(err.contains("is not set"), "{err}");
    assert_eq!(
        kek_row_count(&data).await,
        1,
        "no second KEK may be minted alongside the sealed one"
    );
    assert_eq!(kek_row(&data).await.0, "env-aes-gcm");
}
