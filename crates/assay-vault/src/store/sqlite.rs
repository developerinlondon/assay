//! SQLite backend for the vault module.
//!
//! Phase 0: schema migration. Phase 1: [`SqliteKvStore`] for KV v2.
//!
//! The caller is expected to have ATTACHed `data/vault.db` AS `vault`
//! before invoking any of this — mirrors the wiring `assay-auth` and
//! `assay-workflow` already rely on for their attached databases.

use anyhow::Result;
use async_trait::async_trait;
use sqlx::SqlitePool;

/// Apply the vault-schema DDL idempotently. Caller must have already
/// ATTACHed the vault database as `vault`.
pub async fn migrate(pool: &SqlitePool) -> Result<()> {
    crate::schema::migrate_sqlite(pool).await
}

#[cfg(feature = "vault-kv")]
mod kv {
    use super::*;
    use crate::error::{Result as VaultResult, VaultError};
    use crate::kv::{KvMeta, KvRow, KvStore};
    use serde_json::Value;

    /// SQLite-backed KV store.
    #[derive(Clone)]
    pub struct SqliteKvStore {
        pool: SqlitePool,
    }

    impl SqliteKvStore {
        pub fn new(pool: SqlitePool) -> Self {
            Self { pool }
        }
    }

    fn map_err(ctx: &'static str) -> impl FnOnce(sqlx::Error) -> VaultError {
        move |e| VaultError::Backend(anyhow::anyhow!("{ctx}: {e}"))
    }

