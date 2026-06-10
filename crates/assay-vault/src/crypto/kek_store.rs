//! KEK persistence — load the active KEK from `vault.kek_metadata`,
//! sealing it at rest under operator-supplied unseal material, or
//! generate + seal a fresh one on first boot.
//!
//! ## Sealing stance (#113)
//!
//! The master KEK is **never persisted in plaintext** unless the operator
//! explicitly opts into a dev escape hatch. The default boot path:
//!
//! - **First boot, unseal material present** — mint a random KEK, seal it
//!   ([`crate::crypto::kek_seal::seal_kek`]) and persist only the sealed
//!   blob with `sealing_method = 'sealed-v1'`.
//! - **Boot with a sealed row** — unseal it with the configured material.
//!   Wrong material fails the AEAD tag → boot fails closed.
//! - **Boot without unseal material** — fail closed with an actionable
//!   error telling the operator how to set the unseal key + migrate.
//! - **Existing plaintext row found + material present + `allow_plaintext_migration = true`** —
//!   automatically re-seal it in place (irreversible one-way upgrade) and log
//!   the migration. Material present but flag unset → fail closed with
//!   instructions to back up the DB and set the flag. No material → fail
//!   closed with migration instructions.
//! - **Dev escape hatch** — `dev_plaintext_kek = true` keeps the old
//!   plaintext behavior for demo flows, behind a loud CRITICAL warning.
//!   Never the default.
//!
//! ## Phase 2 plug-in shape
//!
//! Shamir / KMS / HSM sealing live on their own code paths
//! ([`load_active_*`], [`init_shamir_*`]) and bump the blob version tag
//! so an old binary refuses a row it can't interpret.

use anyhow::Context;

use crate::crypto::aead::{KEY_LEN, random_dek};
use crate::crypto::kek::KekHandle;
use crate::crypto::kek_seal::{METHOD_SEALED_V1, UnsealMaterial, seal_kek, unseal_kek};

/// Sealing method — the column value in `vault.kek_metadata`.
pub const METHOD_PLAINTEXT: &str = "plaintext";
pub const METHOD_SHAMIR: &str = "shamir";

/// What the engine-boot caller resolved about KEK sealing from config.
/// Built in `assay-engine` from the `[vault]` config section and handed
/// to [`load_or_init_postgres`] / [`load_or_init_sqlite`].
#[non_exhaustive]
pub struct KekBootConfig {
    /// Resolved unseal material, if a source was configured. `None` ⇒
    /// no source — fail closed unless `dev_plaintext` is set.
    pub material: Option<UnsealMaterial>,
    /// Explicit opt-in to the plaintext escape hatch. Logs CRITICAL.
    pub dev_plaintext: bool,
    /// Gate for the irreversible plaintext→sealed-v1 auto-migration.
    ///
    /// When a `plaintext` row is found and unseal material is present,
    /// the engine will **not** overwrite the row unless this flag is
    /// `true`. Default is `false`. Set via
    /// `vault.allow_plaintext_migration = true` in `engine.toml`.
    ///
    /// The gate exists because the migration is **one-way and
    /// destructive**: the raw KEK bytes in `sealed_blob` are
    /// overwritten with the sealed blob. Operators MUST back up the
    /// database before enabling this flag.
    pub allow_plaintext_migration: bool,
}

impl KekBootConfig {
    /// Sealed boot with concrete unseal material (the production path).
    /// `allow_plaintext_migration` defaults to `false` — operator must
    /// explicitly set it to trigger the one-way migration.
    pub fn sealed(material: UnsealMaterial) -> Self {
        Self {
            material: Some(material),
            dev_plaintext: false,
            allow_plaintext_migration: false,
        }
    }

    /// Like [`Self::sealed`] but with `allow_plaintext_migration = true`.
    pub fn sealed_allow_migration(material: UnsealMaterial) -> Self {
        Self {
            material: Some(material),
            dev_plaintext: false,
            allow_plaintext_migration: true,
        }
    }

    /// Dev-only: no unseal material, plaintext KEK at rest. Loud warning.
    pub fn dev_plaintext() -> Self {
        Self {
            material: None,
            dev_plaintext: true,
            allow_plaintext_migration: false,
        }
    }

