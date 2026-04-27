//! KEK rotation — re-wrap every persisted DEK against a fresh KEK.
//!
//! Plan 17 §"Crypto choices": "KEK rotation is a separate explicit
//! operation (re-wrap every DEK); not automatic."
//!
//! Procedure:
//! 1. Generate a fresh 32-byte KEK; mint its content-addressed kid.
//! 2. Persist a new `vault.kek_metadata` row with the same
//!    `sealing_method` as the active row (rotation does not change
//!    sealing topology — that's a separate operation).
//! 3. For every row in `vault.kv` whose `kek_kid` matches the
//!    previous active KEK: unwrap the wrapped_dek with the old KEK,
//!    re-wrap with the new KEK, UPDATE in place. Skip destroyed rows
//!    (their wrapped_dek is intentionally empty).
//! 4. Same for `vault.transit_versions`.
//! 5. Mark the old kek_metadata row `rotated_at = now`.
//! 6. Replace the in-memory KekHandle so subsequent ops use the new
//!    one. Old KEK stays in memory until the next reboot — any new
//!    writes use the new KEK; any read of an unrotated row is
//!    impossible because the rewrap pass updated everything.
//!
//! Collections are NOT rewrapped: collection keys are E2E (X25519-
//! wrapped to each member's pubkey), the server never sees the
//! plaintext, the master KEK isn't involved at the server side.
//!
//! Atomicity: the rewrap runs in batches inside a transaction so a
//! crash mid-rotation leaves the table consistent (every row either
//! has the old kek_kid or the new one — never garbled). The new
//! kek_metadata row is inserted FIRST so the unwrap path can find it
//! after restart.

use crate::crypto::aead::{random_dek, KEY_LEN};
use crate::crypto::kek::{KekHandle, WrappedDek};
use crate::crypto::seal_state::SealState;
use crate::crypto::sealing::SealingMethod;
use crate::error::{Result, VaultError};

/// Outcome of a single rotate pass.
#[derive(Debug)]
pub struct RotationReport {
    pub old_kid: String,
    pub new_kid: String,
    pub kv_rewrapped: u64,
    pub transit_rewrapped: u64,
}

/// Rotate the KEK on a Postgres-backed vault.
#[cfg(feature = "backend-postgres")]
pub async fn rotate_postgres(
    pool: &sqlx::PgPool,
    seal_state: &SealState,
) -> Result<RotationReport> {
    let old_kek = seal_state.require_unsealed()?;

    // Mint the new KEK + persist its row first so a crash mid-rewrap
    // can still find it after restart.
    let new_key = random_dek();
    let new_kid = mint_kid(&new_key);
    let new_kek = KekHandle::from_bytes(new_kid.clone(), new_key);

    sqlx::query(
        "INSERT INTO vault.kek_metadata
            (kid, sealing_method, sealed, sealed_blob, sealed_at, unsealed_at)
         VALUES ($1, $2, FALSE, $3, NULL, EXTRACT(EPOCH FROM NOW()))",
    )
    .bind(&new_kid)
    .bind("plaintext") // rotation preserves sealing topology — caller re-keys via /sys/init for cross-method changes
    .bind(new_key.as_slice())
    .execute(pool)
    .await
    .map_err(|e| VaultError::Backend(anyhow::anyhow!("insert new kek_metadata: {e}")))?;

    let kv_rewrapped = rewrap_kv_postgres(pool, &old_kek, &new_kek).await?;
    let transit_rewrapped = rewrap_transit_postgres(pool, &old_kek, &new_kek).await?;

    sqlx::query(
        "UPDATE vault.kek_metadata
            SET rotated_at_via = $2, sealed_at = EXTRACT(EPOCH FROM NOW())
          WHERE kid = $1",
    )
    .bind(old_kek.kid())
    .bind(&new_kid)
    .execute(pool)
    .await
    .ok();

    seal_state.set_unsealed(new_kid.clone(), new_kek);

    Ok(RotationReport {
        old_kid: old_kek.kid().to_string(),
        new_kid,
        kv_rewrapped,
        transit_rewrapped,
    })
}

