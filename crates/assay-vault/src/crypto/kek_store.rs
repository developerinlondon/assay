//! KEK persistence — load the active KEK from `vault.kek_metadata` or
//! generate a fresh one on first boot.
//!
//! ## Nothing here decides whether plaintext is acceptable
//!
//! The loaders take an already-resolved
//! [`crate::crypto::seal_policy::Unseal`] rather than an
//! `Option<&SealKey>`, so "there is no seal key" cannot reach this
//! module as a bare `None` that silently means "write it in the clear".
//! By the time a call gets here, an operator has either supplied unseal
//! material or explicitly accepted a plaintext KEK — that decision
//! belongs to [`crate::crypto::seal_policy::SealPolicy`], which fails
//! boot when neither happened.
//!
//! ## Re-sealing is one-way and is asked about
//!
//! A store holding a plaintext row, booted with a seal key, used to be
//! rewritten in place on the spot. Afterwards the KEK exists only under
//! that key, so an operator who loses it has lost every secret the vault
//! wraps with no plaintext copy left to fall back on. The rewrite now
//! needs consent, and refusing names the backup to take first. That
//! consent rides on the [`Unseal`] the caller already had to resolve, so
//! there is exactly one place to say yes to it.
//!
//! ## Phase 2 plug-in shape
//!
//! Phase 2 will add `load_with_unseal` variants that take the full
//! unseal-material enum (Shamir shares, KMS handle, HSM session). The
//! loaders here stay the env-key and plaintext path.

use anyhow::Context;
use zeroize::Zeroizing;

use crate::crypto::aead::{KEY_LEN, random_dek};
use crate::crypto::env_seal::{ENV_VAR, METHOD_ENV};
use crate::crypto::kek::{KEK_TABLE, KekHandle};
use crate::crypto::seal_policy::{CFG_ALLOW_PLAINTEXT_KEK, CFG_ALLOW_PLAINTEXT_MIGRATION, Unseal};

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

