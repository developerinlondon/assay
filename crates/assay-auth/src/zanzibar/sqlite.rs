//! SQLite [`ZanzibarStore`] implementation — same recursive-CTE shape
//! as the Postgres backend, with the dialect differences absorbed:
//!
//! - `JSONB` → `TEXT`. We round-trip through `serde_json::to_string` /
//!   `from_str` rather than relying on `sqlx::types::Json` so the
//!   storage format is human-readable in the file (matters when an
//!   operator opens `data/auth.db` to debug).
//! - `text[]` → JSON-encoded array consumed by `json_each`. SQLite
//!   doesn't have a native array type; the standard idiom is `WHERE
//!   relation IN (SELECT value FROM json_each(?))`.
//! - `IS NOT DISTINCT FROM` doesn't exist; SQLite's `IS` operator
//!   already treats NULL=NULL as true, so direct-tuple delete uses
//!   `subject_rel IS ?`.
//! - The cycle-guard "path" is encoded as a JSON array via
//!   `json_array(...)` + `json_array_length` for the depth limit, and
//!   `instr(json_path, key)` for the membership check — matches the
//!   PG `path || ROW(...) ANY` semantic in spirit.

use std::sync::Arc;

use anyhow::{Context, Result};
use sqlx::{Row, SqlitePool};

use super::resolve::resolve;
use super::store::ZanzibarStore;
use super::types::{
    CheckResult, Consistency, NamespaceSchema, ObjectRef, SubjectRef, Tuple, TreeOp,
    UsersetTree, MAX_DEPTH,
};

/// SQLite-backed Zanzibar store. Cheap to clone (the underlying pool
/// is `Arc`d).
#[derive(Clone)]
pub struct SqliteZanzibarStore {
    pool: SqlitePool,
}

impl SqliteZanzibarStore {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub fn into_dyn(self) -> Arc<dyn ZanzibarStore> {
        Arc::new(self)
    }
}

fn now_secs() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

#[async_trait::async_trait]
impl ZanzibarStore for SqliteZanzibarStore {
    async fn define_namespace(&self, schema: &NamespaceSchema) -> Result<()> {
        let json = serde_json::to_string(schema)
            .context("zanzibar serialize NamespaceSchema")?;
        let now = now_secs();
        sqlx::query(
            "INSERT INTO auth.zanzibar_namespaces (name, schema_json, updated_at)
             VALUES (?, ?, ?)
             ON CONFLICT (name) DO UPDATE
                 SET schema_json = excluded.schema_json,
                     updated_at  = excluded.updated_at",
        )
        .bind(&schema.name)
        .bind(json)
        .bind(now)
        .execute(&self.pool)
        .await
        .context("auth.zanzibar_namespaces upsert")?;
        Ok(())
    }

    async fn get_namespace(&self, name: &str) -> Result<Option<NamespaceSchema>> {
        let row: Option<(String,)> = sqlx::query_as(
            "SELECT schema_json FROM auth.zanzibar_namespaces WHERE name = ?",
        )
        .bind(name)
        .fetch_optional(&self.pool)
        .await
        .context("auth.zanzibar_namespaces get")?;
        Ok(match row {
            Some((json,)) => Some(
                serde_json::from_str(&json)
                    .context("zanzibar deserialize NamespaceSchema")?,
            ),
            None => None,
        })
    }

