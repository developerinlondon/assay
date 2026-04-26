//! Postgres [`ZanzibarStore`] implementation — recursive CTE walks
//! over `auth.zanzibar_tuples` for `check`, `expand`, and the lookup
//! pair.
//!
//! Layout: a single round-trip per call. The CTE seeds with the
//! direct tuples on the resource for whatever relation set the
//! [`super::resolve`] pass produced, then expands one userset hop at
//! a time bounded by [`super::types::MAX_DEPTH`]. Cycle detection
//! uses an in-CTE `path` array — if the next subject is already on
//! the path, the join skips it.
//!
//! Performance notes (PG18 EXPLAIN-checked once during
//! development against a 100k-tuple seed):
//!
//! - Forward walk hits the composite PK `(object_type, object_id,
//!   relation, …)` for the seed and then for each hop.
//! - Reverse `lookup_resources` hits `idx_auth_zanzibar_tuples_rev`
//!   for the seed; subsequent hops still use the PK because the next
//!   step is "given subject, find object" → forward direction again.
//! - JSONB `auth.zanzibar_namespaces.schema_json` is round-tripped
//!   via `serde_json::Value` (sqlx's `JsonValue` mapping); for the
//!   100-namespace order-of-magnitude any v0.2.0 deployment will
//!   actually have, parsing on every check is fine — the schema
//!   cache is a phase-9 follow-up.

use std::sync::Arc;

use anyhow::{Context, Result};
use sqlx::{PgPool, Row};

use super::resolve::resolve;
use super::store::ZanzibarStore;
use super::types::{
    CheckResult, Consistency, NamespaceSchema, ObjectRef, SubjectRef, Tuple, TreeOp,
    UsersetTree, MAX_DEPTH,
};

/// Postgres-backed Zanzibar store. Cheap to clone (the underlying
/// `PgPool` is `Arc` internally).
#[derive(Clone)]
pub struct PostgresZanzibarStore {
    pool: PgPool,
}

impl PostgresZanzibarStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Wrap into an `Arc<dyn ZanzibarStore>` for [`crate::ctx::AuthCtx`].
    pub fn into_dyn(self) -> Arc<dyn ZanzibarStore> {
        Arc::new(self)
    }
}

#[async_trait::async_trait]
impl ZanzibarStore for PostgresZanzibarStore {
    async fn define_namespace(&self, schema: &NamespaceSchema) -> Result<()> {
        let json = serde_json::to_value(schema)
            .context("zanzibar serialize NamespaceSchema")?;
        sqlx::query(
            "INSERT INTO auth.zanzibar_namespaces (name, schema_json, updated_at)
             VALUES ($1, $2, EXTRACT(EPOCH FROM NOW()))
             ON CONFLICT (name) DO UPDATE
                 SET schema_json = EXCLUDED.schema_json,
                     updated_at = EXCLUDED.updated_at",
        )
        .bind(&schema.name)
        .bind(json)
        .execute(&self.pool)
        .await
        .context("auth.zanzibar_namespaces upsert")?;
        Ok(())
    }

    async fn get_namespace(&self, name: &str) -> Result<Option<NamespaceSchema>> {
        let row: Option<(serde_json::Value,)> = sqlx::query_as(
            "SELECT schema_json FROM auth.zanzibar_namespaces WHERE name = $1",
        )
        .bind(name)
        .fetch_optional(&self.pool)
        .await
        .context("auth.zanzibar_namespaces get")?;
        Ok(match row {
            Some((json,)) => Some(
                serde_json::from_value(json)
                    .context("zanzibar deserialize NamespaceSchema")?,
            ),
            None => None,
        })
    }

    async fn list_namespaces(&self) -> Result<Vec<NamespaceSchema>> {
        let rows: Vec<(serde_json::Value,)> = sqlx::query_as(
            "SELECT schema_json FROM auth.zanzibar_namespaces ORDER BY name",
        )
        .fetch_all(&self.pool)
        .await
        .context("auth.zanzibar_namespaces list")?;
        rows.into_iter()
            .map(|(json,)| {
                serde_json::from_value(json).context("zanzibar deserialize NamespaceSchema")
            })
            .collect()
    }