/// Load the active KEK, or mint one on first boot, under `unseal`.
///
/// "Active" = the row with the most recent `created_at`. A fresh KEK is
/// sealed under the supplied material, or — only when the operator set
/// `allow_plaintext_kek` — written in the clear.
///
/// A store already holding a plaintext KEK is re-sealed in place on the
/// first boot that has a seal key, so turning sealing on is a restart
/// rather than a migration. Because that rewrite is one-way it happens
/// only when the `Unseal` carries consent for it. Re-running with the
/// same key is a no-op.
#[cfg(feature = "backend-postgres")]
pub async fn load_or_init_postgres_sealed(
    pool: &sqlx::PgPool,
    unseal: &Unseal,
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
        let key = open_stored_kek(&kid, &method, &blob, unseal)?;
        if let Some(seal) = unseal.seal_key()
            && method == METHOD_PLAINTEXT
        {
            require_migration_consent(&kid, unseal.may_migrate_plaintext())?;
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
        return Ok(KekHandle::from_bytes(kid, *key));
    }

    let key = Zeroizing::new(random_dek());
    let handle = KekHandle::from_bytes(content_addressed_kid(&key), *key);
    let (method, blob) = seal_for_storage(handle.kid(), &key, unseal)?;
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

/// SQLite mirror of [`load_or_init_postgres_sealed`].
#[cfg(feature = "backend-sqlite")]
pub async fn load_or_init_sqlite_sealed(
    pool: &sqlx::SqlitePool,
    unseal: &Unseal,
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
        let key = open_stored_kek(&kid, &method, &blob, unseal)?;
        if let Some(seal) = unseal.seal_key()
            && method == METHOD_PLAINTEXT
        {
            require_migration_consent(&kid, unseal.may_migrate_plaintext())?;
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
        return Ok(KekHandle::from_bytes(kid, *key));
    }

    let key = Zeroizing::new(random_dek());
    let handle = KekHandle::from_bytes(content_addressed_kid(&key), *key);
    let (method, blob) = seal_for_storage(handle.kid(), &key, unseal)?;
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
/// `allow_plaintext_kek` does not help here — it gates *minting* a key
/// in the clear, never *opening* a store that is already sealed.
fn open_stored_kek(
    kid: &str,
    method: &str,
    blob: &[u8],
    unseal: &Unseal,
) -> anyhow::Result<Zeroizing<[u8; KEY_LEN]>> {
    match method {
        METHOD_PLAINTEXT => {
            let key = parse_plaintext_blob(method, blob)
                .with_context(|| format!("unwrap KEK kid={kid}"))?;
            if !unseal.is_sealed() {
                warn_if_plaintext(kid, method);
            }
            Ok(key)
        }
        METHOD_ENV => {
            let seal = unseal.seal_key().ok_or_else(|| {
                anyhow::anyhow!(
                    "vault KEK kid={kid} is sealed with {METHOD_ENV} but no unseal material is \
                     configured — {ENV_VAR} is not set and `[vault.sealing]` names no other \
                     source that produced a key. Set it to the key this store was sealed with; \
                     `{CFG_ALLOW_PLAINTEXT_KEK}` does not open a sealed store."
                )
            })?;
            Ok(Zeroizing::new(seal.unseal(kid, blob)?))
        }
        other => anyhow::bail!("unsupported vault sealing_method '{other}' for kid={kid}"),
    }
}

/// Gate the one-way rewrite of a plaintext row into a sealed one.
///
/// The refusal has to leave an operator able to act: what to back up,
/// and what they are accepting by proceeding.
fn require_migration_consent(kid: &str, allowed: bool) -> anyhow::Result<()> {
    if allowed {
        return Ok(());
    }
    anyhow::bail!(
        "vault KEK kid={kid} is stored in the clear and unseal material is now configured, but \
         re-sealing it is one-way: afterwards the key exists only under that material, and losing \
         the material loses every secret the vault wraps, with no plaintext copy left to fall back \
         on. Back up {KEK_TABLE} first, then set `{CFG_ALLOW_PLAINTEXT_MIGRATION} = true` to let \
         this boot re-seal it. To carry on unsealed for now, remove the unseal material instead."
    )
}

/// The method name and blob to persist for a freshly minted KEK.
fn seal_for_storage(
    kid: &str,
    key: &[u8; KEY_LEN],
    unseal: &Unseal,
) -> anyhow::Result<(&'static str, Vec<u8>)> {
    match unseal {
        Unseal::Sealed { key: seal, .. } => {
            tracing::info!(
                target: "assay-vault",
                kid = %kid,
                source = %seal.origin(),
                "first-boot KEK sealed with the configured unseal material"
            );
            Ok((METHOD_ENV, seal.seal(kid, key)?))
        }
        Unseal::PlaintextPermitted => {
            error_first_boot_plaintext(kid);
            Ok((METHOD_PLAINTEXT, key.to_vec()))
        }
    }
}

fn warn_resealed(kid: &str) {
    tracing::warn!(
        target: "assay-vault",
        kid = %kid,
        "vault KEK was stored in plaintext and has been re-sealed; \
         database backups taken before now still contain the unsealed key"
    );
}

fn parse_plaintext_blob(method: &str, blob: &[u8]) -> anyhow::Result<Zeroizing<[u8; KEY_LEN]>> {
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
    let mut key = Zeroizing::new([0u8; KEY_LEN]);
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
                handle: KekHandle::from_bytes(kid, *key),
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
                handle: KekHandle::from_bytes(kid, *key),
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
            "vault running with a plaintext KEK at rest. Configure `[vault.sealing]` and set \
             `{CFG_ALLOW_PLAINTEXT_MIGRATION} = true` once {KEK_TABLE} is backed up, to seal it."
        );
    }
}