    fn unix_now() -> f64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64()
    }

    fn parse_md(s: &str) -> Value {
        serde_json::from_str(s).unwrap_or_else(|_| Value::Object(Default::default()))
    }

    #[async_trait]
    impl KvStore for SqliteKvStore {
        async fn put_row(
            &self,
            path: &str,
            ciphertext: &[u8],
            nonce: &[u8],
            wrapped_dek: &[u8],
            kek_kid: &str,
            custom_md: &Value,
        ) -> VaultResult<i64> {
            let mut tx = self.pool.begin().await.map_err(map_err("kv put begin"))?;
            let now = unix_now();
            let md_str =
                serde_json::to_string(custom_md).unwrap_or_else(|_| "{}".to_string());

            // SQLite lacks JSONB merge, so do the merge in two steps.
            // Read the current md (if any), merge, UPSERT the bumped row,
            // RETURNING the new latest_version.
            let existing_md: Option<(String,)> =
                sqlx::query_as("SELECT custom_md FROM vault.kv_meta WHERE path = ?")
                    .bind(path)
                    .fetch_optional(&mut *tx)
                    .await
                    .map_err(map_err("kv put read existing meta"))?;

            let merged_md = match existing_md {
                Some((s,)) if !custom_md.is_object()
                    || custom_md
                        .as_object()
                        .map(|o| o.is_empty())
                        .unwrap_or(true) =>
                {
                    s
                }
                Some((s,)) => merge_json(&s, &md_str),
                None => md_str,
            };

            let new_version: i64 = sqlx::query_scalar(
                "INSERT INTO vault.kv_meta (path, latest_version, custom_md, created_at, updated_at)
                 VALUES (?, 1, ?, ?, ?)
                 ON CONFLICT (path) DO UPDATE
                   SET latest_version = latest_version + 1,
                       custom_md = excluded.custom_md,
                       updated_at = excluded.updated_at
                 RETURNING latest_version",
            )
            .bind(path)
            .bind(merged_md)
            .bind(now)
            .bind(now)
            .fetch_one(&mut *tx)
            .await
            .map_err(map_err("kv put upsert meta"))?;

            sqlx::query(
                "INSERT INTO vault.kv
                    (path, version, ciphertext, nonce, wrapped_dek, kek_kid, created_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(path)
            .bind(new_version)
            .bind(ciphertext)
            .bind(nonce)
            .bind(wrapped_dek)
            .bind(kek_kid)
            .bind(now)
            .execute(&mut *tx)
            .await
            .map_err(map_err("kv put insert row"))?;

            tx.commit().await.map_err(map_err("kv put commit"))?;
            Ok(new_version)
        }

        async fn get_row(&self, path: &str, version: i64) -> VaultResult<Option<KvRow>> {
            let row: Option<(Vec<u8>, Vec<u8>, Vec<u8>, String, Option<f64>, i64, f64)> =
                sqlx::query_as(
                    "SELECT ciphertext, nonce, wrapped_dek, kek_kid, deleted_at, destroyed, created_at
                       FROM vault.kv
                      WHERE path = ? AND version = ?",
                )
                .bind(path)
                .bind(version)
                .fetch_optional(&self.pool)
                .await
                .map_err(map_err("kv get_row"))?;
            Ok(row.map(|(ct, n, wd, kk, da, dst, ca)| KvRow {
                path: path.to_string(),
                version,
                ciphertext: ct,
                nonce: n,
                wrapped_dek: wd,
                kek_kid: kk,
                deleted_at: da,
                destroyed: dst != 0,
                created_at: ca,
            }))
        }

        async fn get_latest_row(&self, path: &str) -> VaultResult<Option<KvRow>> {
            let row: Option<(i64, Vec<u8>, Vec<u8>, Vec<u8>, String, Option<f64>, i64, f64)> =
                sqlx::query_as(
                    "SELECT version, ciphertext, nonce, wrapped_dek, kek_kid, deleted_at, destroyed, created_at
                       FROM vault.kv
                      WHERE path = ?
                        AND destroyed = 0
                      ORDER BY version DESC
                      LIMIT 1",
                )
                .bind(path)
                .fetch_optional(&self.pool)
                .await
                .map_err(map_err("kv get_latest_row"))?;
            Ok(row.map(|(v, ct, n, wd, kk, da, dst, ca)| KvRow {
                path: path.to_string(),
                version: v,
                ciphertext: ct,
                nonce: n,
                wrapped_dek: wd,
                kek_kid: kk,
                deleted_at: da,
                destroyed: dst != 0,
                created_at: ca,
            }))
        }

        async fn list_meta(&self, prefix: &str) -> VaultResult<Vec<KvMeta>> {
            // SQLite LIKE escaping: '%' / '_' become wildcards. Escape
            // them with '\\' (specified via ESCAPE).
            let pattern = format!("{}%", prefix.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_"));
            let rows: Vec<(String, i64, String, f64, f64)> = sqlx::query_as(
                "SELECT path, latest_version, custom_md, created_at, updated_at
                   FROM vault.kv_meta
                  WHERE path LIKE ? ESCAPE '\\'
                  ORDER BY path",
            )
            .bind(pattern)
            .fetch_all(&self.pool)
            .await
            .map_err(map_err("kv list_meta"))?;
            Ok(rows
                .into_iter()
                .map(|(path, latest_version, md_str, ca, ua)| KvMeta {
                    path,
                    latest_version,
                    custom_md: parse_md(&md_str),
                    created_at: ca,
                    updated_at: ua,
                })
                .collect())
        }

        async fn read_meta(&self, path: &str) -> VaultResult<Option<KvMeta>> {
            let row: Option<(i64, String, f64, f64)> = sqlx::query_as(
                "SELECT latest_version, custom_md, created_at, updated_at
                   FROM vault.kv_meta
                  WHERE path = ?",
            )
            .bind(path)
            .fetch_optional(&self.pool)
            .await
            .map_err(map_err("kv read_meta"))?;
            Ok(row.map(|(lv, md_str, ca, ua)| KvMeta {
                path: path.to_string(),
                latest_version: lv,
                custom_md: parse_md(&md_str),
                created_at: ca,
                updated_at: ua,
            }))
        }

        async fn soft_delete(&self, path: &str, version: i64, deleted_at: f64) -> VaultResult<bool> {
            let n = sqlx::query(
                "UPDATE vault.kv
                    SET deleted_at = ?
                  WHERE path = ?
                    AND version = ?
                    AND destroyed = 0
                    AND deleted_at IS NULL",
            )
            .bind(deleted_at)
            .bind(path)
            .bind(version)
            .execute(&self.pool)
            .await
            .map_err(map_err("kv soft_delete"))?
            .rows_affected();
            Ok(n > 0)
        }

        async fn destroy(&self, path: &str, version: i64) -> VaultResult<bool> {
            let n = sqlx::query(
                "UPDATE vault.kv
                    SET destroyed = 1,
                        ciphertext = x'',
                        wrapped_dek = x''
                  WHERE path = ? AND version = ? AND destroyed = 0",
            )
            .bind(path)
            .bind(version)
            .execute(&self.pool)
            .await
            .map_err(map_err("kv destroy"))?
            .rows_affected();
            Ok(n > 0)
        }

        async fn undelete(&self, path: &str, version: i64) -> VaultResult<bool> {
            let n = sqlx::query(
                "UPDATE vault.kv
                    SET deleted_at = NULL
                  WHERE path = ?
                    AND version = ?
                    AND destroyed = 0
                    AND deleted_at IS NOT NULL",
            )
            .bind(path)
            .bind(version)
            .execute(&self.pool)
            .await
            .map_err(map_err("kv undelete"))?
            .rows_affected();
            Ok(n > 0)
        }
    }

    /// Shallow object-merge of two JSON strings — overlay's keys win.
    /// Falls back to overlay-only on parse failure (i.e. badly-formed
    /// existing md is replaced rather than blocking the write).
    fn merge_json(base: &str, overlay: &str) -> String {
        let base_v: Value = serde_json::from_str(base).unwrap_or(Value::Null);
        let over_v: Value = serde_json::from_str(overlay).unwrap_or(Value::Null);
        match (base_v, over_v) {
            (Value::Object(mut b), Value::Object(o)) => {
                for (k, v) in o {
                    b.insert(k, v);
                }
                serde_json::to_string(&Value::Object(b)).unwrap_or_else(|_| overlay.to_string())
            }
            (_, over) => serde_json::to_string(&over).unwrap_or_else(|_| overlay.to_string()),
        }
    }
}

#[cfg(feature = "vault-kv")]
pub use kv::SqliteKvStore;

#[cfg(feature = "vault-transit")]
mod transit {
    use super::*;
    use crate::error::{Result as VaultResult, VaultError};
    use crate::transit::{TransitKey, TransitStore, TransitVersion};

    /// SQLite-backed transit store.
    #[derive(Clone)]
    pub struct SqliteTransitStore {
        pool: SqlitePool,
    }

    impl SqliteTransitStore {
        pub fn new(pool: SqlitePool) -> Self {
            Self { pool }
        }
    }

    fn map_err(ctx: &'static str) -> impl FnOnce(sqlx::Error) -> VaultError {
        move |e| VaultError::Backend(anyhow::anyhow!("{ctx}: {e}"))
    }

    fn unix_now() -> f64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64()
    }

    #[async_trait]
    impl TransitStore for SqliteTransitStore {
        async fn create_key(
            &self,
            name: &str,
            algo: &str,
            version_wrapped: &[u8],
            kek_kid: &str,
        ) -> VaultResult<()> {
            let mut tx = self.pool.begin().await.map_err(map_err("transit create begin"))?;
            let now = unix_now();

            let res = sqlx::query(
                "INSERT INTO vault.transit_keys (name, latest_ver, algo, created_at)
                 VALUES (?, 1, ?, ?)",
            )
            .bind(name)
            .bind(algo)
            .bind(now)
            .execute(&mut *tx)
            .await;
            if let Err(sqlx::Error::Database(dberr)) = &res
                && dberr.message().contains("UNIQUE")
            {
                return Err(VaultError::Conflict(format!(
                    "transit key '{name}' already exists"
                )));
            }
            res.map_err(map_err("transit create insert key"))?;

            sqlx::query(
                "INSERT INTO vault.transit_versions (name, version, key_wrapped, kek_kid, created_at)
                 VALUES (?, 1, ?, ?, ?)",
            )
            .bind(name)
            .bind(version_wrapped)
            .bind(kek_kid)
            .bind(now)
            .execute(&mut *tx)
            .await
            .map_err(map_err("transit create insert version"))?;

            tx.commit().await.map_err(map_err("transit create commit"))?;
            Ok(())
        }

        async fn get_key(&self, name: &str) -> VaultResult<Option<TransitKey>> {
            let row: Option<(String, i64, f64)> = sqlx::query_as(
                "SELECT algo, latest_ver, created_at FROM vault.transit_keys WHERE name = ?",
            )
            .bind(name)
            .fetch_optional(&self.pool)
            .await
            .map_err(map_err("transit get_key"))?;
            Ok(row.map(|(algo, lv, ca)| TransitKey {
                name: name.to_string(),
                algo,
                latest_ver: lv,
                created_at: ca,
            }))
        }

        async fn get_version(&self, name: &str, version: i64) -> VaultResult<Option<TransitVersion>> {
            let row: Option<(Vec<u8>, String, f64)> = sqlx::query_as(
                "SELECT key_wrapped, kek_kid, created_at
                   FROM vault.transit_versions
                  WHERE name = ? AND version = ?",
            )
            .bind(name)
            .bind(version)
            .fetch_optional(&self.pool)
            .await
            .map_err(map_err("transit get_version"))?;
            Ok(row.map(|(kw, kk, ca)| TransitVersion {
                name: name.to_string(),
                version,
                key_wrapped: kw,
                kek_kid: kk,
                created_at: ca,
            }))
        }

        async fn get_latest_version(&self, name: &str) -> VaultResult<Option<TransitVersion>> {
            let lv: Option<i64> = sqlx::query_scalar(
                "SELECT latest_ver FROM vault.transit_keys WHERE name = ?",
            )
            .bind(name)
            .fetch_optional(&self.pool)
            .await
            .map_err(map_err("transit get_latest version-ptr"))?;
            match lv {
                None => Ok(None),
                Some(v) => self.get_version(name, v).await,
            }
        }

        async fn rotate(&self, name: &str, version_wrapped: &[u8], kek_kid: &str) -> VaultResult<i64> {
            let mut tx = self.pool.begin().await.map_err(map_err("transit rotate begin"))?;
            let new_ver: Option<i64> = sqlx::query_scalar(
                "UPDATE vault.transit_keys
                    SET latest_ver = latest_ver + 1
                  WHERE name = ?
                  RETURNING latest_ver",
            )
            .bind(name)
            .fetch_optional(&mut *tx)
            .await
            .map_err(map_err("transit rotate bump"))?;
            let new_ver = new_ver.ok_or(VaultError::NotFound)?;
            let now = unix_now();
            sqlx::query(
                "INSERT INTO vault.transit_versions (name, version, key_wrapped, kek_kid, created_at)
                 VALUES (?, ?, ?, ?, ?)",
            )
            .bind(name)
            .bind(new_ver)
            .bind(version_wrapped)
            .bind(kek_kid)
            .bind(now)
            .execute(&mut *tx)
            .await
            .map_err(map_err("transit rotate insert version"))?;
            tx.commit().await.map_err(map_err("transit rotate commit"))?;
            Ok(new_ver)
        }

        async fn list_keys(&self) -> VaultResult<Vec<TransitKey>> {
            let rows: Vec<(String, String, i64, f64)> = sqlx::query_as(
                "SELECT name, algo, latest_ver, created_at
                   FROM vault.transit_keys
                  ORDER BY name",
            )
            .fetch_all(&self.pool)
            .await
            .map_err(map_err("transit list_keys"))?;
            Ok(rows
                .into_iter()
                .map(|(n, a, lv, ca)| TransitKey {
                    name: n,
                    algo: a,
                    latest_ver: lv,
                    created_at: ca,
                })
                .collect())
        }
    }
}

#[cfg(feature = "vault-transit")]
pub use transit::SqliteTransitStore;

#[cfg(feature = "vault-sealing-shamir")]
mod sealing {
    use super::*;
    use crate::crypto::sealing::SealStore;
    use crate::error::{Result as VaultResult, VaultError};

    /// SQLite-backed [`SealStore`] — delegates to the
    /// [`crate::crypto::kek_store`] helpers.
    #[derive(Clone)]
    pub struct SqliteSealStore {
        pool: SqlitePool,
    }

    impl SqliteSealStore {
        pub fn new(pool: SqlitePool) -> Self {
            Self { pool }
        }
    }

    #[async_trait]
    impl SealStore for SqliteSealStore {
        async fn init_shamir(
            &self,
            threshold: u8,
            shares_count: u8,
        ) -> VaultResult<(String, Vec<Vec<u8>>)> {
            let (kid, shares) = crate::crypto::kek_store::init_shamir_sqlite(
                &self.pool,
                threshold,
                shares_count,
            )
            .await
            .map_err(|e| VaultError::Backend(anyhow::anyhow!("seal init_shamir: {e}")))?;
            Ok((kid, shares.into_iter().map(|s| s.0).collect()))
        }

        async fn set_sealed(&self, kid: &str, sealed: bool) -> VaultResult<()> {
            crate::crypto::kek_store::set_sealed_flag_sqlite(&self.pool, kid, sealed)
                .await
                .map_err(|e| VaultError::Backend(anyhow::anyhow!("set_sealed: {e}")))
        }
    }
}

#[cfg(feature = "vault-sealing-shamir")]
pub use sealing::SqliteSealStore;

#[cfg(feature = "vault-collections")]
mod personal_vault {
    use super::*;
    use crate::error::{Result as VaultResult, VaultError};
    use crate::personal_vault::{PersonalVault, PersonalVaultStore};

    #[derive(Clone)]
    pub struct SqlitePersonalVaultStore {
        pool: SqlitePool,
    }

    impl SqlitePersonalVaultStore {
        pub fn new(pool: SqlitePool) -> Self {
            Self { pool }
        }
    }

    fn map_err(ctx: &'static str) -> impl FnOnce(sqlx::Error) -> VaultError {
        move |e| VaultError::Backend(anyhow::anyhow!("{ctx}: {e}"))
    }

    fn unix_now() -> f64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64()
    }

    #[async_trait]
    impl PersonalVaultStore for SqlitePersonalVaultStore {
        async fn ensure_vault(
            &self,
            id: &str,
            owner_user: &str,
            public_key: &[u8],
        ) -> VaultResult<PersonalVault> {
            sqlx::query(
                "INSERT INTO vault.vaults (id, owner_user, public_key, created_at)
                 VALUES (?, ?, ?, ?)
                 ON CONFLICT (owner_user) DO NOTHING",
            )
            .bind(id)
            .bind(owner_user)
            .bind(public_key)
            .bind(unix_now())
            .execute(&self.pool)
            .await
            .map_err(map_err("ensure_vault insert"))?;

            self.get_by_owner(owner_user)
                .await?
                .ok_or_else(|| VaultError::Backend(anyhow::anyhow!("vault row missing post-insert")))
        }

        async fn get_by_owner(&self, owner_user: &str) -> VaultResult<Option<PersonalVault>> {
            let row: Option<(String, Vec<u8>, f64)> = sqlx::query_as(
                "SELECT id, public_key, created_at FROM vault.vaults WHERE owner_user = ?",
            )
            .bind(owner_user)
            .fetch_optional(&self.pool)
            .await
            .map_err(map_err("get_by_owner"))?;
            Ok(row.map(|(id, pk, ca)| PersonalVault {
                id,
                owner_user: owner_user.to_string(),
                public_key: pk,
                created_at: ca,
            }))
        }

        async fn get_by_id(&self, id: &str) -> VaultResult<Option<PersonalVault>> {
            let row: Option<(String, Vec<u8>, f64)> = sqlx::query_as(
                "SELECT owner_user, public_key, created_at FROM vault.vaults WHERE id = ?",
            )
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(map_err("get_by_id"))?;
            Ok(row.map(|(o, pk, ca)| PersonalVault {
                id: id.to_string(),
                owner_user: o,
                public_key: pk,
                created_at: ca,
            }))
        }

        async fn rotate_public_key(
            &self,
            owner_user: &str,
            new_public_key: &[u8],
        ) -> VaultResult<bool> {
            let n = sqlx::query(
                "UPDATE vault.vaults SET public_key = ? WHERE owner_user = ?",
            )
            .bind(new_public_key)
            .bind(owner_user)
            .execute(&self.pool)
            .await
            .map_err(map_err("rotate_public_key"))?
            .rows_affected();
            Ok(n > 0)
        }
    }
}

