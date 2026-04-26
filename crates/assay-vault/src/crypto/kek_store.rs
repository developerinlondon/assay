//! KEK persistence — load the active KEK from `vault.kek_metadata` or
//! generate a fresh one on first boot.
//!
//! ## Phase 1 stance
//!
//! Phase 1 ships `sealing_method = 'plaintext'` only — the KEK lives in
//! `kek_metadata.sealed_blob` as raw bytes. Engine boot logs a WARN so
//! operators know vault is running unsealed and that Phase 2 is the path
//! to real sealing.
//!
//! ## Phase 2 plug-in shape
//!
//! Phase 2 will add `load_with_unseal` variants that take an
//! `UnsealMaterial` enum (Shamir shares, KMS handle, HSM session). The
//! Phase 1 loader stays usable for the plaintext path indefinitely so
//! tests don't need to set up KMS.

use anyhow::Context;

use crate::crypto::aead::{random_dek, KEY_LEN};
use crate::crypto::kek::KekHandle;

/// Sealing method — the column value in `vault.kek_metadata`.
pub const METHOD_PLAINTEXT: &str = "plaintext";

/// Load the active KEK or generate one on first boot.
///
/// "Active" = the row with the most recent `created_at`. If no row
/// exists, a fresh 32-byte KEK is minted, persisted with
/// `sealing_method = 'plaintext'`, and returned.
///
/// The returned handle holds the unsealed bytes in memory; persisting
/// them in plaintext is the explicit Phase 1 trade-off.
#[cfg(feature = "backend-postgres")]
pub async fn load_or_init_postgres(pool: &sqlx::PgPool) -> anyhow::Result<KekHandle> {
    let existing: Option<(String, String, Vec<u8>)> = sqlx::query_as(
        "SELECT kid, sealing_method, sealed_blob
           FROM vault.kek_metadata
          ORDER BY created_at DESC
          LIMIT 1",
    )
    .fetch_optional(pool)
    .await
    .context("read vault.kek_metadata")?;

    if let Some((kid, method, blob)) = existing {
        let key = parse_plaintext_blob(&method, &blob)
            .with_context(|| format!("unwrap KEK kid={kid}"))?;
        warn_if_plaintext(&kid, &method);
        return Ok(KekHandle::from_bytes(kid, key));
    }

    let key = random_dek();
    let handle = KekHandle::from_bytes(content_addressed_kid(&key), key);
    sqlx::query(
        "INSERT INTO vault.kek_metadata
            (kid, sealing_method, sealed, sealed_blob, sealed_at, unsealed_at)
         VALUES ($1, $2, FALSE, $3, NULL, EXTRACT(EPOCH FROM NOW()))",
    )
    .bind(handle.kid())
    .bind(METHOD_PLAINTEXT)
    .bind(key.as_slice())
    .execute(pool)
    .await
    .context("seed vault.kek_metadata")?;
    warn_first_boot_plaintext(handle.kid());
    Ok(handle)
}

/// SQLite mirror of [`load_or_init_postgres`].
#[cfg(feature = "backend-sqlite")]
pub async fn load_or_init_sqlite(pool: &sqlx::SqlitePool) -> anyhow::Result<KekHandle> {
    let existing: Option<(String, String, Vec<u8>)> = sqlx::query_as(
        "SELECT kid, sealing_method, sealed_blob
           FROM vault.kek_metadata
          ORDER BY created_at DESC
          LIMIT 1",
    )
    .fetch_optional(pool)
    .await
    .context("read vault.kek_metadata")?;

    if let Some((kid, method, blob)) = existing {
        let key = parse_plaintext_blob(&method, &blob)
            .with_context(|| format!("unwrap KEK kid={kid}"))?;
        warn_if_plaintext(&kid, &method);
        return Ok(KekHandle::from_bytes(kid, key));
    }

    let key = random_dek();
    let handle = KekHandle::from_bytes(content_addressed_kid(&key), key);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64();
    sqlx::query(
        "INSERT INTO vault.kek_metadata
            (kid, sealing_method, sealed, sealed_blob, sealed_at, unsealed_at, created_at)
         VALUES (?, ?, 0, ?, NULL, ?, ?)",
    )
    .bind(handle.kid())
    .bind(METHOD_PLAINTEXT)
    .bind(key.as_slice())
    .bind(now)
    .bind(now)
    .execute(pool)
    .await
    .context("seed vault.kek_metadata")?;
    warn_first_boot_plaintext(handle.kid());
    Ok(handle)
}