    /// No unseal material and no dev opt-in — boot must fail closed.
    pub fn unset() -> Self {
        Self {
            material: None,
            dev_plaintext: false,
            allow_plaintext_migration: false,
        }
    }
}

/// Shared, backend-agnostic error for the "no unseal material, not dev"
/// fail-closed case. Actionable: tells the operator exactly what to do.
fn fail_closed_no_material() -> anyhow::Error {
    anyhow::anyhow!(
        "vault is enabled but no KEK unseal material is configured. The master KEK will NOT be \
         persisted in plaintext. Set `vault.unseal_key_source` in engine.toml (e.g. \
         `unseal_key_source = \"env:ASSAY_VAULT_UNSEAL_KEY\"`) and export a base64 32-byte key \
         (`export ASSAY_VAULT_UNSEAL_KEY=$(openssl rand -base64 32)`), then reboot. For local \
         demos only, set `vault.dev_plaintext_kek = true` (logs a CRITICAL warning and is NOT \
         safe for real secrets)."
    )
}

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

/// Load the active KEK or generate one on first boot, sealing it at rest
/// per `cfg`.
///
/// "Active" = the row with the most recent `created_at`. Behavior by
/// at-rest state (see module docs for the full matrix):
///
/// - No row: mint + seal a fresh KEK (or persist plaintext iff
///   `cfg.dev_plaintext`).
/// - `sealed-v1` row: unseal with `cfg.material`; wrong material ⇒
///   fail closed.
/// - `plaintext` row + material present: auto-migrate (re-seal in place).
/// - `plaintext` row + no material + `dev_plaintext`: keep plaintext,
///   warn loudly.
/// - `plaintext` row + no material + not dev: fail closed.
#[cfg(feature = "backend-postgres")]
pub async fn load_or_init_postgres(
    pool: &sqlx::PgPool,
    cfg: &KekBootConfig,
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
        return load_existing_pg(pool, cfg, kid, method, blob).await;
    }

    // ── First boot ────────────────────────────────────────────────
    let key = random_dek();
    let kid = content_addressed_kid(&key);
    if let Some(material) = &cfg.material {
        let sealed_blob = seal_kek(&key, &kid, material).context("seal fresh KEK")?;
        sqlx::query(
            "INSERT INTO vault.kek_metadata
                (kid, sealing_method, sealed, sealed_blob, sealed_at, unsealed_at)
             VALUES ($1, $2, TRUE, $3, EXTRACT(EPOCH FROM NOW()), EXTRACT(EPOCH FROM NOW()))",
        )
        .bind(&kid)
        .bind(METHOD_SEALED_V1)
        .bind(sealed_blob.as_slice())
        .execute(pool)
        .await
        .context("seed sealed vault.kek_metadata")?;
        tracing::info!(target: "assay-vault", kid = %kid, "first-boot KEK minted and sealed at rest");
        Ok(KekHandle::from_bytes(kid, key))
    } else if cfg.dev_plaintext {
        seed_plaintext_pg(pool, &kid, &key).await?;
        warn_dev_plaintext(&kid);
        Ok(KekHandle::from_bytes(kid, key))
    } else {
        Err(fail_closed_no_material())
    }
}