#[cfg(feature = "vault-collections")]
pub use personal_vault::SqlitePersonalVaultStore;

#[cfg(feature = "vault-collections")]
mod collections {
    use super::*;
    use crate::collections::{Collection, CollectionMember, CollectionStore};
    use crate::error::{Result as VaultResult, VaultError};

    #[derive(Clone)]
    pub struct SqliteCollectionStore {
        pool: SqlitePool,
    }

    impl SqliteCollectionStore {
        pub fn new(pool: SqlitePool) -> Self {
            Self { pool }
        }
    }

    fn map_err(ctx: &'static str) -> impl FnOnce(sqlx::Error) -> VaultError {
        move |e| VaultError::Backend(anyhow::anyhow!("{ctx}: {e}"))
    }

    fn unix_now() -> f64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64()
    }

    #[async_trait]
    impl CollectionStore for SqliteCollectionStore {
        async fn create_collection(
            &self,
            id: &str,
            org_id: Option<&str>,
            name: &str,
            created_by: &str,
        ) -> VaultResult<Collection> {
            let res = sqlx::query(
                "INSERT INTO vault.collections (id, org_id, name, created_by, created_at)
                 VALUES (?, ?, ?, ?, ?)",
            )
            .bind(id)
            .bind(org_id)
            .bind(name)
            .bind(created_by)
            .bind(unix_now())
            .execute(&self.pool)
            .await;
            if let Err(sqlx::Error::Database(dberr)) = &res
                && dberr.message().contains("UNIQUE")
            {
                return Err(VaultError::Conflict(format!(
                    "collection id '{id}' already exists"
                )));
            }
            res.map_err(map_err("create_collection"))?;
            self.get_collection(id)
                .await?
                .ok_or_else(|| VaultError::Backend(anyhow::anyhow!("collection missing post-insert")))
        }

        async fn get_collection(&self, id: &str) -> VaultResult<Option<Collection>> {
            let row: Option<(Option<String>, String, String, f64)> = sqlx::query_as(
                "SELECT org_id, name, created_by, created_at
                   FROM vault.collections WHERE id = ?",
            )
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(map_err("get_collection"))?;
            Ok(row.map(|(org, name, by, ca)| Collection {
                id: id.to_string(),
                org_id: org,
                name,
                created_by: by,
                created_at: ca,
            }))
        }

        async fn list_collections(
            &self,
            org_id: Option<&str>,
        ) -> VaultResult<Vec<Collection>> {
            let rows: Vec<(String, Option<String>, String, String, f64)> = match org_id {
                Some(o) => sqlx::query_as(
                    "SELECT id, org_id, name, created_by, created_at
                       FROM vault.collections WHERE org_id = ? ORDER BY name",
                )
                .bind(o)
                .fetch_all(&self.pool)
                .await,
                None => sqlx::query_as(
                    "SELECT id, org_id, name, created_by, created_at
                       FROM vault.collections ORDER BY name",
                )
                .fetch_all(&self.pool)
                .await,
            }
            .map_err(map_err("list_collections"))?;
            Ok(rows
                .into_iter()
                .map(|(id, org, name, by, ca)| Collection {
                    id,
                    org_id: org,
                    name,
                    created_by: by,
                    created_at: ca,
                })
                .collect())
        }

        async fn delete_collection(&self, id: &str) -> VaultResult<bool> {
            let n = sqlx::query("DELETE FROM vault.collections WHERE id = ?")
                .bind(id)
                .execute(&self.pool)
                .await
                .map_err(map_err("delete_collection"))?
                .rows_affected();
            Ok(n > 0)
        }

        async fn upsert_member(
            &self,
            collection_id: &str,
            user_id: &str,
            wrapped_key: &[u8],
            role: &str,
        ) -> VaultResult<()> {
            sqlx::query(
                "INSERT INTO vault.collection_members
                    (collection_id, user_id, wrapped_key, role, added_at)
                 VALUES (?, ?, ?, ?, ?)
                 ON CONFLICT (collection_id, user_id) DO UPDATE
                   SET wrapped_key = excluded.wrapped_key,
                       role        = excluded.role",
            )
            .bind(collection_id)
            .bind(user_id)
            .bind(wrapped_key)
            .bind(role)
            .bind(unix_now())
            .execute(&self.pool)
            .await
            .map_err(map_err("upsert_member"))?;
            Ok(())
        }

        async fn list_members(
            &self,
            collection_id: &str,
        ) -> VaultResult<Vec<CollectionMember>> {
            let rows: Vec<(String, Vec<u8>, String, f64)> = sqlx::query_as(
                "SELECT user_id, wrapped_key, role, added_at
                   FROM vault.collection_members
                  WHERE collection_id = ?
                  ORDER BY added_at",
            )
            .bind(collection_id)
            .fetch_all(&self.pool)
            .await
            .map_err(map_err("list_members"))?;
            Ok(rows
                .into_iter()
                .map(|(uid, wk, role, at)| CollectionMember {
                    collection_id: collection_id.to_string(),
                    user_id: uid,
                    wrapped_key: wk,
                    role,
                    added_at: at,
                })
                .collect())
        }

        async fn remove_member(
            &self,
            collection_id: &str,
            user_id: &str,
        ) -> VaultResult<bool> {
            let n = sqlx::query(
                "DELETE FROM vault.collection_members
                  WHERE collection_id = ? AND user_id = ?",
            )
            .bind(collection_id)
            .bind(user_id)
            .execute(&self.pool)
            .await
            .map_err(map_err("remove_member"))?
            .rows_affected();
            Ok(n > 0)
        }

        async fn is_member(
            &self,
            collection_id: &str,
            user_id: &str,
        ) -> VaultResult<bool> {
            let row: Option<(i64,)> = sqlx::query_as(
                "SELECT 1 FROM vault.collection_members
                  WHERE collection_id = ? AND user_id = ?",
            )
            .bind(collection_id)
            .bind(user_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(map_err("is_member"))?;
            Ok(row.is_some())
        }
    }
}

#[cfg(feature = "vault-collections")]
pub use collections::SqliteCollectionStore;
