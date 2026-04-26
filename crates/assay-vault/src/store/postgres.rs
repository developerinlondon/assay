//! Postgres backend for the vault module.
//!
//! Phase 0: schema migration.
//! Phase 1: [`PgKvStore`] for KV v2 (this commit).

use anyhow::Result;
use async_trait::async_trait;
use sqlx::PgPool;

/// Apply the vault-schema DDL idempotently. Called by the engine boot
/// path when the `vault` module is enabled.
pub async fn migrate(pool: &PgPool) -> Result<()> {
    crate::schema::migrate_postgres(pool).await
}

#[cfg(feature = "vault-kv")]
mod kv {
    use super::*;
    use crate::error::{Result as VaultResult, VaultError};
    use crate::kv::{KvMeta, KvRow, KvStore};
    use serde_json::Value;

    /// Postgres-backed KV store. Cheap to clone — wraps a `sqlx::PgPool`.
    #[derive(Clone)]
    pub struct PgKvStore {
        pool: PgPool,
    }

    impl PgKvStore {
        pub fn new(pool: PgPool) -> Self {
            Self { pool }
        }
    }

    fn map_err(ctx: &'static str) -> impl FnOnce(sqlx::Error) -> VaultError {
        move |e| VaultError::Backend(anyhow::anyhow!("{ctx}: {e}"))
    }

    #[async_trait]
    impl KvStore for PgKvStore {
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

            // UPSERT meta — bump latest_version atomically and merge
            // custom_md. `custom_md = '{}'` keeps the existing row's md
            // so a vanilla PUT with no metadata is non-destructive.
            let new_version: i64 = sqlx::query_scalar(
                "INSERT INTO vault.kv_meta (path, latest_version, custom_md, created_at, updated_at)
                 VALUES ($1, 1, $2, EXTRACT(EPOCH FROM NOW()), EXTRACT(EPOCH FROM NOW()))
                 ON CONFLICT (path) DO UPDATE
                   SET latest_version = vault.kv_meta.latest_version + 1,
                       custom_md = CASE
                                     WHEN $2::jsonb = '{}'::jsonb THEN vault.kv_meta.custom_md
                                     ELSE vault.kv_meta.custom_md || $2::jsonb
                                   END,
                       updated_at = EXTRACT(EPOCH FROM NOW())
                 RETURNING latest_version",
            )
            .bind(path)
            .bind(custom_md)
            .fetch_one(&mut *tx)
            .await
            .map_err(map_err("kv put upsert meta"))?;

            sqlx::query(
                "INSERT INTO vault.kv
                    (path, version, ciphertext, nonce, wrapped_dek, kek_kid)
                 VALUES ($1, $2, $3, $4, $5, $6)",
            )
            .bind(path)
            .bind(new_version)
            .bind(ciphertext)
            .bind(nonce)
            .bind(wrapped_dek)
            .bind(kek_kid)
            .execute(&mut *tx)
            .await
            .map_err(map_err("kv put insert row"))?;

            tx.commit().await.map_err(map_err("kv put commit"))?;
            Ok(new_version)
        }