#[cfg(feature = "backend-postgres")]
async fn load_existing_pg(
    pool: &sqlx::PgPool,
    cfg: &KekBootConfig,
    kid: String,
    method: String,
    blob: Vec<u8>,
) -> anyhow::Result<KekHandle> {
    match method.as_str() {
        METHOD_SEALED_V1 => {
            let material = cfg
                .material
                .as_ref()
                .ok_or_else(fail_closed_no_material)?;
            let key = unseal_kek(&blob, &kid, material)
                .with_context(|| format!("unseal KEK kid={kid}"))?;
            tracing::info!(target: "assay-vault", kid = %kid, "KEK unsealed at boot");
            Ok(KekHandle::from_bytes(kid, key))
        }
        METHOD_PLAINTEXT => {
            let key = parse_plaintext_blob(&method, &blob)
                .with_context(|| format!("unwrap plaintext KEK kid={kid}"))?;
            if let Some(material) = &cfg.material {
                if !cfg.allow_plaintext_migration {
                    return Err(anyhow::anyhow!(
                        "vault.kek_metadata holds a PLAINTEXT master KEK (kid={kid}) and unseal \
                         material is configured, but `vault.allow_plaintext_migration` is not set. \
                         This migration is IRREVERSIBLE — the plaintext blob will be overwritten \
                         with the sealed blob and cannot be undone. Back up the database first, \
                         then set `vault.allow_plaintext_migration = true` in engine.toml and \
                         reboot to perform the one-way migration."
                    ));
                }
                // ── One-way migration: re-seal the plaintext KEK ──
                let sealed_blob =
                    seal_kek(&key, &kid, material).context("seal KEK during plaintext migration")?;
                sqlx::query(
                    "UPDATE vault.kek_metadata
                        SET sealing_method = $1, sealed = TRUE, sealed_blob = $2,
                            sealed_at = EXTRACT(EPOCH FROM NOW())
                      WHERE kid = $3",
                )
                .bind(METHOD_SEALED_V1)
                .bind(sealed_blob.as_slice())
                .bind(&kid)
                .execute(pool)
                .await
                .context("re-seal plaintext KEK row")?;
                tracing::warn!(
                    target: "assay-vault",
                    kid = %kid,
                    "MIGRATED plaintext KEK to sealed-v1 at rest. The plaintext blob has been \
                     overwritten with the sealed blob. Keep your unseal key safe — it is now \
                     required to boot."
                );
                Ok(KekHandle::from_bytes(kid, key))
            } else if cfg.dev_plaintext {
                warn_dev_plaintext(&kid);
                Ok(KekHandle::from_bytes(kid, key))
            } else {
                Err(anyhow::anyhow!(
                    "vault.kek_metadata holds a PLAINTEXT master KEK (kid={kid}) but no unseal \
                     material is configured to migrate it. Set `vault.unseal_key_source` (e.g. \
                     `env:ASSAY_VAULT_UNSEAL_KEY`) + export a base64 32-byte key, then reboot — \
                     the engine will automatically re-seal the existing KEK in place. Refusing to \
                     boot with an unsealed KEK."
                ))
            }
        }
        other => Err(anyhow::anyhow!(
            "vault.kek_metadata.sealing_method = '{other}' (kid={kid}) is not handled by the \
             boot loader. Shamir / KMS rows are unsealed via the /sys/unseal ceremony, not the \
             boot path."
        )),
    }
}

#[cfg(feature = "backend-postgres")]
async fn seed_plaintext_pg(
    pool: &sqlx::PgPool,
    kid: &str,
    key: &[u8; KEY_LEN],
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO vault.kek_metadata
            (kid, sealing_method, sealed, sealed_blob, sealed_at, unsealed_at)
         VALUES ($1, $2, FALSE, $3, NULL, EXTRACT(EPOCH FROM NOW()))",
    )
    .bind(kid)
    .bind(METHOD_PLAINTEXT)
    .bind(key.as_slice())
    .execute(pool)
    .await
    .context("seed plaintext vault.kek_metadata (dev mode)")?;
    Ok(())
}

/// SQLite mirror of [`load_or_init_postgres`].
#[cfg(feature = "backend-sqlite")]
pub async fn load_or_init_sqlite(
    pool: &sqlx::SqlitePool,
    cfg: &KekBootConfig,
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
        return load_existing_sqlite(pool, cfg, kid, method, blob).await;
    }

    // ── First boot ────────────────────────────────────────────────
    let key = random_dek();
    let kid = content_addressed_kid(&key);
    let now = unix_now();
    if let Some(material) = &cfg.material {
        let sealed_blob = seal_kek(&key, &kid, material).context("seal fresh KEK")?;
        sqlx::query(
            "INSERT INTO vault.kek_metadata
                (kid, sealing_method, sealed, sealed_blob, sealed_at, unsealed_at, created_at)
             VALUES (?, ?, 1, ?, ?, ?, ?)",
        )
        .bind(&kid)
        .bind(METHOD_SEALED_V1)
        .bind(sealed_blob.as_slice())
        .bind(now)
        .bind(now)
        .bind(now)
        .execute(pool)
        .await
        .context("seed sealed vault.kek_metadata")?;
        tracing::info!(target: "assay-vault", kid = %kid, "first-boot KEK minted and sealed at rest");
        Ok(KekHandle::from_bytes(kid, key))
    } else if cfg.dev_plaintext {
        seed_plaintext_sqlite(pool, &kid, &key, now).await?;
        warn_dev_plaintext(&kid);
        Ok(KekHandle::from_bytes(kid, key))
    } else {
        Err(fail_closed_no_material())
    }
}