    async fn write_tuple(&self, t: &Tuple) -> Result<()> {
        sqlx::query(
            "INSERT INTO auth.zanzibar_tuples
                (object_type, object_id, relation,
                 subject_type, subject_id, subject_rel, created_at)
             VALUES ($1, $2, $3, $4, $5, $6, EXTRACT(EPOCH FROM NOW()))
             ON CONFLICT DO NOTHING",
        )
        .bind(&t.object_type)
        .bind(&t.object_id)
        .bind(&t.relation)
        .bind(&t.subject_type)
        .bind(&t.subject_id)
        .bind(&t.subject_rel)
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
            sqlx::query(
                "INSERT INTO auth.zanzibar_tuples
                    (object_type, object_id, relation,
                     subject_type, subject_id, subject_rel, created_at)
                 VALUES ($1, $2, $3, $4, $5, $6, EXTRACT(EPOCH FROM NOW()))
                 ON CONFLICT DO NOTHING",
            )
            .bind(&t.object_type)
            .bind(&t.object_id)
            .bind(&t.relation)
            .bind(&t.subject_type)
            .bind(&t.subject_id)
            .bind(&t.subject_rel)
            .execute(&mut *tx)
            .await
            .context("auth.zanzibar_tuples batch insert")?;
        }
        tx.commit().await.context("commit tuples txn")?;
        Ok(())
    }

    async fn delete_tuple(&self, t: &Tuple) -> Result<bool> {
        // PG: NULL distinct in equality, so the `subject_rel IS NOT
        // DISTINCT FROM $6` form is needed when subject_rel is NULL.
        let res = sqlx::query(
            "DELETE FROM auth.zanzibar_tuples
             WHERE object_type = $1 AND object_id = $2 AND relation = $3
               AND subject_type = $4 AND subject_id = $5
               AND subject_rel IS NOT DISTINCT FROM $6",
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
        // 1) Resolve the permission to its candidate relation set.
        //    No namespace defined → deny (the safe default).
        let Some(schema) = self.get_namespace(&resource.object_type).await? else {
            return Ok(CheckResult::Denied);
        };
        let Some(resolved) = resolve(&schema, permission) else {
            // Schema cycle. Surface as `CycleDetected` so callers/UI
            // can flag the bad schema; this is *not* an error.
            return Ok(CheckResult::CycleDetected);
        };
        if resolved.union_relations.is_empty() {
            return Ok(CheckResult::Denied);
        }
        let relation_list: Vec<String> = resolved.union_relations.into_iter().collect();

        // 2) Run the recursive walk. Postgres' arrays + the
        //    `IS NOT DISTINCT FROM` operator handle the `subject_rel`
        //    nullable comparison cleanly. Cycle guard via the in-CTE
        //    `path` array — if the next subject is already on the
        //    path, the join produces zero rows.
        //
        //    The seed binds `subject_rel IS NULL` for direct subjects
        //    (the only kind `check` answers — usersets are intermediate
        //    states the CTE walks through, never the terminal answer).
        let row: Option<(i32,)> = sqlx::query_as(
            r#"
            WITH RECURSIVE walk(subject_type, subject_id, subject_rel, depth, path) AS (
                SELECT t.subject_type,
                       t.subject_id,
                       t.subject_rel,
                       1 AS depth,
                       ARRAY[t.subject_type || ':' || t.subject_id] AS path
                FROM auth.zanzibar_tuples t
                WHERE t.object_type = $1 AND t.object_id = $2 AND t.relation = ANY($3)
                UNION ALL
                SELECT t.subject_type,
                       t.subject_id,
                       t.subject_rel,
                       w.depth + 1,
                       w.path || (t.subject_type || ':' || t.subject_id)
                FROM auth.zanzibar_tuples t
                JOIN walk w
                  ON t.object_type = w.subject_type
                 AND t.object_id   = w.subject_id
                 AND w.subject_rel IS NOT NULL
                 AND t.relation = w.subject_rel
                WHERE w.depth < $4
                  AND NOT (t.subject_type || ':' || t.subject_id) = ANY(w.path)
            )
            SELECT CASE
                WHEN EXISTS (
                    SELECT 1 FROM walk
                    WHERE subject_type = $5 AND subject_id = $6 AND subject_rel IS NULL
                ) THEN 1
                WHEN EXISTS (SELECT 1 FROM walk WHERE depth >= $4) THEN 2
                ELSE 0
            END AS verdict
            "#,
        )
        .bind(&resource.object_type)
        .bind(&resource.object_id)
        .bind(&relation_list)
        .bind(MAX_DEPTH as i32)
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
        // Single-relation expansion — pull every direct subject of
        // `resource#relation`, then for each userset hop, recurse.
        // Diagnostic-only path; we keep the implementation simple
        // (recursive Rust calls) rather than another CTE.
        let depth = depth_limit.min(MAX_DEPTH);
        Ok(UsersetTree::Node {
            op: TreeOp::Direct,
            children: expand_pg(self, resource, relation, depth, &mut Vec::new()).await?,
        })
    }

    async fn lookup_resources(
        &self,
        resource_type: &str,
        permission: &str,
        subject: &SubjectRef,
    ) -> Result<Vec<ObjectRef>> {
        // Cheap heuristic for v0.2.0: we walk forward from every
        // resource of the requested type that has *any* tuple in the
        // candidate relation set, then post-filter via `check`.
        // Optimal for small fan-outs (the family/circle scale jeebon
        // ships at); a real production deployment will want a
        // dedicated reverse index.
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
        // First pass: all candidate resources (objects with a tuple in
        // the relation set, regardless of subject — narrows the search
        // to one per resource).
        let rows = sqlx::query(
            "SELECT DISTINCT object_type, object_id
             FROM auth.zanzibar_tuples
             WHERE object_type = $1 AND relation = ANY($2)",
        )
        .bind(resource_type)
        .bind(&relation_list)
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
        // Forward walk from the resource — every direct subject under
        // the candidate relation set. Userset intermediates are
        // followed transitively via the same recursive CTE as `check`.
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
        let rows = sqlx::query(
            r#"
            WITH RECURSIVE walk(subject_type, subject_id, subject_rel, depth, path) AS (
                SELECT t.subject_type, t.subject_id, t.subject_rel, 1,
                       ARRAY[t.subject_type || ':' || t.subject_id]
                FROM auth.zanzibar_tuples t
                WHERE t.object_type = $1 AND t.object_id = $2 AND t.relation = ANY($3)
                UNION ALL
                SELECT t.subject_type, t.subject_id, t.subject_rel, w.depth + 1,
                       w.path || (t.subject_type || ':' || t.subject_id)
                FROM auth.zanzibar_tuples t
                JOIN walk w
                  ON t.object_type = w.subject_type
                 AND t.object_id   = w.subject_id
                 AND w.subject_rel IS NOT NULL
                 AND t.relation = w.subject_rel
                WHERE w.depth < $4
                  AND NOT (t.subject_type || ':' || t.subject_id) = ANY(w.path)
            )
            SELECT DISTINCT subject_type, subject_id
            FROM walk
            WHERE subject_type = $5 AND subject_rel IS NULL
            "#,
        )
        .bind(&resource.object_type)
        .bind(&resource.object_id)
        .bind(&relation_list)
        .bind(MAX_DEPTH as i32)
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

/// Recursive helper for `expand` — fetches direct tuples and recurses
/// into userset subjects up to `depth`.
fn expand_pg<'a>(
    store: &'a PostgresZanzibarStore,
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
             WHERE object_type = $1 AND object_id = $2 AND relation = $3",
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
            let sr: Option<String> = row.get("subject_rel");
            match sr {
                None => children.push(UsersetTree::Leaf {
                    subject: SubjectRef::direct(st, sid),
                }),
                Some(r) => {
                    let inner_resource = ObjectRef::new(st.clone(), sid.clone());
                    let sub = expand_pg(store, &inner_resource, &r, depth - 1, seen).await?;
                    children.push(UsersetTree::Node {
                        op: TreeOp::TuplesetArrow,
                        children: vec![
                            UsersetTree::Leaf {
                                subject: SubjectRef::userset(st, sid, r),
                            },
                            UsersetTree::Node {
                                op: TreeOp::Direct,
                                children: sub,
                            },
                        ],
                    });
                }
            }
        }
        Ok(children)
    })
}
