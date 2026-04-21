//! SurrealDB implementation of API-key `WorkflowStore` methods (Task 3.13).

use std::future::Future;

use assay_core::ApiKeyRecord;

use super::SurrealDbStore;

fn row_to_api_key(v: &serde_json::Value) -> Option<ApiKeyRecord> {
    let prefix = v.get("prefix")?.as_str()?.to_string();
    let label = v.get("label").and_then(|x| {
        if x.is_null() {
            None
        } else {
            x.as_str().map(|s| s.to_string())
        }
    });
    let created_at = v.get("created_at")?.as_f64().unwrap_or(0.0);
    Some(ApiKeyRecord {
        prefix,
        label,
        created_at,
    })
}

impl SurrealDbStore {
    /// Insert a new API key. Uses SELECT-then-CREATE to avoid duplicate errors
    /// (SurrealDB SCHEMAFULL tables don't support ON CONFLICT).
    pub(crate) fn create_api_key_impl(
        &self,
        key_hash: &str,
        prefix: &str,
        label: Option<&str>,
        created_at: f64,
    ) -> impl Future<Output = anyhow::Result<()>> + Send {
        let db = self.db.clone();
        let key_hash = key_hash.to_string();
        let prefix = prefix.to_string();
        let label = label.map(|s| s.to_string());
        async move {
            // Check for existing record with this key_hash (unique index).
            let existing: Vec<serde_json::Value> = db
                .query("SELECT key_hash FROM api_key WHERE key_hash = $kh LIMIT 1")
                .bind(("kh", key_hash.clone()))
                .await?
                .take(0)?;
            if !existing.is_empty() {
                // Already exists — idempotent.
                return Ok(());
            }
            // Use the prefix as the record ID (also unique).
            db.query(
                "CREATE type::record('api_key', $rec_id) CONTENT {
                    key_hash:   $key_hash,
                    prefix:     $prefix,
                    label:      $lbl,
                    created_at: $created_at
                }",
            )
            .bind(("rec_id", prefix.clone()))
            .bind(("key_hash", key_hash))
            .bind(("prefix", prefix))
            .bind(("lbl", label))
            .bind(("created_at", created_at))
            .await
            .map_err(|e| anyhow::anyhow!("create_api_key: {e}"))?;
            Ok(())
        }
    }

    pub(crate) fn validate_api_key_impl(
        &self,
        key_hash: &str,
    ) -> impl Future<Output = anyhow::Result<bool>> + Send {
        let db = self.db.clone();
        let key_hash = key_hash.to_string();
        async move {
            let rows: Vec<serde_json::Value> = db
                .query("SELECT key_hash FROM api_key WHERE key_hash = $kh LIMIT 1")
                .bind(("kh", key_hash))
                .await?
                .take(0)?;
            Ok(!rows.is_empty())
        }
    }

    pub(crate) fn list_api_keys_impl(
        &self,
    ) -> impl Future<Output = anyhow::Result<Vec<ApiKeyRecord>>> + Send {
        let db = self.db.clone();
        async move {
            let rows: Vec<serde_json::Value> = db
                .query("SELECT prefix, label, created_at FROM api_key ORDER BY created_at DESC")
                .await?
                .take(0)?;
            Ok(rows.iter().filter_map(row_to_api_key).collect())
        }
    }

    pub(crate) fn revoke_api_key_impl(
        &self,
        prefix: &str,
    ) -> impl Future<Output = anyhow::Result<bool>> + Send {
        let db = self.db.clone();
        let prefix = prefix.to_string();
        async move {
            let existing: Vec<serde_json::Value> = db
                .query("SELECT prefix FROM api_key WHERE prefix = $pfx LIMIT 1")
                .bind(("pfx", prefix.clone()))
                .await?
                .take(0)?;
            if existing.is_empty() {
                return Ok(false);
            }
            db.query("DELETE api_key WHERE prefix = $pfx")
                .bind(("pfx", prefix))
                .await?;
            Ok(true)
        }
    }

    pub(crate) fn api_keys_empty_impl(
        &self,
    ) -> impl Future<Output = anyhow::Result<bool>> + Send {
        let db = self.db.clone();
        async move {
            let rows: Vec<serde_json::Value> = db
                .query("SELECT count() AS c FROM api_key GROUP ALL")
                .await?
                .take(0)?;
            let count = rows
                .first()
                .and_then(|v| v.get("c"))
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            Ok(count == 0)
        }
    }

    pub(crate) fn get_api_key_by_label_impl(
        &self,
        label: &str,
    ) -> impl Future<Output = anyhow::Result<Option<ApiKeyRecord>>> + Send {
        let db = self.db.clone();
        let label = label.to_string();
        async move {
            let rows: Vec<serde_json::Value> = db
                .query("SELECT prefix, label, created_at FROM api_key WHERE label = $lbl LIMIT 1")
                .bind(("lbl", label))
                .await?
                .take(0)?;
            Ok(rows.first().and_then(row_to_api_key))
        }
    }
}