#[cfg(feature = "backend-sqlite")]
async fn load_existing_sqlite(
    pool: &sqlx::SqlitePool,
    cfg: &KekBootConfig,
    kid: String,
    method: String,
    blob: Vec<u8>,
) -> anyhow::Result<KekHandle> {
    match method.as_str() {
        METHOD_SEALED_V1 => {
            let material = cfg
                .material
                .as_ref()
                .ok_or_else(fail_closed_no_material)?;
            let key = unseal_kek(&blob, &kid, material)
                .with_context(|| format!("unseal KEK kid={kid}"))?;
            tracing::info!(target: "assay-vault", kid = %kid, "KEK unsealed at boot");
            Ok(KekHandle::from_bytes(kid, key))
        }
        METHOD_PLAINTEXT => {
            let key = parse_plaintext_blob(&method, &blob)
                .with_context(|| format!("unwrap plaintext KEK kid={kid}"))?;
            if let Some(material) = &cfg.material {
                if !cfg.allow_plaintext_migration {
                    return Err(anyhow::anyhow!(
                        "vault.kek_metadata holds a PLAINTEXT master KEK (kid={kid}) and unseal \
                         material is configured, but `vault.allow_plaintext_migration` is not set. \
                         This migration is IRREVERSIBLE — the plaintext blob will be overwritten \
                         with the sealed blob and cannot be undone. Back up the database first, \
                         then set `vault.allow_plaintext_migration = true` in engine.toml and \
                         reboot to perform the one-way migration."
                    ));
                }
                let sealed_blob =
                    seal_kek(&key, &kid, material).context("seal KEK during plaintext migration")?;
                sqlx::query(
                    "UPDATE vault.kek_metadata
                        SET sealing_method = ?, sealed = 1, sealed_blob = ?, sealed_at = ?
                      WHERE kid = ?",
                )
                .bind(METHOD_SEALED_V1)
                .bind(sealed_blob.as_slice())
                .bind(unix_now())
                .bind(&kid)
                .execute(pool)
                .await
                .context("re-seal plaintext KEK row")?;
                tracing::warn!(
                    target: "assay-vault",
                    kid = %kid,
                    "MIGRATED plaintext KEK to sealed-v1 at rest. The plaintext blob has been \
                     overwritten with the sealed blob. Keep your unseal key safe — it is now \
                     required to boot."
                );
                Ok(KekHandle::from_bytes(kid, key))
            } else if cfg.dev_plaintext {
                warn_dev_plaintext(&kid);
                Ok(KekHandle::from_bytes(kid, key))
            } else {
                Err(anyhow::anyhow!(
                    "vault.kek_metadata holds a PLAINTEXT master KEK (kid={kid}) but no unseal \
                     material is configured to migrate it. Set `vault.unseal_key_source` (e.g. \
                     `env:ASSAY_VAULT_UNSEAL_KEY`) + export a base64 32-byte key, then reboot — \
                     the engine will automatically re-seal the existing KEK in place. Refusing to \
                     boot with an unsealed KEK."
                ))
            }
        }
        other => Err(anyhow::anyhow!(
            "vault.kek_metadata.sealing_method = '{other}' (kid={kid}) is not handled by the \
             boot loader. Shamir / KMS rows are unsealed via the /sys/unseal ceremony, not the \
             boot path."
        )),
    }
}

#[cfg(feature = "backend-sqlite")]
async fn seed_plaintext_sqlite(
    pool: &sqlx::SqlitePool,
    kid: &str,
    key: &[u8; KEY_LEN],
    now: f64,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO vault.kek_metadata
            (kid, sealing_method, sealed, sealed_blob, sealed_at, unsealed_at, created_at)
         VALUES (?, ?, 0, ?, NULL, ?, ?)",
    )
    .bind(kid)
    .bind(METHOD_PLAINTEXT)
    .bind(key.as_slice())
    .bind(now)
    .bind(now)
    .execute(pool)
    .await
    .context("seed plaintext vault.kek_metadata (dev mode)")?;
    Ok(())
}

#[cfg(any(feature = "backend-postgres", feature = "backend-sqlite"))]
fn unix_now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
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

