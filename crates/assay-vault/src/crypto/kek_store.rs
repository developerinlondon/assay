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

use crate::crypto::aead::{KEY_LEN, random_dek};
use crate::crypto::env_seal::{ENV_VAR, METHOD_ENV, SealKey};
use crate::crypto::kek::KekHandle;

/// Sealing method — the column value in `vault.kek_metadata`.
pub const METHOD_PLAINTEXT: &str = "plaintext";
pub const METHOD_SHAMIR: &str = "shamir";

/// Outcome of [`load_active_*`] — fully describes the at-rest state so
/// engine boot can construct the right [`crate::crypto::seal_state::SealState`].
#[non_exhaustive]
pub enum ActiveKek {
    /// Plaintext sealing (Phase 1 placeholder). The KEK is in memory.
    Plaintext { kid: String, handle: KekHandle },
    /// Shamir-sealed. The engine cannot use the vault until the
    /// operator submits `threshold` shares.
    Shamir {
        kid: String,
        threshold: u8,
        shares_count: u8,
    },
}

#[cfg(feature = "vault-sealing-shamir")]
use crate::crypto::sealing::shamir::{Share, split_kek};

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
    load_or_init_postgres_sealed(pool, None).await
}

/// Load the active KEK, sealing it under `seal` when one is supplied.
///
/// A store already holding a plaintext KEK is re-sealed in place on the
/// first boot that has a seal key, so turning sealing on is a restart
/// rather than a migration. Re-running with the same key is a no-op.
#[cfg(feature = "backend-postgres")]
pub async fn load_or_init_postgres_sealed(
    pool: &sqlx::PgPool,
    seal: Option<&SealKey>,
) -> anyhow::Result<KekHandle> {
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
        let key = open_stored_kek(&kid, &method, &blob, seal)?;
        if let Some(seal) = seal
            && method == METHOD_PLAINTEXT
        {
            let resealed = seal.seal(&kid, &key)?;
            sqlx::query(
                "UPDATE vault.kek_metadata
                    SET sealing_method = $1, sealed_blob = $2
                  WHERE kid = $3",
            )
            .bind(METHOD_ENV)
            .bind(resealed)
            .bind(&kid)
            .execute(pool)
            .await
            .context("re-seal vault.kek_metadata")?;
            warn_resealed(&kid);
        }
        return Ok(KekHandle::from_bytes(kid, key));
    }

    let key = random_dek();
    let handle = KekHandle::from_bytes(content_addressed_kid(&key), key);
    let (method, blob) = seal_for_storage(handle.kid(), &key, seal)?;
    sqlx::query(
        "INSERT INTO vault.kek_metadata
            (kid, sealing_method, sealed, sealed_blob, sealed_at, unsealed_at)
         VALUES ($1, $2, FALSE, $3, NULL, EXTRACT(EPOCH FROM NOW()))",
    )
    .bind(handle.kid())
    .bind(method)
    .bind(blob)
    .execute(pool)
    .await
    .context("seed vault.kek_metadata")?;
    Ok(handle)
}

/// SQLite mirror of [`load_or_init_postgres`].
#[cfg(feature = "backend-sqlite")]
pub async fn load_or_init_sqlite(pool: &sqlx::SqlitePool) -> anyhow::Result<KekHandle> {
    load_or_init_sqlite_sealed(pool, None).await
}

/// SQLite mirror of [`load_or_init_postgres_sealed`].
#[cfg(feature = "backend-sqlite")]
pub async fn load_or_init_sqlite_sealed(
    pool: &sqlx::SqlitePool,
    seal: Option<&SealKey>,
) -> anyhow::Result<KekHandle> {
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
        let key = open_stored_kek(&kid, &method, &blob, seal)?;
        if let Some(seal) = seal
            && method == METHOD_PLAINTEXT
        {
            let resealed = seal.seal(&kid, &key)?;
            sqlx::query(
                "UPDATE vault.kek_metadata
                    SET sealing_method = ?, sealed_blob = ?
                  WHERE kid = ?",
            )
            .bind(METHOD_ENV)
            .bind(resealed)
            .bind(&kid)
            .execute(pool)
            .await
            .context("re-seal vault.kek_metadata")?;
            warn_resealed(&kid);
        }
        return Ok(KekHandle::from_bytes(kid, key));
    }

    let key = random_dek();
    let handle = KekHandle::from_bytes(content_addressed_kid(&key), key);
    let (method, blob) = seal_for_storage(handle.kid(), &key, seal)?;
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
    .bind(method)
    .bind(blob)
    .bind(now)
    .bind(now)
    .execute(pool)
    .await
    .context("seed vault.kek_metadata")?;
    Ok(handle)
}