fn parse_plaintext_blob(method: &str, blob: &[u8]) -> anyhow::Result<[u8; KEY_LEN]> {
    if method != METHOD_PLAINTEXT {
        anyhow::bail!(
            "vault.kek_metadata.sealing_method = '{method}' is not supported in Phase 1; \
             Phase 2 ships shamir / kms / hsm sealing"
        );
    }
    if blob.len() != KEY_LEN {
        anyhow::bail!(
            "plaintext KEK blob is {} bytes; expected {KEY_LEN}",
            blob.len()
        );
    }
    let mut key = [0u8; KEY_LEN];
    key.copy_from_slice(blob);
    Ok(key)
}

fn content_addressed_kid(key: &[u8; KEY_LEN]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(b"assay-vault/kek-kid/v1");
    h.update(key);
    let d = h.finalize();
    format!("kek-{}", data_encoding::HEXLOWER_PERMISSIVE.encode(&d[..8]))
}

fn warn_if_plaintext(kid: &str, method: &str) {
    if method == METHOD_PLAINTEXT {
        tracing::warn!(
            target: "assay-vault",
            kid, method,
            "vault running with plaintext KEK at rest. Move to shamir / kms / hsm sealing in Phase 2 — \
             see plan 17 §S7."
        );
    }
}

fn warn_first_boot_plaintext(kid: &str) {
    tracing::warn!(
        target: "assay-vault",
        kid,
        "first-boot plaintext KEK persisted. Phase 1 placeholder; rotate to a real sealing method as \
         Phase 2 lands."
    );
}

#[cfg(all(test, feature = "backend-sqlite"))]
mod tests {
    use super::*;
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
    use sqlx::Executor;
    use std::str::FromStr;

    async fn boot_pool() -> sqlx::SqlitePool {
        use std::sync::atomic::{AtomicU64, Ordering};
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let suffix = format!(
            "{}_{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        );
        let v = format!("file:assay_kek_v_{suffix}?mode=memory&cache=shared");
        let e = format!("file:assay_kek_e_{suffix}?mode=memory&cache=shared");
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
        crate::schema::migrate_sqlite(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn first_boot_seeds_kek() {
        let pool = boot_pool().await;
        let h1 = load_or_init_sqlite(&pool).await.unwrap();
        let h2 = load_or_init_sqlite(&pool).await.unwrap();
        // Same kid both times — second call loads, doesn't re-seed.
        assert_eq!(h1.kid(), h2.kid());
        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM vault.kek_metadata")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count.0, 1, "second load must not insert a new row");
    }

    #[tokio::test]
    async fn rejects_unknown_sealing_method() {
        let pool = boot_pool().await;
        sqlx::query(
            "INSERT INTO vault.kek_metadata
                (kid, sealing_method, sealed, sealed_blob, created_at)
             VALUES ('kek-x', 'kms-aws', 1, x'', 0.0)",
        )
        .execute(&pool)
        .await
        .unwrap();
        let res = load_or_init_sqlite(&pool).await;
        assert!(
            res.is_err(),
            "non-plaintext sealing must be rejected in Phase 1"
        );
    }

    #[tokio::test]
    async fn rejects_truncated_plaintext_blob() {
        let pool = boot_pool().await;
        sqlx::query(
            "INSERT INTO vault.kek_metadata
                (kid, sealing_method, sealed, sealed_blob, created_at)
             VALUES ('kek-y', 'plaintext', 0, x'beef', 0.0)",
        )
        .execute(&pool)
        .await
        .unwrap();
        assert!(load_or_init_sqlite(&pool).await.is_err());
    }
}