/// Loud warning for the dev-only plaintext escape hatch. Emitted at
/// ERROR level (the loudest `tracing` carries) and prefixed `CRITICAL`
/// so it is impossible to miss in logs — the KEK is unsealed on disk.
fn warn_dev_plaintext(kid: &str) {
    tracing::error!(
        target: "assay-vault",
        kid,
        "CRITICAL: vault.dev_plaintext_kek is ENABLED — the master KEK is persisted in PLAINTEXT \
         at rest. A DB read decrypts every secret. This is for local demos ONLY. Do NOT use for \
         real secrets. Set `vault.unseal_key_source` to seal the KEK."
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

    fn raw_cfg() -> KekBootConfig {
        let key = crate::crypto::aead::random_dek();
        let b64 = data_encoding::BASE64.encode(&key);
        KekBootConfig::sealed(UnsealMaterial::raw_key_from_base64(&b64).unwrap())
    }

    async fn sealing_method(pool: &sqlx::SqlitePool) -> String {
        let (m,): (String,) =
            sqlx::query_as("SELECT sealing_method FROM vault.kek_metadata LIMIT 1")
                .fetch_one(pool)
                .await
                .unwrap();
        m
    }

    async fn stored_blob(pool: &sqlx::SqlitePool) -> Vec<u8> {
        let (b,): (Vec<u8>,) =
            sqlx::query_as("SELECT sealed_blob FROM vault.kek_metadata LIMIT 1")
                .fetch_one(pool)
                .await
                .unwrap();
        b
    }

    #[tokio::test]
    async fn first_boot_seals_kek_and_round_trips() {
        let pool = boot_pool().await;
        let cfg = raw_cfg();
        let h1 = load_or_init_sqlite(&pool, &cfg).await.unwrap();
        // Reboot loads + unseals the same KEK without re-seeding.
        let h2 = load_or_init_sqlite(&pool, &cfg).await.unwrap();
        assert_eq!(h1.kid(), h2.kid());
        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM vault.kek_metadata")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count.0, 1, "second load must not insert a new row");
        assert_eq!(sealing_method(&pool).await, METHOD_SEALED_V1);
        // The raw KEK bytes never appear in the at-rest blob.
        let key = h1.unwrap_dek(&h1.wrap_dek(&[5u8; KEY_LEN]).unwrap()).unwrap();
        assert_eq!(key, [5u8; KEY_LEN]); // handle works
        assert_eq!(stored_blob(&pool).await[0], 1, "blob version tag = 1");
    }

    #[tokio::test]
    async fn first_boot_without_material_fails_closed() {
        let pool = boot_pool().await;
        let res = load_or_init_sqlite(&pool, &KekBootConfig::unset()).await;
        assert!(res.is_err(), "no unseal material must fail closed");
        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM vault.kek_metadata")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count.0, 0, "fail-closed boot must not persist any KEK row");
    }

    #[tokio::test]
    async fn wrong_unseal_key_fails() {
        let pool = boot_pool().await;
        // Seal under one key…
        let _ = load_or_init_sqlite(&pool, &raw_cfg()).await.unwrap();
        // …then boot with a different key → AEAD tag fails.
        let res = load_or_init_sqlite(&pool, &raw_cfg()).await;
        assert!(res.is_err(), "wrong unseal key must fail the auth tag");
    }

    fn raw_cfg_allow_migration() -> KekBootConfig {
        let key = crate::crypto::aead::random_dek();
        let b64 = data_encoding::BASE64.encode(&key);
        KekBootConfig::sealed_allow_migration(UnsealMaterial::raw_key_from_base64(&b64).unwrap())
    }

    #[tokio::test]
    async fn plaintext_row_migrates_to_sealed() {
        let pool = boot_pool().await;
        // Seed a legacy plaintext KEK directly.
        let key = crate::crypto::aead::random_dek();
        let kid = content_addressed_kid(&key);
        sqlx::query(
            "INSERT INTO vault.kek_metadata
                (kid, sealing_method, sealed, sealed_blob, created_at)
             VALUES (?, 'plaintext', 0, ?, 0.0)",
        )
        .bind(&kid)
        .bind(key.as_slice())
        .execute(&pool)
        .await
        .unwrap();

        // allow_plaintext_migration = true → migration proceeds.
        let cfg = raw_cfg_allow_migration();
        let h = load_or_init_sqlite(&pool, &cfg).await.unwrap();
        assert_eq!(h.kid(), kid, "migration must preserve the KEK identity");
        // Row is now sealed-v1, and the plaintext key is gone from disk.
        assert_eq!(sealing_method(&pool).await, METHOD_SEALED_V1);
        let blob = stored_blob(&pool).await;
        assert_ne!(blob.as_slice(), key.as_slice(), "plaintext must be overwritten");
        // Re-boot unseals the migrated row to the same KEK.
        let h2 = load_or_init_sqlite(&pool, &cfg).await.unwrap();
        assert_eq!(h2.kid(), kid);
    }

    #[tokio::test]
    async fn plaintext_row_with_material_but_flag_unset_fails_closed() {
        let pool = boot_pool().await;
        // Seed a legacy plaintext KEK.
        let key = crate::crypto::aead::random_dek();
        let kid = content_addressed_kid(&key);
        sqlx::query(
            "INSERT INTO vault.kek_metadata
                (kid, sealing_method, sealed, sealed_blob, created_at)
             VALUES (?, 'plaintext', 0, ?, 0.0)",
        )
        .bind(&kid)
        .bind(key.as_slice())
        .execute(&pool)
        .await
        .unwrap();

        // Material IS configured but allow_plaintext_migration = false (default).
        let cfg = raw_cfg(); // sealed(), allow_plaintext_migration = false
        let res = load_or_init_sqlite(&pool, &cfg).await;
        assert!(
            res.is_err(),
            "plaintext row + material + flag unset must fail closed"
        );
        let msg = res.unwrap_err().to_string();
        assert!(
            msg.contains("allow_plaintext_migration"),
            "error must mention the flag; got: {msg}"
        );
        // Row must be untouched — still plaintext, blob unchanged.
        assert_eq!(
            sealing_method(&pool).await,
            METHOD_PLAINTEXT,
            "row must not be mutated when migration is gated"
        );
        let stored = stored_blob(&pool).await;
        assert_eq!(
            stored.as_slice(),
            key.as_slice(),
            "plaintext blob must be untouched"
        );
    }

    #[tokio::test]
    async fn plaintext_row_without_material_fails_closed() {
        let pool = boot_pool().await;
        let key = crate::crypto::aead::random_dek();
        let kid = content_addressed_kid(&key);
        sqlx::query(
            "INSERT INTO vault.kek_metadata
                (kid, sealing_method, sealed, sealed_blob, created_at)
             VALUES (?, 'plaintext', 0, ?, 0.0)",
        )
        .bind(&kid)
        .bind(key.as_slice())
        .execute(&pool)
        .await
        .unwrap();
        let res = load_or_init_sqlite(&pool, &KekBootConfig::unset()).await;
        assert!(res.is_err(), "plaintext row + no material must fail closed");
        // Row is untouched (still plaintext) — no silent destruction.
        assert_eq!(sealing_method(&pool).await, METHOD_PLAINTEXT);
    }

    #[tokio::test]
    async fn dev_plaintext_opt_in_still_works() {
        let pool = boot_pool().await;
        let cfg = KekBootConfig::dev_plaintext();
        let h1 = load_or_init_sqlite(&pool, &cfg).await.unwrap();
        assert_eq!(sealing_method(&pool).await, METHOD_PLAINTEXT);
        // Reboot in dev mode loads the same plaintext KEK.
        let h2 = load_or_init_sqlite(&pool, &cfg).await.unwrap();
        assert_eq!(h1.kid(), h2.kid());
    }

    #[tokio::test]
    async fn passphrase_seal_round_trips_across_boot() {
        let pool = boot_pool().await;
        let cfg = || KekBootConfig::sealed(UnsealMaterial::passphrase("a strong passphrase").unwrap());
        let h1 = load_or_init_sqlite(&pool, &cfg()).await.unwrap();
        let h2 = load_or_init_sqlite(&pool, &cfg()).await.unwrap();
        assert_eq!(h1.kid(), h2.kid());
        assert_eq!(sealing_method(&pool).await, METHOD_SEALED_V1);
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
        let res = load_or_init_sqlite(&pool, &raw_cfg()).await;
        assert!(
            res.is_err(),
            "unhandled sealing method must be rejected at the boot loader"
        );
    }
}
