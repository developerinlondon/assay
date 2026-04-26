//! Transit integration tests — full encrypt / decrypt / rotate cycle
//! against the SQLite store.

#![cfg(all(feature = "backend-sqlite", feature = "vault-transit"))]

use assay_vault::store::sqlite::SqliteTransitStore;
use assay_vault::{KekHandle, TransitService};
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
    let v = format!("file:assay_vault_tr_{suffix}?mode=memory&cache=shared");
    let e = format!("file:assay_vault_tr_e_{suffix}?mode=memory&cache=shared");

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

fn service(pool: SqlitePool) -> TransitService<SqliteTransitStore> {
    TransitService::new(SqliteTransitStore::new(pool), KekHandle::generate_ephemeral())
}

#[tokio::test]
async fn create_encrypt_decrypt_round_trip() {
    let svc = service(boot_pool().await);
    svc.create_key("logs", None).await.unwrap();
    let ct = svc.encrypt("logs", b"hello world").await.unwrap();
    assert!(ct.starts_with("vault:v1:"));
    let pt = svc.decrypt("logs", &ct).await.unwrap();
    assert_eq!(pt, b"hello world");
}

#[tokio::test]
async fn duplicate_key_is_conflict() {
    let svc = service(boot_pool().await);
    svc.create_key("dup", None).await.unwrap();
    let res = svc.create_key("dup", None).await;
    assert!(matches!(res, Err(assay_vault::VaultError::Conflict(_))));
}

#[tokio::test]
async fn missing_key_is_not_found() {
    let svc = service(boot_pool().await);
    assert!(matches!(
        svc.encrypt("nope", b"x").await,
        Err(assay_vault::VaultError::NotFound)
    ));
}

#[tokio::test]
async fn rotation_preserves_old_versions_decryptability() {
    let svc = service(boot_pool().await);
    svc.create_key("rot", None).await.unwrap();
    let ct_v1 = svc.encrypt("rot", b"first").await.unwrap();
    assert!(ct_v1.starts_with("vault:v1:"));

    let v2 = svc.rotate("rot").await.unwrap();
    assert_eq!(v2, 2);

    let ct_v2 = svc.encrypt("rot", b"second").await.unwrap();
    assert!(ct_v2.starts_with("vault:v2:"));

    // Old ciphertext still decrypts after rotation — that's the whole
    // point of versioned transit keys.
    assert_eq!(svc.decrypt("rot", &ct_v1).await.unwrap(), b"first");
    assert_eq!(svc.decrypt("rot", &ct_v2).await.unwrap(), b"second");
}

#[tokio::test]
async fn aad_binds_key_name_so_swap_fails() {
    let svc = service(boot_pool().await);
    svc.create_key("alpha", None).await.unwrap();
    svc.create_key("beta", None).await.unwrap();
    let ct = svc.encrypt("alpha", b"sensitive").await.unwrap();
    // Decrypting alpha's ciphertext under beta's name fetches a
    // different DEK with a different AAD — fails.
    let res = svc.decrypt("beta", &ct).await;
    assert!(
        res.is_err(),
        "AAD must reject ciphertext-under-wrong-key-name"
    );
}

#[tokio::test]
async fn list_keys_alphabetised() {
    let svc = service(boot_pool().await);
    svc.create_key("c-zone", None).await.unwrap();
    svc.create_key("a-zone", None).await.unwrap();
    svc.create_key("b-zone", None).await.unwrap();
    let names: Vec<String> = svc.list_keys().await.unwrap().into_iter().map(|k| k.name).collect();
    assert_eq!(names, vec!["a-zone", "b-zone", "c-zone"]);
}

#[tokio::test]
async fn rotate_unknown_key_is_not_found() {
    let svc = service(boot_pool().await);
    assert!(matches!(
        svc.rotate("ghost").await,
        Err(assay_vault::VaultError::NotFound)
    ));
}

#[tokio::test]
async fn malformed_envelope_rejects_at_decrypt() {
    let svc = service(boot_pool().await);
    svc.create_key("k", None).await.unwrap();
    assert!(matches!(
        svc.decrypt("k", "not-an-envelope").await,
        Err(assay_vault::VaultError::Invalid(_))
    ));
    assert!(matches!(
        svc.decrypt("k", "vault:vX:abc").await,
        Err(assay_vault::VaultError::Invalid(_))
    ));
}

#[tokio::test]
async fn decrypt_unknown_version_is_not_found() {
    let svc = service(boot_pool().await);
    svc.create_key("k", None).await.unwrap();
    let ct = svc.encrypt("k", b"x").await.unwrap();
    // Hand-edit the version to one that doesn't exist.
    let bogus = ct.replacen("v1", "v99", 1);
    assert!(matches!(
        svc.decrypt("k", &bogus).await,
        Err(assay_vault::VaultError::NotFound)
    ));
}
