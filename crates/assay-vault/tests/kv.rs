//! KV v2 integration tests for the SQLite store + service.
//!
//! Exercises the full Phase-1 lifecycle: PUT (multi-version), GET
//! (specific + latest), LIST, soft-delete, undelete, hard-destroy.
//! Crypto round-trips through the real KekHandle and AES-GCM-SIV path.

#![cfg(all(feature = "backend-sqlite", feature = "vault-kv"))]

use assay_vault::crypto::seal_state::SealState;
use assay_vault::crypto::sealing::SealingMethod;
use assay_vault::store::sqlite::SqliteKvStore;
use assay_vault::{KekHandle, KvService};
use serde_json::json;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Executor, SqlitePool};
use std::str::FromStr;

async fn boot_pool() -> SqlitePool {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let suffix = format!(
        "{}_{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    );
    let v = format!("file:assay_vault_kv_{suffix}?mode=memory&cache=shared");
    let e = format!("file:assay_vault_kv_e_{suffix}?mode=memory&cache=shared");

    let opts = SqliteConnectOptions::from_str("sqlite::memory:")
        .unwrap()
        .create_if_missing(true);

    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .after_connect(move |conn, _| {
            let v = v.clone();
            let e = e.clone();
            Box::pin(async move {
                conn.execute(format!("ATTACH DATABASE '{e}' AS engine").as_str())
                    .await?;
                conn.execute(format!("ATTACH DATABASE '{v}' AS vault").as_str())
                    .await?;
                Ok(())
            })
        })
        .connect_with(opts)
        .await
        .unwrap();
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS engine.migrations (
            module  TEXT NOT NULL,
            version INTEGER NOT NULL,
            PRIMARY KEY (module, version)
        )",
    )
    .execute(&pool)
    .await
    .unwrap();
    assay_vault::schema::migrate_sqlite(&pool).await.unwrap();
    pool
}

fn service(pool: SqlitePool) -> KvService<SqliteKvStore> {
    let kek = KekHandle::generate_ephemeral();
    let seal_state = SealState::unsealed(SealingMethod::Plaintext, kek.kid().to_string(), kek);
    KvService::new(SqliteKvStore::new(pool), seal_state)
}

#[tokio::test]
async fn put_and_get_round_trip() {
    let svc = service(boot_pool().await);
    let v = svc
        .put("api/stripe", b"sk_live_xxx", json!({}))
        .await
        .unwrap();
    assert_eq!(v, 1);
    let r = svc.get("api/stripe", None).await.unwrap();
    assert_eq!(r.plaintext, b"sk_live_xxx");
    assert_eq!(r.version, 1);
    assert!(r.deleted_at.is_none());
}

#[tokio::test]
async fn versions_increment_per_put() {
    let svc = service(boot_pool().await);
    assert_eq!(svc.put("k", b"v1", json!({})).await.unwrap(), 1);
    assert_eq!(svc.put("k", b"v2", json!({})).await.unwrap(), 2);
    assert_eq!(svc.put("k", b"v3", json!({})).await.unwrap(), 3);
    let r1 = svc.get("k", Some(1)).await.unwrap();
    assert_eq!(r1.plaintext, b"v1");
    let r3 = svc.get("k", Some(3)).await.unwrap();
    assert_eq!(r3.plaintext, b"v3");
    let latest = svc.get("k", None).await.unwrap();
    assert_eq!(latest.plaintext, b"v3");
}

#[tokio::test]
async fn missing_path_is_not_found() {
    let svc = service(boot_pool().await);
    assert!(matches!(
        svc.get("nope", None).await,
        Err(assay_vault::VaultError::NotFound)
    ));
}

#[tokio::test]
async fn list_filters_by_prefix() {
    let svc = service(boot_pool().await);
    svc.put("api/stripe", b"a", json!({})).await.unwrap();
    svc.put("api/twilio", b"b", json!({})).await.unwrap();
    svc.put("infra/postgres", b"c", json!({})).await.unwrap();
    let api = svc.list("api/").await.unwrap();
    assert_eq!(api.len(), 2);
    assert!(api.iter().any(|m| m.path == "api/stripe"));
    assert!(api.iter().any(|m| m.path == "api/twilio"));
    let all = svc.list("").await.unwrap();
    assert_eq!(all.len(), 3);
}