    async fn list_namespaces(&self) -> Result<Vec<NamespaceSchema>> {
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT schema_json FROM auth.zanzibar_namespaces ORDER BY name",
        )
        .fetch_all(&self.pool)
        .await
        .context("auth.zanzibar_namespaces list")?;
        rows.into_iter()
            .map(|(json,)| {
                serde_json::from_str(&json).context("zanzibar deserialize NamespaceSchema")
            })
            .collect()
    }

    async fn write_tuple(&self, t: &Tuple) -> Result<()> {
        let now = now_secs();
        sqlx::query(
            "INSERT INTO auth.zanzibar_tuples
                (object_type, object_id, relation,
                 subject_type, subject_id, subject_rel, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT DO NOTHING",
        )
        .bind(&t.object_type)
        .bind(&t.object_id)
        .bind(&t.relation)
        .bind(&t.subject_type)
        .bind(&t.subject_id)
        .bind(&t.subject_rel)
        .bind(now)
        .execute(&self.pool)
        .await
        .context("auth.zanzibar_tuples insert")?;
        Ok(())
    }

    async fn write_tuples(&self, tuples: &[Tuple]) -> Result<()> {
        if tuples.is_empty() {
            return Ok(());
        }
        let mut tx = self.pool.begin().await.context("begin tuples txn")?;
        for t in tuples {
            let now = now_secs();
            sqlx::query(
                "INSERT INTO auth.zanzibar_tuples
                    (object_type, object_id, relation,
                     subject_type, subject_id, subject_rel, created_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?)
                 ON CONFLICT DO NOTHING",
            )
            .bind(&t.object_type)
            .bind(&t.object_id)
            .bind(&t.relation)
            .bind(&t.subject_type)
            .bind(&t.subject_id)
            .bind(&t.subject_rel)
            .bind(now)
            .execute(&mut *tx)
            .await
            .context("auth.zanzibar_tuples batch insert")?;
        }
        tx.commit().await.context("commit tuples txn")?;
        Ok(())
    }

    async fn delete_tuple(&self, t: &Tuple) -> Result<bool> {
        // subject_rel is NOT NULL ('' for direct), so plain equality
        // works on both backends — no `IS` / `IS NOT DISTINCT FROM`
        // dance needed.
        let res = sqlx::query(
            "DELETE FROM auth.zanzibar_tuples
             WHERE object_type = ? AND object_id = ? AND relation = ?
               AND subject_type = ? AND subject_id = ?
               AND subject_rel = ?",
        )
        .bind(&t.object_type)
        .bind(&t.object_id)
        .bind(&t.relation)
        .bind(&t.subject_type)
        .bind(&t.subject_id)
        .bind(&t.subject_rel)
        .execute(&self.pool)
        .await
        .context("auth.zanzibar_tuples delete")?;
        Ok(res.rows_affected() > 0)
    }

    async fn check(
        &self,
        resource: &ObjectRef,
        permission: &str,
        subject: &SubjectRef,
        _consistency: Consistency,
    ) -> Result<CheckResult> {
        let Some(schema) = self.get_namespace(&resource.object_type).await? else {
            return Ok(CheckResult::Denied);
        };
        let Some(resolved) = resolve(&schema, permission) else {
            return Ok(CheckResult::CycleDetected);
        };
        if resolved.union_relations.is_empty() {
            return Ok(CheckResult::Denied);
        }
        // SQLite has no array type — encode the relation set as a JSON
        // array consumed via `json_each(?)`. Same shape as PG's
        // `text[]` parameter, just one extra serialisation step.
        let relation_list: Vec<String> = resolved.union_relations.into_iter().collect();
        let relation_json = serde_json::to_string(&relation_list)
            .context("encode relation list as JSON")?;

        // Cycle-guard via a JSON-encoded path string. Each hop
        // appends `<type>:<id>` and the join refuses to add a node
        // already on the path. SQLite recursive CTEs allow this with
        // ordinary `||` string concat.
        let row: Option<(i64,)> = sqlx::query_as(
            r#"
            WITH RECURSIVE walk(subject_type, subject_id, subject_rel, depth, path) AS (
                SELECT t.subject_type,
                       t.subject_id,
                       t.subject_rel,
                       1 AS depth,
                       '|' || t.subject_type || ':' || t.subject_id || '|' AS path
                FROM auth.zanzibar_tuples t
                WHERE t.object_type = ?1
                  AND t.object_id = ?2
                  AND t.relation IN (SELECT value FROM json_each(?3))
                UNION ALL
                SELECT t.subject_type,
                       t.subject_id,
                       t.subject_rel,
                       w.depth + 1,
                       w.path || t.subject_type || ':' || t.subject_id || '|'
                FROM auth.zanzibar_tuples t
                JOIN walk w
                  ON t.object_type = w.subject_type
                 AND t.object_id   = w.subject_id
                 AND w.subject_rel <> ''
                 AND t.relation = w.subject_rel
                WHERE w.depth < ?4
                  AND instr(w.path, '|' || t.subject_type || ':' || t.subject_id || '|') = 0
            )
            SELECT CASE
                WHEN EXISTS (
                    SELECT 1 FROM walk
                    WHERE subject_type = ?5 AND subject_id = ?6 AND subject_rel = ''
                ) THEN 1
                WHEN EXISTS (SELECT 1 FROM walk WHERE depth >= ?4) THEN 2
                ELSE 0
            END AS verdict
            "#,
        )
        .bind(&resource.object_type)
        .bind(&resource.object_id)
        .bind(relation_json)
        .bind(MAX_DEPTH as i64)
        .bind(&subject.subject_type)
        .bind(&subject.subject_id)
        .fetch_optional(&self.pool)
        .await
        .context("auth.zanzibar check CTE")?;

        let verdict = row.map(|(v,)| v).unwrap_or(0);
        Ok(match verdict {
            1 => CheckResult::Allowed {
                resolved_via: Vec::new(),
            },
            2 => CheckResult::DepthExceeded,
            _ => CheckResult::Denied,
        })
    }

    async fn expand(
        &self,
        resource: &ObjectRef,
        relation: &str,
        depth_limit: u32,
    ) -> Result<UsersetTree> {
        let depth = depth_limit.min(MAX_DEPTH);
        Ok(UsersetTree::Node {
            op: TreeOp::Direct,
            children: expand_sqlite(self, resource, relation, depth, &mut Vec::new())
                .await?,
        })
    }

    async fn lookup_resources(
        &self,
        resource_type: &str,
        permission: &str,
        subject: &SubjectRef,
    ) -> Result<Vec<ObjectRef>> {
        let Some(schema) = self.get_namespace(resource_type).await? else {
            return Ok(Vec::new());
        };
        let Some(resolved) = resolve(&schema, permission) else {
            return Ok(Vec::new());
        };
        if resolved.union_relations.is_empty() {
            return Ok(Vec::new());
        }
        let relation_list: Vec<String> = resolved.union_relations.into_iter().collect();
        let relation_json = serde_json::to_string(&relation_list)
            .context("encode relation list as JSON")?;
        let rows = sqlx::query(
            "SELECT DISTINCT object_type, object_id
             FROM auth.zanzibar_tuples
             WHERE object_type = ?
               AND relation IN (SELECT value FROM json_each(?))",
        )
        .bind(resource_type)
        .bind(relation_json)
        .fetch_all(&self.pool)
        .await
        .context("auth.zanzibar_tuples candidate resources")?;
        let mut out = Vec::new();
        for row in rows {
            let object_type: String = row.get("object_type");
            let object_id: String = row.get("object_id");
            let r = ObjectRef::new(object_type, object_id);
            if self
                .check(&r, permission, subject, Consistency::Minimum)
                .await?
                .is_allowed()
            {
                out.push(r);
            }
        }
        Ok(out)
    }

    async fn lookup_subjects(
        &self,
        subject_type: &str,
        resource: &ObjectRef,
        permission: &str,
    ) -> Result<Vec<SubjectRef>> {
        let Some(schema) = self.get_namespace(&resource.object_type).await? else {
            return Ok(Vec::new());
        };
        let Some(resolved) = resolve(&schema, permission) else {
            return Ok(Vec::new());
        };
        if resolved.union_relations.is_empty() {
            return Ok(Vec::new());
        }
        let relation_list: Vec<String> = resolved.union_relations.into_iter().collect();
        let relation_json = serde_json::to_string(&relation_list)
            .context("encode relation list as JSON")?;

        let rows = sqlx::query(
            r#"
            WITH RECURSIVE walk(subject_type, subject_id, subject_rel, depth, path) AS (
                SELECT t.subject_type, t.subject_id, t.subject_rel, 1,
                       '|' || t.subject_type || ':' || t.subject_id || '|'
                FROM auth.zanzibar_tuples t
                WHERE t.object_type = ?1
                  AND t.object_id   = ?2
                  AND t.relation IN (SELECT value FROM json_each(?3))
                UNION ALL
                SELECT t.subject_type, t.subject_id, t.subject_rel, w.depth + 1,
                       w.path || t.subject_type || ':' || t.subject_id || '|'
                FROM auth.zanzibar_tuples t
                JOIN walk w
                  ON t.object_type = w.subject_type
                 AND t.object_id   = w.subject_id
                 AND w.subject_rel <> ''
                 AND t.relation = w.subject_rel
                WHERE w.depth < ?4
                  AND instr(w.path, '|' || t.subject_type || ':' || t.subject_id || '|') = 0
            )
            SELECT DISTINCT subject_type, subject_id
            FROM walk
            WHERE subject_type = ?5 AND subject_rel = ''
            "#,
        )
        .bind(&resource.object_type)
        .bind(&resource.object_id)
        .bind(relation_json)
        .bind(MAX_DEPTH as i64)
        .bind(subject_type)
        .fetch_all(&self.pool)
        .await
        .context("auth.zanzibar lookup_subjects CTE")?;
        Ok(rows
            .into_iter()
            .map(|row| {
                SubjectRef::direct(
                    row.get::<String, _>("subject_type"),
                    row.get::<String, _>("subject_id"),
                )
            })
            .collect())
    }
}