/// Open a stored KEK according to the method its row records.
///
/// An env-sealed row without a seal key is a hard error: booting on
/// would mint a second KEK and orphan every secret the first one wraps.
fn open_stored_kek(
    kid: &str,
    method: &str,
    blob: &[u8],
    seal: Option<&SealKey>,
) -> anyhow::Result<[u8; KEY_LEN]> {
    match method {
        METHOD_PLAINTEXT => {
            let key = parse_plaintext_blob(method, blob)
                .with_context(|| format!("unwrap KEK kid={kid}"))?;
            if seal.is_none() {
                warn_if_plaintext(kid, method);
            }
            Ok(key)
        }
        METHOD_ENV => {
            let seal = seal.ok_or_else(|| {
                anyhow::anyhow!(
                    "vault KEK kid={kid} is sealed with {METHOD_ENV} but {ENV_VAR} is not set; \
                     set it to the key this store was sealed with"
                )
            })?;
            Ok(seal.unseal(kid, blob)?)
        }
        other => anyhow::bail!("unsupported vault sealing_method '{other}' for kid={kid}"),
    }
}

/// The method name and blob to persist for a freshly minted KEK.
fn seal_for_storage(
    kid: &str,
    key: &[u8; KEY_LEN],
    seal: Option<&SealKey>,
) -> anyhow::Result<(&'static str, Vec<u8>)> {
    match seal {
        Some(seal) => {
            tracing::info!(
                target: "assay-vault",
                kid = %kid,
                "first-boot KEK sealed with the environment seal key"
            );
            Ok((METHOD_ENV, seal.seal(kid, key)?))
        }
        None => {
            warn_first_boot_plaintext(kid);
            Ok((METHOD_PLAINTEXT, key.to_vec()))
        }
    }
}

fn warn_resealed(kid: &str) {
    tracing::warn!(
        target: "assay-vault",
        kid = %kid,
        "vault KEK was stored in plaintext and has been re-sealed with {ENV_VAR}; \
         database backups taken before now still contain the unsealed key"
    );
}