#[tokio::test]
async fn soft_delete_then_undelete() {
    let svc = service(boot_pool().await);
    svc.put("k", b"v1", json!({})).await.unwrap();
    svc.put("k", b"v2", json!({})).await.unwrap();

    svc.soft_delete("k", 2).await.unwrap();
    // Latest live read returns v1 — the soft-deleted v2 is no longer
    // the "active" head. Strict semantics: the store still returns the
    // most-recent non-destroyed row; service-level filtering is the
    // caller's job. Phase-1 test: confirm `deleted_at` round-trips on
    // the explicit version read.
    let v2 = svc.get("k", Some(2)).await.unwrap();
    assert!(
        v2.deleted_at.is_some(),
        "deleted_at must be set after soft_delete"
    );

    svc.undelete("k", 2).await.unwrap();
    let v2_back = svc.get("k", Some(2)).await.unwrap();
    assert!(
        v2_back.deleted_at.is_none(),
        "undelete must clear deleted_at"
    );
}

#[tokio::test]
async fn destroy_makes_get_404() {
    let svc = service(boot_pool().await);
    svc.put("secret", b"top", json!({})).await.unwrap();
    svc.destroy("secret", 1).await.unwrap();
    // Specific version: gone (the store returns row but service maps
    // destroyed → NotFound).
    assert!(matches!(
        svc.get("secret", Some(1)).await,
        Err(assay_vault::VaultError::NotFound)
    ));
    // Latest: also gone (get_latest_row filters destroyed).
    assert!(matches!(
        svc.get("secret", None).await,
        Err(assay_vault::VaultError::NotFound)
    ));
}

#[tokio::test]
async fn destroy_is_irreversible() {
    let svc = service(boot_pool().await);
    svc.put("k", b"v", json!({})).await.unwrap();
    svc.destroy("k", 1).await.unwrap();
    // Undelete refuses to revive a destroyed row.
    assert!(matches!(
        svc.undelete("k", 1).await,
        Err(assay_vault::VaultError::NotFound)
    ));
}

#[tokio::test]
async fn destroy_zeros_ciphertext_on_disk() {
    let pool = boot_pool().await;
    let svc = service(pool.clone());
    svc.put("paranoid", b"deeply secret", json!({}))
        .await
        .unwrap();
    svc.destroy("paranoid", 1).await.unwrap();
    // Reach into the raw row — even with the row preserved for audit,
    // ciphertext + wrapped_dek must be empty.
    let row: (Vec<u8>, Vec<u8>, i64) = sqlx::query_as(
        "SELECT ciphertext, wrapped_dek, destroyed FROM vault.kv WHERE path = ? AND version = 1",
    )
    .bind("paranoid")
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(row.0.is_empty(), "ciphertext must be wiped after destroy");
    assert!(row.1.is_empty(), "wrapped_dek must be wiped after destroy");
    assert_eq!(row.2, 1);
}

#[tokio::test]
async fn custom_md_merges() {
    let svc = service(boot_pool().await);
    svc.put("k", b"v1", json!({"owner": "alice", "rotate": "monthly"}))
        .await
        .unwrap();
    svc.put("k", b"v2", json!({"owner": "bob"})).await.unwrap();
    let m = svc.read_meta("k").await.unwrap();
    assert_eq!(m.custom_md["owner"], "bob");
    assert_eq!(m.custom_md["rotate"], "monthly");
}

#[tokio::test]
async fn aad_binds_path_so_relocated_ciphertext_fails() {
    // Cross-row ciphertext substitution attack: we manually move the
    // ciphertext bytes from path A to path B and confirm decryption
    // fails. Phase-1 design says path is part of AAD — the AEAD tag
    // check must reject this.
    let pool = boot_pool().await;
    let svc = service(pool.clone());
    svc.put("a", b"the secret", json!({})).await.unwrap();

    // Manually plant a row at path "b" with the ciphertext from "a".
    let (ct, n, wd, kk): (Vec<u8>, Vec<u8>, Vec<u8>, String) = sqlx::query_as(
        "SELECT ciphertext, nonce, wrapped_dek, kek_kid FROM vault.kv WHERE path = 'a' AND version = 1",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO vault.kv_meta (path, latest_version, custom_md, created_at, updated_at)
         VALUES ('b', 1, '{}', 0.0, 0.0)",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO vault.kv (path, version, ciphertext, nonce, wrapped_dek, kek_kid, created_at)
         VALUES ('b', 1, ?, ?, ?, ?, 0.0)",
    )
    .bind(ct)
    .bind(n)
    .bind(wd)
    .bind(kk)
    .execute(&pool)
    .await
    .unwrap();

    let res = svc.get("b", None).await;
    assert!(
        matches!(res, Err(assay_vault::VaultError::Crypto(_))),
        "AAD bind must reject ciphertext relocated to a different path; got {res:?}"
    );
}