/// Recursive helper for `expand` — same shape as the PG version, just
/// SQLite's parameter binding (`?` instead of `$N`).
fn expand_sqlite<'a>(
    store: &'a SqliteZanzibarStore,
    resource: &'a ObjectRef,
    relation: &'a str,
    depth: u32,
    seen: &'a mut Vec<String>,
) -> std::pin::Pin<
    Box<dyn std::future::Future<Output = Result<Vec<UsersetTree>>> + Send + 'a>,
> {
    Box::pin(async move {
        if depth == 0 {
            return Ok(Vec::new());
        }
        let key = format!("{}:{}#{}", resource.object_type, resource.object_id, relation);
        if seen.contains(&key) {
            return Ok(Vec::new());
        }
        seen.push(key);
        let rows = sqlx::query(
            "SELECT subject_type, subject_id, subject_rel
             FROM auth.zanzibar_tuples
             WHERE object_type = ? AND object_id = ? AND relation = ?",
        )
        .bind(&resource.object_type)
        .bind(&resource.object_id)
        .bind(relation)
        .fetch_all(&store.pool)
        .await
        .context("auth.zanzibar_tuples expand fetch")?;
        let mut children = Vec::new();
        for row in rows {
            let st: String = row.get("subject_type");
            let sid: String = row.get("subject_id");
            let sr: String = row.get("subject_rel");
            if sr.is_empty() {
                children.push(UsersetTree::Leaf {
                    subject: SubjectRef::direct(st, sid),
                });
            } else {
                let inner_resource = ObjectRef::new(st.clone(), sid.clone());
                let sub = expand_sqlite(store, &inner_resource, &sr, depth - 1, seen).await?;
                children.push(UsersetTree::Node {
                    op: TreeOp::TuplesetArrow,
                    children: vec![
                        UsersetTree::Leaf {
                            subject: SubjectRef::userset(st, sid, sr.clone()),
                        },
                        UsersetTree::Node {
                            op: TreeOp::Direct,
                            children: sub,
                        },
                    ],
                });
            }
        }
        Ok(children)
    })
}