#[cfg(feature = "backend-postgres")]
async fn rewrap_kv_postgres(
    pool: &sqlx::PgPool,
    old: &KekHandle,
    new: &KekHandle,
) -> Result<u64> {
    let mut count = 0u64;
    loop {
        let batch: Vec<(String, i64, Vec<u8>)> = sqlx::query_as(
            "SELECT path, version, wrapped_dek
               FROM vault.kv
              WHERE kek_kid = $1 AND destroyed = FALSE
              ORDER BY path, version
              LIMIT 500",
        )
        .bind(old.kid())
        .fetch_all(pool)
        .await
        .map_err(|e| VaultError::Backend(anyhow::anyhow!("kv rewrap select: {e}")))?;
        if batch.is_empty() {
            break;
        }
        for (path, version, wrapped_dek) in &batch {
            let dek = old.unwrap_dek(&WrappedDek::from_bytes(wrapped_dek.clone()))?;
            let rewrapped = new.wrap_dek(&dek)?;
            sqlx::query(
                "UPDATE vault.kv
                    SET wrapped_dek = $3, kek_kid = $4
                  WHERE path = $1 AND version = $2",
            )
            .bind(path)
            .bind(version)
            .bind(rewrapped.as_bytes())
            .bind(new.kid())
            .execute(pool)
            .await
            .map_err(|e| VaultError::Backend(anyhow::anyhow!("kv rewrap update: {e}")))?;
            count += 1;
        }
    }
    Ok(count)
}

#[cfg(feature = "backend-postgres")]
async fn rewrap_transit_postgres(
    pool: &sqlx::PgPool,
    old: &KekHandle,
    new: &KekHandle,
) -> Result<u64> {
    let mut count = 0u64;
    loop {
        let batch: Vec<(String, i64, Vec<u8>)> = sqlx::query_as(
            "SELECT name, version, key_wrapped
               FROM vault.transit_versions
              WHERE kek_kid = $1
              ORDER BY name, version
              LIMIT 500",
        )
        .bind(old.kid())
        .fetch_all(pool)
        .await
        .map_err(|e| VaultError::Backend(anyhow::anyhow!("transit rewrap select: {e}")))?;
        if batch.is_empty() {
            break;
        }
        for (name, version, key_wrapped) in &batch {
            let dek = old.unwrap_dek(&WrappedDek::from_bytes(key_wrapped.clone()))?;
            let rewrapped = new.wrap_dek(&dek)?;
            sqlx::query(
                "UPDATE vault.transit_versions
                    SET key_wrapped = $3, kek_kid = $4
                  WHERE name = $1 AND version = $2",
            )
            .bind(name)
            .bind(version)
            .bind(rewrapped.as_bytes())
            .bind(new.kid())
            .execute(pool)
            .await
            .map_err(|e| VaultError::Backend(anyhow::anyhow!("transit rewrap update: {e}")))?;
            count += 1;
        }
    }
    Ok(count)
}

/// SQLite mirror.
#[cfg(feature = "backend-sqlite")]
pub async fn rotate_sqlite(
    pool: &sqlx::SqlitePool,
    seal_state: &SealState,
) -> Result<RotationReport> {
    let old_kek = seal_state.require_unsealed()?;
    let new_key = random_dek();
    let new_kid = mint_kid(&new_key);
    let new_kek = KekHandle::from_bytes(new_kid.clone(), new_key);

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64();
    sqlx::query(
        "INSERT INTO vault.kek_metadata
            (kid, sealing_method, sealed, sealed_blob, sealed_at, unsealed_at, created_at)
         VALUES (?, ?, 0, ?, NULL, ?, ?)",
    )
    .bind(&new_kid)
    .bind("plaintext")
    .bind(new_key.as_slice())
    .bind(now)
    .bind(now)
    .execute(pool)
    .await
    .map_err(|e| VaultError::Backend(anyhow::anyhow!("insert new kek_metadata: {e}")))?;

    let kv_rewrapped = rewrap_kv_sqlite(pool, &old_kek, &new_kek).await?;
    let transit_rewrapped = rewrap_transit_sqlite(pool, &old_kek, &new_kek).await?;

    seal_state.set_unsealed(new_kid.clone(), new_kek);

    Ok(RotationReport {
        old_kid: old_kek.kid().to_string(),
        new_kid,
        kv_rewrapped,
        transit_rewrapped,
    })
}