        async fn get_row(&self, path: &str, version: i64) -> VaultResult<Option<KvRow>> {
            let row: Option<(Vec<u8>, Vec<u8>, Vec<u8>, String, Option<f64>, bool, f64)> =
                sqlx::query_as(
                    "SELECT ciphertext, nonce, wrapped_dek, kek_kid, deleted_at, destroyed, created_at
                       FROM vault.kv
                      WHERE path = $1 AND version = $2",
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
                destroyed: dst,
                created_at: ca,
            }))
        }

        async fn get_latest_row(&self, path: &str) -> VaultResult<Option<KvRow>> {
            let row: Option<(i64, Vec<u8>, Vec<u8>, Vec<u8>, String, Option<f64>, bool, f64)> =
                sqlx::query_as(
                    "SELECT version, ciphertext, nonce, wrapped_dek, kek_kid, deleted_at, destroyed, created_at
                       FROM vault.kv
                      WHERE path = $1
                        AND destroyed = FALSE
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
                destroyed: dst,
                created_at: ca,
            }))
        }

        async fn list_meta(&self, prefix: &str) -> VaultResult<Vec<KvMeta>> {
            let pattern = format!("{}%", prefix.replace('%', "\\%").replace('_', "\\_"));
            let rows: Vec<(String, i64, Value, f64, f64)> = sqlx::query_as(
                "SELECT path, latest_version, custom_md, created_at, updated_at
                   FROM vault.kv_meta
                  WHERE path LIKE $1 ESCAPE '\\'
                  ORDER BY path",
            )
            .bind(pattern)
            .fetch_all(&self.pool)
            .await
            .map_err(map_err("kv list_meta"))?;
            Ok(rows
                .into_iter()
                .map(|(path, latest_version, custom_md, created_at, updated_at)| KvMeta {
                    path,
                    latest_version,
                    custom_md,
                    created_at,
                    updated_at,
                })
                .collect())
        }

        async fn read_meta(&self, path: &str) -> VaultResult<Option<KvMeta>> {
            let row: Option<(i64, Value, f64, f64)> = sqlx::query_as(
                "SELECT latest_version, custom_md, created_at, updated_at
                   FROM vault.kv_meta
                  WHERE path = $1",
            )
            .bind(path)
            .fetch_optional(&self.pool)
            .await
            .map_err(map_err("kv read_meta"))?;
            Ok(row.map(|(lv, md, ca, ua)| KvMeta {
                path: path.to_string(),
                latest_version: lv,
                custom_md: md,
                created_at: ca,
                updated_at: ua,
            }))
        }

        async fn soft_delete(&self, path: &str, version: i64, deleted_at: f64) -> VaultResult<bool> {
            let n = sqlx::query(
                "UPDATE vault.kv
                    SET deleted_at = $3
                  WHERE path = $1
                    AND version = $2
                    AND destroyed = FALSE
                    AND deleted_at IS NULL",
            )
            .bind(path)
            .bind(version)
            .bind(deleted_at)
            .execute(&self.pool)
            .await
            .map_err(map_err("kv soft_delete"))?
            .rows_affected();
            Ok(n > 0)
        }

        async fn destroy(&self, path: &str, version: i64) -> VaultResult<bool> {
            let n = sqlx::query(
                "UPDATE vault.kv
                    SET destroyed = TRUE,
                        ciphertext = ''::bytea,
                        wrapped_dek = ''::bytea
                  WHERE path = $1 AND version = $2 AND destroyed = FALSE",
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
                  WHERE path = $1
                    AND version = $2
                    AND destroyed = FALSE
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
}

#[cfg(feature = "vault-kv")]
pub use kv::PgKvStore;

#[cfg(feature = "vault-transit")]
mod transit {
    use super::*;
    use crate::error::{Result as VaultResult, VaultError};
    use crate::transit::{TransitKey, TransitStore, TransitVersion};

    /// Postgres-backed transit store.
    #[derive(Clone)]
    pub struct PgTransitStore {
        pool: PgPool,
    }

    impl PgTransitStore {
        pub fn new(pool: PgPool) -> Self {
            Self { pool }
        }
    }

    fn map_err(ctx: &'static str) -> impl FnOnce(sqlx::Error) -> VaultError {
        move |e| VaultError::Backend(anyhow::anyhow!("{ctx}: {e}"))
    }

    #[async_trait]
    impl TransitStore for PgTransitStore {
        async fn create_key(
            &self,
            name: &str,
            algo: &str,
            version_wrapped: &[u8],
            kek_kid: &str,
        ) -> VaultResult<()> {
            let mut tx = self.pool.begin().await.map_err(map_err("transit create begin"))?;

            // Strict create: 23505 (unique violation) → Conflict; everything
            // else surfaces as Backend.
            let res = sqlx::query(
                "INSERT INTO vault.transit_keys (name, latest_ver, algo, created_at)
                 VALUES ($1, 1, $2, EXTRACT(EPOCH FROM NOW()))",
            )
            .bind(name)
            .bind(algo)
            .execute(&mut *tx)
            .await;
            if let Err(sqlx::Error::Database(dberr)) = &res
                && dberr.code().as_deref() == Some("23505")
            {
                return Err(VaultError::Conflict(format!(
                    "transit key '{name}' already exists"
                )));
            }
            res.map_err(map_err("transit create insert key"))?;

            sqlx::query(
                "INSERT INTO vault.transit_versions (name, version, key_wrapped, kek_kid)
                 VALUES ($1, 1, $2, $3)",
            )
            .bind(name)
            .bind(version_wrapped)
            .bind(kek_kid)
            .execute(&mut *tx)
            .await
            .map_err(map_err("transit create insert version"))?;

            tx.commit().await.map_err(map_err("transit create commit"))?;
            Ok(())
        }

        async fn get_key(&self, name: &str) -> VaultResult<Option<TransitKey>> {
            let row: Option<(String, i64, f64)> = sqlx::query_as(
                "SELECT algo, latest_ver, created_at FROM vault.transit_keys WHERE name = $1",
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
                  WHERE name = $1 AND version = $2",
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
            // Two-step (read latest_ver, fetch row) keeps the SELECT
            // simple and lets sqlx infer types cleanly. The transit_keys
            // row is the source of truth for "which version is latest".
            let lv: Option<i64> = sqlx::query_scalar(
                "SELECT latest_ver FROM vault.transit_keys WHERE name = $1",
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
            let new_ver: i64 = sqlx::query_scalar(
                "UPDATE vault.transit_keys
                    SET latest_ver = latest_ver + 1
                  WHERE name = $1
                  RETURNING latest_ver",
            )
            .bind(name)
            .fetch_optional(&mut *tx)
            .await
            .map_err(map_err("transit rotate bump"))?
            .ok_or(VaultError::NotFound)?;
            sqlx::query(
                "INSERT INTO vault.transit_versions (name, version, key_wrapped, kek_kid)
                 VALUES ($1, $2, $3, $4)",
            )
            .bind(name)
            .bind(new_ver)
            .bind(version_wrapped)
            .bind(kek_kid)
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
pub use transit::PgTransitStore;

#[cfg(feature = "vault-sealing-shamir")]
mod sealing {
    use super::*;
    use crate::crypto::sealing::SealStore;
    use crate::error::{Result as VaultResult, VaultError};

    /// Postgres-backed [`SealStore`] — delegates to the
    /// [`crate::crypto::kek_store`] helpers.
    #[derive(Clone)]
    pub struct PgSealStore {
        pool: PgPool,
    }

    impl PgSealStore {
        pub fn new(pool: PgPool) -> Self {
            Self { pool }
        }
    }

    #[async_trait]
    impl SealStore for PgSealStore {
        async fn init_shamir(
            &self,
            threshold: u8,
            shares_count: u8,
        ) -> VaultResult<(String, Vec<Vec<u8>>)> {
            let (kid, shares) = crate::crypto::kek_store::init_shamir_postgres(
                &self.pool,
                threshold,
                shares_count,
            )
            .await
            .map_err(|e| VaultError::Backend(anyhow::anyhow!("seal init_shamir: {e}")))?;
            Ok((kid, shares.into_iter().map(|s| s.0).collect()))
        }

        async fn set_sealed(&self, kid: &str, sealed: bool) -> VaultResult<()> {
            crate::crypto::kek_store::set_sealed_flag_postgres(&self.pool, kid, sealed)
                .await
                .map_err(|e| VaultError::Backend(anyhow::anyhow!("set_sealed: {e}")))
        }
    }
}

#[cfg(feature = "vault-sealing-shamir")]
pub use sealing::PgSealStore;

#[cfg(feature = "vault-collections")]
mod personal_vault {
    use super::*;
    use crate::error::{Result as VaultResult, VaultError};
    use crate::personal_vault::{PersonalVault, PersonalVaultStore};

    #[derive(Clone)]
    pub struct PgPersonalVaultStore {
        pool: PgPool,
    }

    impl PgPersonalVaultStore {
        pub fn new(pool: PgPool) -> Self {
            Self { pool }
        }
    }

    fn map_err(ctx: &'static str) -> impl FnOnce(sqlx::Error) -> VaultError {
        move |e| VaultError::Backend(anyhow::anyhow!("{ctx}: {e}"))
    }

    #[async_trait]
    impl PersonalVaultStore for PgPersonalVaultStore {
        async fn ensure_vault(
            &self,
            id: &str,
            owner_user: &str,
            public_key: &[u8],
        ) -> VaultResult<PersonalVault> {
            // ON CONFLICT DO NOTHING; then SELECT — keeps the public_key
            // stable across concurrent ensure_vault calls.
            sqlx::query(
                "INSERT INTO vault.vaults (id, owner_user, public_key, created_at)
                 VALUES ($1, $2, $3, EXTRACT(EPOCH FROM NOW()))
                 ON CONFLICT (owner_user) DO NOTHING",
            )
            .bind(id)
            .bind(owner_user)
            .bind(public_key)
            .execute(&self.pool)
            .await
            .map_err(map_err("ensure_vault insert"))?;

            self.get_by_owner(owner_user)
                .await?
                .ok_or_else(|| VaultError::Backend(anyhow::anyhow!("vault row missing post-insert")))
        }

        async fn get_by_owner(&self, owner_user: &str) -> VaultResult<Option<PersonalVault>> {
            let row: Option<(String, Vec<u8>, f64)> = sqlx::query_as(
                "SELECT id, public_key, created_at FROM vault.vaults WHERE owner_user = $1",
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
                "SELECT owner_user, public_key, created_at FROM vault.vaults WHERE id = $1",
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
                "UPDATE vault.vaults SET public_key = $2 WHERE owner_user = $1",
            )
            .bind(owner_user)
            .bind(new_public_key)
            .execute(&self.pool)
            .await
            .map_err(map_err("rotate_public_key"))?
            .rows_affected();
            Ok(n > 0)
        }
    }
}

#[cfg(feature = "vault-collections")]
pub use personal_vault::PgPersonalVaultStore;

#[cfg(feature = "vault-collections")]
mod collections {
    use super::*;
    use crate::collections::{Collection, CollectionMember, CollectionStore};
    use crate::error::{Result as VaultResult, VaultError};

    #[derive(Clone)]
    pub struct PgCollectionStore {
        pool: PgPool,
    }

    impl PgCollectionStore {
        pub fn new(pool: PgPool) -> Self {
            Self { pool }
        }
    }

    fn map_err(ctx: &'static str) -> impl FnOnce(sqlx::Error) -> VaultError {
        move |e| VaultError::Backend(anyhow::anyhow!("{ctx}: {e}"))
    }

    #[async_trait]
    impl CollectionStore for PgCollectionStore {
        async fn create_collection(
            &self,
            id: &str,
            org_id: Option<&str>,
            name: &str,
            created_by: &str,
        ) -> VaultResult<Collection> {
            let res = sqlx::query(
                "INSERT INTO vault.collections (id, org_id, name, created_by)
                 VALUES ($1, $2, $3, $4)",
            )
            .bind(id)
            .bind(org_id)
            .bind(name)
            .bind(created_by)
            .execute(&self.pool)
            .await;
            if let Err(sqlx::Error::Database(dberr)) = &res
                && dberr.code().as_deref() == Some("23505")
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
                   FROM vault.collections WHERE id = $1",
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
                       FROM vault.collections
                      WHERE org_id = $1
                      ORDER BY name",
                )
                .bind(o)
                .fetch_all(&self.pool)
                .await,
                None => sqlx::query_as(
                    "SELECT id, org_id, name, created_by, created_at
                       FROM vault.collections
                      ORDER BY name",
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
            let n = sqlx::query("DELETE FROM vault.collections WHERE id = $1")
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
                    (collection_id, user_id, wrapped_key, role)
                 VALUES ($1, $2, $3, $4)
                 ON CONFLICT (collection_id, user_id) DO UPDATE
                   SET wrapped_key = excluded.wrapped_key,
                       role        = excluded.role",
            )
            .bind(collection_id)
            .bind(user_id)
            .bind(wrapped_key)
            .bind(role)
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
                  WHERE collection_id = $1
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
                  WHERE collection_id = $1 AND user_id = $2",
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
                  WHERE collection_id = $1 AND user_id = $2",
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
pub use collections::PgCollectionStore;

#[cfg(feature = "vault-collections")]
mod items {
    use super::*;
    use crate::error::{Result as VaultResult, VaultError};
    use crate::items::{Folder, FolderStore, Item, ItemStore, Parent};

    #[derive(Clone)]
    pub struct PgItemStore {
        pool: PgPool,
    }
    #[derive(Clone)]
    pub struct PgFolderStore {
        pool: PgPool,
    }

    impl PgItemStore {
        pub fn new(pool: PgPool) -> Self {
            Self { pool }
        }
    }
    impl PgFolderStore {
        pub fn new(pool: PgPool) -> Self {
            Self { pool }
        }
    }

    fn map_err(ctx: &'static str) -> impl FnOnce(sqlx::Error) -> VaultError {
        move |e| VaultError::Backend(anyhow::anyhow!("{ctx}: {e}"))
    }

    fn parent_pair<'a>(p: Parent<'a>) -> (Option<&'a str>, Option<&'a str>) {
        match p {
            Parent::Vault(id) => (Some(id), None),
            Parent::Collection(id) => (None, Some(id)),
        }
    }

    #[async_trait]
    impl ItemStore for PgItemStore {
        async fn create_item(
            &self,
            id: &str,
            parent: Parent<'_>,
            folder_id: Option<&str>,
            item_type: &str,
            name: &str,
            ciphertext: &[u8],
            nonce: &[u8],
        ) -> VaultResult<Item> {
            let (vid, cid) = parent_pair(parent);
            let res = sqlx::query(
                "INSERT INTO vault.items
                    (id, vault_id, collection_id, folder_id, item_type, name, ciphertext, nonce)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
            )
            .bind(id)
            .bind(vid)
            .bind(cid)
            .bind(folder_id)
            .bind(item_type)
            .bind(name)
            .bind(ciphertext)
            .bind(nonce)
            .execute(&self.pool)
            .await;
            if let Err(sqlx::Error::Database(dberr)) = &res
                && dberr.code().as_deref() == Some("23505")
            {
                return Err(VaultError::Conflict(format!(
                    "item id '{id}' already exists"
                )));
            }
            res.map_err(map_err("create_item"))?;
            self.get_item(id)
                .await?
                .ok_or_else(|| VaultError::Backend(anyhow::anyhow!("item missing post-insert")))
        }

        async fn get_item(&self, id: &str) -> VaultResult<Option<Item>> {
            let row: Option<(
                Option<String>, Option<String>, Option<String>, String, String, Vec<u8>, Vec<u8>, f64, f64,
            )> = sqlx::query_as(
                "SELECT vault_id, collection_id, folder_id, item_type, name, ciphertext, nonce, created_at, updated_at
                   FROM vault.items WHERE id = $1",
            )
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(map_err("get_item"))?;
            Ok(row.map(|(v, c, f, t, n, ct, nc, ca, ua)| Item {
                id: id.to_string(),
                vault_id: v,
                collection_id: c,
                folder_id: f,
                item_type: t,
                name: n,
                ciphertext: ct,
                nonce: nc,
                created_at: ca,
                updated_at: ua,
            }))
        }

        async fn list_items(&self, parent: Parent<'_>) -> VaultResult<Vec<Item>> {
            let (vid, cid) = parent_pair(parent);
            let rows: Vec<(String, Option<String>, Option<String>, Option<String>, String, String, Vec<u8>, Vec<u8>, f64, f64)> =
                match (vid, cid) {
                    (Some(v), None) => sqlx::query_as(
                        "SELECT id, vault_id, collection_id, folder_id, item_type, name, ciphertext, nonce, created_at, updated_at
                           FROM vault.items WHERE vault_id = $1 ORDER BY created_at",
                    )
                    .bind(v)
                    .fetch_all(&self.pool)
                    .await,
                    (None, Some(c)) => sqlx::query_as(
                        "SELECT id, vault_id, collection_id, folder_id, item_type, name, ciphertext, nonce, created_at, updated_at
                           FROM vault.items WHERE collection_id = $1 ORDER BY created_at",
                    )
                    .bind(c)
                    .fetch_all(&self.pool)
                    .await,
                    _ => unreachable!("Parent guarantees exactly one Some"),
                }
                .map_err(map_err("list_items"))?;
            Ok(rows
                .into_iter()
                .map(|(id, v, c, f, t, n, ct, nc, ca, ua)| Item {
                    id,
                    vault_id: v,
                    collection_id: c,
                    folder_id: f,
                    item_type: t,
                    name: n,
                    ciphertext: ct,
                    nonce: nc,
                    created_at: ca,
                    updated_at: ua,
                })
                .collect())
        }

        async fn update_item(
            &self,
            id: &str,
            item_type: &str,
            name: &str,
            ciphertext: &[u8],
            nonce: &[u8],
            folder_id: Option<&str>,
        ) -> VaultResult<bool> {
            let n = sqlx::query(
                "UPDATE vault.items
                    SET item_type  = $2,
                        name       = $3,
                        ciphertext = $4,
                        nonce      = $5,
                        folder_id  = $6,
                        updated_at = EXTRACT(EPOCH FROM NOW())
                  WHERE id = $1",
            )
            .bind(id)
            .bind(item_type)
            .bind(name)
            .bind(ciphertext)
            .bind(nonce)
            .bind(folder_id)
            .execute(&self.pool)
            .await
            .map_err(map_err("update_item"))?
            .rows_affected();
            Ok(n > 0)
        }

        async fn delete_item(&self, id: &str) -> VaultResult<bool> {
            let n = sqlx::query("DELETE FROM vault.items WHERE id = $1")
                .bind(id)
                .execute(&self.pool)
                .await
                .map_err(map_err("delete_item"))?
                .rows_affected();
            Ok(n > 0)
        }
    }

    #[async_trait]
    impl FolderStore for PgFolderStore {
        async fn create_folder(
            &self,
            id: &str,
            parent: Parent<'_>,
            parent_folder_id: Option<&str>,
            name: &str,
        ) -> VaultResult<Folder> {
            let (vid, cid) = parent_pair(parent);
            sqlx::query(
                "INSERT INTO vault.folders
                    (id, vault_id, collection_id, parent_id, name)
                 VALUES ($1, $2, $3, $4, $5)",
            )
            .bind(id)
            .bind(vid)
            .bind(cid)
            .bind(parent_folder_id)
            .bind(name)
            .execute(&self.pool)
            .await
            .map_err(map_err("create_folder"))?;
            self.get_folder(id)
                .await?
                .ok_or_else(|| VaultError::Backend(anyhow::anyhow!("folder missing post-insert")))
        }

        async fn get_folder(&self, id: &str) -> VaultResult<Option<Folder>> {
            let row: Option<(Option<String>, Option<String>, Option<String>, String, f64)> =
                sqlx::query_as(
                    "SELECT vault_id, collection_id, parent_id, name, created_at
                       FROM vault.folders WHERE id = $1",
                )
                .bind(id)
                .fetch_optional(&self.pool)
                .await
                .map_err(map_err("get_folder"))?;
            Ok(row.map(|(v, c, p, n, ca)| Folder {
                id: id.to_string(),
                vault_id: v,
                collection_id: c,
                parent_id: p,
                name: n,
                created_at: ca,
            }))
        }

        async fn list_folders(&self, parent: Parent<'_>) -> VaultResult<Vec<Folder>> {
            let (vid, cid) = parent_pair(parent);
            let rows: Vec<(String, Option<String>, Option<String>, Option<String>, String, f64)> =
                match (vid, cid) {
                    (Some(v), None) => sqlx::query_as(
                        "SELECT id, vault_id, collection_id, parent_id, name, created_at
                           FROM vault.folders WHERE vault_id = $1 ORDER BY name",
                    )
                    .bind(v)
                    .fetch_all(&self.pool)
                    .await,
                    (None, Some(c)) => sqlx::query_as(
                        "SELECT id, vault_id, collection_id, parent_id, name, created_at
                           FROM vault.folders WHERE collection_id = $1 ORDER BY name",
                    )
                    .bind(c)
                    .fetch_all(&self.pool)
                    .await,
                    _ => unreachable!("Parent guarantees exactly one Some"),
                }
                .map_err(map_err("list_folders"))?;
            Ok(rows
                .into_iter()
                .map(|(id, v, c, p, n, ca)| Folder {
                    id,
                    vault_id: v,
                    collection_id: c,
                    parent_id: p,
                    name: n,
                    created_at: ca,
                })
                .collect())
        }

        async fn rename_folder(&self, id: &str, name: &str) -> VaultResult<bool> {
            let n = sqlx::query("UPDATE vault.folders SET name = $2 WHERE id = $1")
                .bind(id)
                .bind(name)
                .execute(&self.pool)
                .await
                .map_err(map_err("rename_folder"))?
                .rows_affected();
            Ok(n > 0)
        }

        async fn delete_folder(&self, id: &str) -> VaultResult<bool> {
            let n = sqlx::query("DELETE FROM vault.folders WHERE id = $1")
                .bind(id)
                .execute(&self.pool)
                .await
                .map_err(map_err("delete_folder"))?
                .rows_affected();
            Ok(n > 0)
        }
    }
}

#[cfg(feature = "vault-collections")]
pub use items::{PgFolderStore, PgItemStore};

#[cfg(feature = "vault-share")]
mod share {
    use super::*;
    use crate::error::{Result as VaultResult, VaultError};
    use crate::share::{RevocationEntry, RevocationStore};

    #[derive(Clone)]
    pub struct PgRevocationStore {
        pool: PgPool,
    }
    impl PgRevocationStore {
        pub fn new(pool: PgPool) -> Self {
            Self { pool }
        }
    }
    fn map_err(ctx: &'static str) -> impl FnOnce(sqlx::Error) -> VaultError {
        move |e| VaultError::Backend(anyhow::anyhow!("{ctx}: {e}"))
    }

    #[async_trait]
    impl RevocationStore for PgRevocationStore {
        async fn add(&self, key_id: &str, reason: &str) -> VaultResult<()> {
            sqlx::query(
                "INSERT INTO vault.share_revoked (key_id, reason)
                 VALUES ($1, $2)
                 ON CONFLICT (key_id) DO NOTHING",
            )
            .bind(key_id)
            .bind(reason)
            .execute(&self.pool)
            .await
            .map_err(map_err("share_revoked add"))?;
            Ok(())
        }

        async fn any_revoked(&self, key_ids: &[String]) -> VaultResult<bool> {
            if key_ids.is_empty() {
                return Ok(false);
            }
            let row: Option<(i64,)> = sqlx::query_as(
                "SELECT 1 FROM vault.share_revoked WHERE key_id = ANY($1) LIMIT 1",
            )
            .bind(key_ids)
            .fetch_optional(&self.pool)
            .await
            .map_err(map_err("share_revoked any_revoked"))?;
            Ok(row.is_some())
        }

        async fn list(&self) -> VaultResult<Vec<RevocationEntry>> {
            let rows: Vec<(String, f64, String)> = sqlx::query_as(
                "SELECT key_id, revoked_at, reason FROM vault.share_revoked ORDER BY revoked_at DESC",
            )
            .fetch_all(&self.pool)
            .await
            .map_err(map_err("share_revoked list"))?;
            Ok(rows
                .into_iter()
                .map(|(k, ra, r)| RevocationEntry {
                    key_id: k,
                    revoked_at: ra,
                    reason: r,
                })
                .collect())
        }
    }

    /// Load the active vault biscuit root keypair, or generate one on
    /// first boot.
    pub async fn load_or_init_biscuit_root_postgres(
        pool: &PgPool,
    ) -> anyhow::Result<biscuit_auth::KeyPair> {
        use anyhow::Context;
        let row: Option<(String, Vec<u8>)> = sqlx::query_as(
            "SELECT kid, private_pem FROM vault.biscuit_root_keys
              WHERE rotated_at IS NULL
              ORDER BY created_at DESC LIMIT 1",
        )
        .fetch_optional(pool)
        .await
        .context("read vault.biscuit_root_keys")?;
        if let Some((_kid, pem_bytes)) = row {
            let pem = String::from_utf8(pem_bytes)
                .context("vault biscuit private_pem is not valid UTF-8")?;
            return biscuit_auth::KeyPair::from_private_key_pem(&pem)
                .map_err(|e| anyhow::anyhow!("biscuit KeyPair::from_pem: {e:?}"));
        }
        let kp = biscuit_auth::KeyPair::new();
        let private_pem = kp
            .to_private_key_pem()
            .map_err(|e| anyhow::anyhow!("biscuit private_pem: {e:?}"))?;
        let public_pem = kp
            .public()
            .to_pem()
            .map_err(|e| anyhow::anyhow!("biscuit public_pem: {e:?}"))?;
        let kid = mint_kid(&public_pem);
        sqlx::query(
            "INSERT INTO vault.biscuit_root_keys (kid, private_pem, public_pem)
             VALUES ($1, $2, $3)",
        )
        .bind(&kid)
        .bind(private_pem.as_bytes())
        .bind(&public_pem)
        .execute(pool)
        .await
        .context("seed vault.biscuit_root_keys")?;
        tracing::info!(target: "assay-vault", %kid, "vault biscuit root keypair seeded");
        Ok(kp)
    }

    fn mint_kid(public_pem: &str) -> String {
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        h.update(b"assay-vault/biscuit-root/v1");
        h.update(public_pem.as_bytes());
        let d = h.finalize();
        format!("vbr-{}", data_encoding::HEXLOWER.encode(&d[..8]))
    }
}

#[cfg(feature = "vault-share")]
pub use share::{load_or_init_biscuit_root_postgres, PgRevocationStore};

#[cfg(any(
    feature = "vault-dynamic-postgres",
    feature = "vault-dynamic-aws",
    feature = "vault-dynamic-gcp",
    feature = "vault-dynamic-kubernetes",
))]
mod dynamic {
    use super::*;
    use crate::dynamic::{LeaseRecord, LeaseStore};
    use crate::error::{Result as VaultResult, VaultError};

    #[derive(Clone)]
    pub struct PgLeaseStore {
        pool: PgPool,
    }
    impl PgLeaseStore {
        pub fn new(pool: PgPool) -> Self {
            Self { pool }
        }
    }
    fn map_err(ctx: &'static str) -> impl FnOnce(sqlx::Error) -> VaultError {
        move |e| VaultError::Backend(anyhow::anyhow!("{ctx}: {e}"))
    }

    #[async_trait]
    impl LeaseStore for PgLeaseStore {
        async fn create_lease(
            &self,
            id: &str,
            provider: &str,
            role: &str,
            expires_at: f64,
            metadata: &serde_json::Value,
        ) -> VaultResult<()> {
            sqlx::query(
                "INSERT INTO vault.leases (id, provider, role, expires_at, metadata)
                 VALUES ($1, $2, $3, $4, $5)",
            )
            .bind(id)
            .bind(provider)
            .bind(role)
            .bind(expires_at)
            .bind(metadata)
            .execute(&self.pool)
            .await
            .map_err(map_err("create_lease"))?;
            Ok(())
        }

        async fn get_lease(&self, id: &str) -> VaultResult<Option<LeaseRecord>> {
            let row: Option<(String, String, f64, f64, Option<f64>, serde_json::Value)> =
                sqlx::query_as(
                    "SELECT provider, role, issued_at, expires_at, revoked_at, metadata
                       FROM vault.leases WHERE id = $1",
                )
                .bind(id)
                .fetch_optional(&self.pool)
                .await
                .map_err(map_err("get_lease"))?;
            Ok(row.map(|(p, r, ia, ea, ra, m)| LeaseRecord {
                id: id.to_string(),
                provider: p,
                role: r,
                issued_at: ia,
                expires_at: ea,
                revoked_at: ra,
                metadata: m,
            }))
        }

        async fn revoke_lease(&self, id: &str, now: f64) -> VaultResult<bool> {
            let n = sqlx::query(
                "UPDATE vault.leases SET revoked_at = $2
                  WHERE id = $1 AND revoked_at IS NULL",
            )
            .bind(id)
            .bind(now)
            .execute(&self.pool)
            .await
            .map_err(map_err("revoke_lease"))?
            .rows_affected();
            Ok(n > 0)
        }

        async fn list_expired_unrevoked(
            &self,
            now: f64,
            limit: i64,
        ) -> VaultResult<Vec<LeaseRecord>> {
            let rows: Vec<(String, String, String, f64, f64, Option<f64>, serde_json::Value)> =
                sqlx::query_as(
                    "SELECT id, provider, role, issued_at, expires_at, revoked_at, metadata
                       FROM vault.leases
                      WHERE expires_at < $1 AND revoked_at IS NULL
                      ORDER BY expires_at
                      LIMIT $2",
                )
                .bind(now)
                .bind(limit)
                .fetch_all(&self.pool)
                .await
                .map_err(map_err("list_expired_unrevoked"))?;
            Ok(rows
                .into_iter()
                .map(|(id, p, r, ia, ea, ra, m)| LeaseRecord {
                    id,
                    provider: p,
                    role: r,
                    issued_at: ia,
                    expires_at: ea,
                    revoked_at: ra,
                    metadata: m,
                })
                .collect())
        }

        async fn list_leases(
            &self,
            provider: Option<&str>,
        ) -> VaultResult<Vec<LeaseRecord>> {
            let rows: Vec<(String, String, String, f64, f64, Option<f64>, serde_json::Value)> =
                match provider {
                    Some(p) => sqlx::query_as(
                        "SELECT id, provider, role, issued_at, expires_at, revoked_at, metadata
                           FROM vault.leases WHERE provider = $1 ORDER BY issued_at DESC",
                    )
                    .bind(p)
                    .fetch_all(&self.pool)
                    .await,
                    None => sqlx::query_as(
                        "SELECT id, provider, role, issued_at, expires_at, revoked_at, metadata
                           FROM vault.leases ORDER BY issued_at DESC",
                    )
                    .fetch_all(&self.pool)
                    .await,
                }
                .map_err(map_err("list_leases"))?;
            Ok(rows
                .into_iter()
                .map(|(id, p, r, ia, ea, ra, m)| LeaseRecord {
                    id,
                    provider: p,
                    role: r,
                    issued_at: ia,
                    expires_at: ea,
                    revoked_at: ra,
                    metadata: m,
                })
                .collect())
        }
    }
}

#[cfg(any(
    feature = "vault-dynamic-postgres",
    feature = "vault-dynamic-aws",
    feature = "vault-dynamic-gcp",
    feature = "vault-dynamic-kubernetes",
))]
pub use dynamic::PgLeaseStore;
