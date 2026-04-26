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