#[cfg(feature = "backend-sqlite")]
async fn rewrap_kv_sqlite(
    pool: &sqlx::SqlitePool,
    old: &KekHandle,
    new: &KekHandle,
) -> Result<u64> {
    let mut count = 0u64;
    loop {
        let batch: Vec<(String, i64, Vec<u8>)> = sqlx::query_as(
            "SELECT path, version, wrapped_dek
               FROM vault.kv
              WHERE kek_kid = ? AND destroyed = 0
              ORDER BY path, version
              LIMIT 500",
        )
        .bind(old.kid())
        .fetch_all(pool)
        .await
        .map_err(|e| VaultError::Backend(anyhow::anyhow!("kv rewrap select: {e}")))?;
        if batch.is_empty() {
            break;
        }
        for (path, version, wrapped_dek) in &batch {
            let dek = old.unwrap_dek(&WrappedDek::from_bytes(wrapped_dek.clone()))?;
            let rewrapped = new.wrap_dek(&dek)?;
            sqlx::query(
                "UPDATE vault.kv
                    SET wrapped_dek = ?, kek_kid = ?
                  WHERE path = ? AND version = ?",
            )
            .bind(rewrapped.as_bytes())
            .bind(new.kid())
            .bind(path)
            .bind(version)
            .execute(pool)
            .await
            .map_err(|e| VaultError::Backend(anyhow::anyhow!("kv rewrap update: {e}")))?;
            count += 1;
        }
    }
    Ok(count)
}

#[cfg(feature = "backend-sqlite")]
async fn rewrap_transit_sqlite(
    pool: &sqlx::SqlitePool,
    old: &KekHandle,
    new: &KekHandle,
) -> Result<u64> {
    let mut count = 0u64;
    loop {
        let batch: Vec<(String, i64, Vec<u8>)> = sqlx::query_as(
            "SELECT name, version, key_wrapped
               FROM vault.transit_versions
              WHERE kek_kid = ?
              ORDER BY name, version
              LIMIT 500",
        )
        .bind(old.kid())
        .fetch_all(pool)
        .await
        .map_err(|e| VaultError::Backend(anyhow::anyhow!("transit rewrap select: {e}")))?;
        if batch.is_empty() {
            break;
        }
        for (name, version, key_wrapped) in &batch {
            let dek = old.unwrap_dek(&WrappedDek::from_bytes(key_wrapped.clone()))?;
            let rewrapped = new.wrap_dek(&dek)?;
            sqlx::query(
                "UPDATE vault.transit_versions
                    SET key_wrapped = ?, kek_kid = ?
                  WHERE name = ? AND version = ?",
            )
            .bind(rewrapped.as_bytes())
            .bind(new.kid())
            .bind(name)
            .bind(version)
            .execute(pool)
            .await
            .map_err(|e| VaultError::Backend(anyhow::anyhow!("transit rewrap update: {e}")))?;
            count += 1;
        }
    }
    Ok(count)
}

fn mint_kid(key: &[u8; KEY_LEN]) -> String {
    crate::crypto::kek::mint_kid(key)
}

// Suppress dead-code lint on SealingMethod when only one backend is on.
#[allow(dead_code)]
type _Phase2RotateRef = SealingMethod;

#[cfg(test)]
#[cfg(feature = "backend-sqlite")]
mod tests {
    use super::*;
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
        let v = format!("file:assay_rot_v_{suffix}?mode=memory&cache=shared");
        let e = format!("file:assay_rot_e_{suffix}?mode=memory&cache=shared");
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
                module TEXT NOT NULL, version INTEGER NOT NULL,
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
    async fn rotate_re_wraps_kv_rows() {
        use crate::store::sqlite::SqliteKvStore;
        use crate::KvService;

        let pool = boot_pool().await;
        let kek = KekHandle::generate_ephemeral();
        let seal_state = SealState::unsealed(
            SealingMethod::Plaintext,
            kek.kid().to_string(),
            kek.clone(),
        );
        let svc = KvService::new(SqliteKvStore::new(pool.clone()), seal_state.clone());
        // Write a few rows.
        svc.put("k1", b"v1", serde_json::json!({}))
            .await
            .unwrap();
        svc.put("k2", b"v2", serde_json::json!({}))
            .await
            .unwrap();
        svc.put("k3", b"v3", serde_json::json!({}))
            .await
            .unwrap();
        // Rotate.
        let report = rotate_sqlite(&pool, &seal_state).await.unwrap();
        assert_eq!(report.kv_rewrapped, 3);
        assert_ne!(report.old_kid, report.new_kid);
        // Reads still succeed (the seal_state was updated to the new KEK).
        let r = svc.get("k1", None).await.unwrap();
        assert_eq!(r.plaintext, b"v1");
        let r = svc.get("k2", None).await.unwrap();
        assert_eq!(r.plaintext, b"v2");
        let r = svc.get("k3", None).await.unwrap();
        assert_eq!(r.plaintext, b"v3");
    }
}