/// ERROR rather than WARN: this is a deployment running without the
/// protection sealing exists to give, and it only happens because an
/// operator asked for it in config. It should be visible in a log the
/// way an incident is, not the way a deprecation notice is.
fn error_first_boot_plaintext(kid: &str) {
    tracing::error!(
        target: "assay-vault",
        kid,
        table = KEK_TABLE,
        "first-boot vault KEK written to {KEK_TABLE} in the clear, because \
         `{CFG_ALLOW_PLAINTEXT_KEK}` is set. A dump of this database is a copy of every secret \
         the vault will hold."
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

    /// The permitted-plaintext path, which is what the old no-argument
    /// convenience wrapper did implicitly. Spelling it out per call is
    /// the point: a test that wants a key in the clear now says so.
    async fn load_plaintext(pool: &sqlx::SqlitePool) -> anyhow::Result<KekHandle> {
        load_or_init_sqlite_sealed(pool, &Unseal::PlaintextPermitted).await
    }

    async fn load_sealed(
        pool: &sqlx::SqlitePool,
        raw: &str,
        migrate: bool,
    ) -> anyhow::Result<KekHandle> {
        let key = crate::crypto::env_seal::SealKey::derive(raw)?;
        let unseal = Unseal::Sealed {
            key,
            migrate_plaintext: migrate,
        };
        load_or_init_sqlite_sealed(pool, &unseal).await
    }

    const SEAL_A: &str = "YzBmZTFhMmIzYzRkNWU2ZjcwODE5MmEzYjRjNWQ2ZTc=";

    #[tokio::test]
    async fn first_boot_seeds_kek() {
        let pool = boot_pool().await;
        let h1 = load_plaintext(&pool).await.unwrap();
        let h2 = load_plaintext(&pool).await.unwrap();
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
        let res = load_plaintext(&pool).await;
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
        assert!(load_plaintext(&pool).await.is_err());
    }

    /// Re-sealing destroys the only plaintext copy, so it waits to be
    /// asked — and the refusal has to say what to back up.
    #[tokio::test]
    async fn a_plaintext_store_is_not_resealed_without_consent() {
        let pool = boot_pool().await;
        load_plaintext(&pool).await.unwrap();

        let err = load_sealed(&pool, SEAL_A, false).await.unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains(KEK_TABLE), "name the backup to take: {msg}");
        assert!(msg.contains(CFG_ALLOW_PLAINTEXT_MIGRATION), "{msg}");

        let method: (String,) =
            sqlx::query_as("SELECT sealing_method FROM vault.kek_metadata LIMIT 1")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(method.0, METHOD_PLAINTEXT, "the row must be untouched");
    }

    #[tokio::test]
    async fn consent_lets_the_reseal_through_and_keeps_the_same_key() {
        let pool = boot_pool().await;
        let before = load_plaintext(&pool).await.unwrap();

        let after = load_sealed(&pool, SEAL_A, true).await.unwrap();
        assert_eq!(
            before.kid(),
            after.kid(),
            "re-sealing must keep the key it already had, not mint a new one"
        );

        let method: (String,) =
            sqlx::query_as("SELECT sealing_method FROM vault.kek_metadata LIMIT 1")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(method.0, METHOD_ENV);
    }

    /// The escape hatch gates minting a key in the clear. It must not
    /// become a way past a store that is already sealed — that would
    /// mint a second KEK and orphan every secret the first one wraps.
    #[tokio::test]
    async fn permitted_plaintext_does_not_open_a_sealed_store() {
        let pool = boot_pool().await;
        load_sealed(&pool, SEAL_A, false).await.unwrap();

        let err = load_plaintext(&pool).await.unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("is not set"), "{msg}");
        assert!(
            msg.contains(CFG_ALLOW_PLAINTEXT_KEK),
            "say plainly that the hatch does not help here: {msg}"
        );

        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM vault.kek_metadata")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count.0, 1, "no second KEK may be minted");
    }

    #[tokio::test]
    async fn the_wrong_key_does_not_open_a_sealed_store() {
        let pool = boot_pool().await;
        load_sealed(&pool, SEAL_A, false).await.unwrap();

        let other = "ZzBmZTFhMmIzYzRkNWU2ZjcwODE5MmEzYjRjNWQ2ZTc=";
        let err = load_sealed(&pool, other, false).await.unwrap_err();
        assert!(err.to_string().contains("does not decrypt"), "{err}");
    }
}