fn parse_plaintext_blob(method: &str, blob: &[u8]) -> anyhow::Result<[u8; KEY_LEN]> {
    if method != METHOD_PLAINTEXT {
        anyhow::bail!(
            "parse_plaintext_blob called for sealing_method = '{method}'; \
             this is a code bug — non-plaintext methods take a different code path"
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

/// One row from vault.kek_metadata as queried by the load_active_*
/// helpers. Tuple alias keeps clippy::type-complexity happy without
/// adding a real DTO.
#[cfg(feature = "backend-sqlite")]
type SqliteKekRow = (String, String, Vec<u8>, Option<i64>, Option<i64>);

#[cfg(feature = "backend-postgres")]
type PgKekRow = (String, String, Vec<u8>, Option<i32>, Option<i32>);

/// Read the active row from `vault.kek_metadata`, returning the parsed
/// state. Phase-2 entrypoint that distinguishes plaintext from
/// shamir-sealed installations. Caller hands the result to
/// [`crate::crypto::seal_state::SealState`] to build the runtime state.
#[cfg(feature = "backend-sqlite")]
pub async fn load_active_sqlite(pool: &sqlx::SqlitePool) -> anyhow::Result<Option<ActiveKek>> {
    let row: Option<SqliteKekRow> = sqlx::query_as(
        "SELECT kid, sealing_method, sealed_blob, share_threshold, share_count
           FROM vault.kek_metadata
          ORDER BY created_at DESC
          LIMIT 1",
    )
    .fetch_optional(pool)
    .await
    .context("read vault.kek_metadata")?;

    let Some((kid, method, blob, threshold, shares_count)) = row else {
        return Ok(None);
    };
    match method.as_str() {
        METHOD_PLAINTEXT => {
            let key = parse_plaintext_blob(&method, &blob)
                .with_context(|| format!("unwrap plaintext KEK kid={kid}"))?;
            warn_if_plaintext(&kid, &method);
            Ok(Some(ActiveKek::Plaintext {
                kid: kid.clone(),
                handle: KekHandle::from_bytes(kid, key),
            }))
        }
        METHOD_SHAMIR => {
            let threshold = threshold
                .ok_or_else(|| anyhow::anyhow!("shamir-sealed kid={kid} missing share_threshold"))?
                as u8;
            let shares_count = shares_count
                .ok_or_else(|| anyhow::anyhow!("shamir-sealed kid={kid} missing share_count"))?
                as u8;
            Ok(Some(ActiveKek::Shamir {
                kid,
                threshold,
                shares_count,
            }))
        }
        other => anyhow::bail!(
            "vault.kek_metadata.sealing_method = '{other}' is not yet supported; \
             current build handles plaintext + shamir"
        ),
    }
}

#[cfg(feature = "backend-postgres")]
pub async fn load_active_postgres(pool: &sqlx::PgPool) -> anyhow::Result<Option<ActiveKek>> {
    let row: Option<PgKekRow> = sqlx::query_as(
        "SELECT kid, sealing_method, sealed_blob, share_threshold, share_count
           FROM vault.kek_metadata
          ORDER BY created_at DESC
          LIMIT 1",
    )
    .fetch_optional(pool)
    .await
    .context("read vault.kek_metadata")?;

    let Some((kid, method, blob, threshold, shares_count)) = row else {
        return Ok(None);
    };
    match method.as_str() {
        METHOD_PLAINTEXT => {
            let key = parse_plaintext_blob(&method, &blob)
                .with_context(|| format!("unwrap plaintext KEK kid={kid}"))?;
            warn_if_plaintext(&kid, &method);
            Ok(Some(ActiveKek::Plaintext {
                kid: kid.clone(),
                handle: KekHandle::from_bytes(kid, key),
            }))
        }
        METHOD_SHAMIR => {
            let threshold = threshold
                .ok_or_else(|| anyhow::anyhow!("shamir-sealed kid={kid} missing share_threshold"))?
                as u8;
            let shares_count = shares_count
                .ok_or_else(|| anyhow::anyhow!("shamir-sealed kid={kid} missing share_count"))?
                as u8;
            Ok(Some(ActiveKek::Shamir {
                kid,
                threshold,
                shares_count,
            }))
        }
        other => anyhow::bail!(
            "vault.kek_metadata.sealing_method = '{other}' is not yet supported; \
             current build handles plaintext + shamir"
        ),
    }
}

/// Init a fresh Shamir-sealed KEK. Generates 32 random bytes, splits
/// them into `shares_count` Shamir shares (any `threshold` reconstruct),
/// persists the metadata row with `sealed_blob = ''`, and returns the
/// shares to the operator. The shares are returned ONCE — the engine
/// does not retain a copy. Operators MUST distribute and store them
/// securely (typically among trusted humans).
///
/// Returns the new kid + the raw share bytes. Each share is the binary
/// `sharks::Share` representation; operators submit these verbatim to
/// `/sys/unseal`.
///
/// The caller is responsible for clearing prior `kek_metadata` rows
/// when rotating from plaintext sealing — Phase 2 ships init-from-empty
/// and init-replacing-plaintext only; cross-method KEK rotation
/// (re-wrapping every existing DEK to the new KEK) lands later.
#[cfg(all(feature = "backend-sqlite", feature = "vault-sealing-shamir"))]
pub async fn init_shamir_sqlite(
    pool: &sqlx::SqlitePool,
    threshold: u8,
    shares_count: u8,
) -> anyhow::Result<(String, Vec<Share>)> {
    if threshold == 0 || shares_count == 0 || threshold > shares_count {
        anyhow::bail!("invalid shamir params: threshold={threshold}, shares_count={shares_count}");
    }
    let key = random_dek();
    let kid = content_addressed_kid(&key);
    let shares =
        split_kek(&key, threshold, shares_count).map_err(|e| anyhow::anyhow!("split_kek: {e}"))?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64();
    sqlx::query(
        "INSERT INTO vault.kek_metadata
            (kid, sealing_method, sealed, sealed_blob, share_threshold, share_count, sealed_at, unsealed_at, created_at)
         VALUES (?, ?, 0, x'', ?, ?, NULL, ?, ?)",
    )
    .bind(&kid)
    .bind(METHOD_SHAMIR)
    .bind(threshold as i64)
    .bind(shares_count as i64)
    .bind(now)
    .bind(now)
    .execute(pool)
    .await
    .context("insert shamir kek_metadata row")?;
    tracing::info!(
        target: "assay-vault",
        kid = %kid, threshold, shares_count,
        "vault sealed with shamir; operator must store the returned shares"
    );
    Ok((kid, shares))
}

#[cfg(all(feature = "backend-postgres", feature = "vault-sealing-shamir"))]
pub async fn init_shamir_postgres(
    pool: &sqlx::PgPool,
    threshold: u8,
    shares_count: u8,
) -> anyhow::Result<(String, Vec<Share>)> {
    if threshold == 0 || shares_count == 0 || threshold > shares_count {
        anyhow::bail!("invalid shamir params: threshold={threshold}, shares_count={shares_count}");
    }
    let key = random_dek();
    let kid = content_addressed_kid(&key);
    let shares =
        split_kek(&key, threshold, shares_count).map_err(|e| anyhow::anyhow!("split_kek: {e}"))?;
    sqlx::query(
        "INSERT INTO vault.kek_metadata
            (kid, sealing_method, sealed, sealed_blob, share_threshold, share_count, sealed_at, unsealed_at)
         VALUES ($1, $2, FALSE, ''::bytea, $3, $4, NULL, EXTRACT(EPOCH FROM NOW()))",
    )
    .bind(&kid)
    .bind(METHOD_SHAMIR)
    .bind(threshold as i32)
    .bind(shares_count as i32)
    .execute(pool)
    .await
    .context("insert shamir kek_metadata row")?;
    tracing::info!(
        target: "assay-vault",
        kid = %kid, threshold, shares_count,
        "vault sealed with shamir; operator must store the returned shares"
    );
    Ok((kid, shares))
}

/// Mark a kek_metadata row as sealed/unsealed in the DB. The runtime
/// [`crate::crypto::seal_state::SealState`] is the source of truth for
/// in-memory state; this is the audit / reboot signal.
#[cfg(feature = "backend-sqlite")]
pub async fn set_sealed_flag_sqlite(
    pool: &sqlx::SqlitePool,
    kid: &str,
    sealed: bool,
) -> anyhow::Result<()> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64();
    let q = if sealed {
        "UPDATE vault.kek_metadata SET sealed = 1, sealed_at = ? WHERE kid = ?"
    } else {
        "UPDATE vault.kek_metadata SET sealed = 0, unsealed_at = ? WHERE kid = ?"
    };
    sqlx::query(q)
        .bind(now)
        .bind(kid)
        .execute(pool)
        .await
        .context("update sealed flag")?;
    Ok(())
}

#[cfg(feature = "backend-postgres")]
pub async fn set_sealed_flag_postgres(
    pool: &sqlx::PgPool,
    kid: &str,
    sealed: bool,
) -> anyhow::Result<()> {
    let q = if sealed {
        "UPDATE vault.kek_metadata SET sealed = TRUE, sealed_at = EXTRACT(EPOCH FROM NOW()) WHERE kid = $1"
    } else {
        "UPDATE vault.kek_metadata SET sealed = FALSE, unsealed_at = EXTRACT(EPOCH FROM NOW()) WHERE kid = $1"
    };
    sqlx::query(q)
        .bind(kid)
        .execute(pool)
        .await
        .context("update sealed flag")?;
    Ok(())
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
    use sqlx::Executor;
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
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
