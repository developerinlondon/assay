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

use super::eval::{LeafCheck, Verdict, evaluate};
use super::resolve::resolve;
use super::store::ZanzibarStore;
use super::types::{
    CheckResult, Consistency, MAX_DEPTH, NamespaceSchema, ObjectRef, SubjectRef, TreeOp, Tuple,
    TupleFilter, UsersetTree,
};
use std::future::Future;
use std::pin::Pin;

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
        let json = serde_json::to_value(schema).context("zanzibar serialize NamespaceSchema")?;
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
        let row: Option<(serde_json::Value,)> =
            sqlx::query_as("SELECT schema_json FROM auth.zanzibar_namespaces WHERE name = $1")
                .bind(name)
                .fetch_optional(&self.pool)
                .await
                .context("auth.zanzibar_namespaces get")?;
        Ok(match row {
            Some((json,)) => {
                Some(serde_json::from_value(json).context("zanzibar deserialize NamespaceSchema")?)
            }
            None => None,
        })
    }

    async fn list_namespaces(&self) -> Result<Vec<NamespaceSchema>> {
        let rows: Vec<(serde_json::Value,)> =
            sqlx::query_as("SELECT schema_json FROM auth.zanzibar_namespaces ORDER BY name")
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
        // subject_rel is NOT NULL ('' for direct), so plain equality
        // suffices — no IS NOT DISTINCT FROM dance.
        let res = sqlx::query(
            "DELETE FROM auth.zanzibar_tuples
             WHERE object_type = $1 AND object_id = $2 AND relation = $3
               AND subject_type = $4 AND subject_id = $5
               AND subject_rel = $6",
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

    async fn list_tuples(&self, filter: &TupleFilter) -> Result<Vec<Tuple>> {
        let rows: Vec<(String, String, String, String, String, String)> = sqlx::query_as(
            "SELECT object_type, object_id, relation,
                    subject_type, subject_id, subject_rel
             FROM auth.zanzibar_tuples
             WHERE ($1::text IS NULL OR object_type  = $1)
               AND ($2::text IS NULL OR object_id    = $2)
               AND ($3::text IS NULL OR relation     = $3)
               AND ($4::text IS NULL OR subject_type = $4)
               AND ($5::text IS NULL OR subject_id   = $5)
             ORDER BY object_type, object_id, relation, subject_type, subject_id
             LIMIT $6 OFFSET $7",
        )
        .bind(&filter.object_type)
        .bind(&filter.object_id)
        .bind(&filter.relation)
        .bind(&filter.subject_type)
        .bind(&filter.subject_id)
        .bind(filter.effective_limit())
        .bind(filter.effective_offset())
        .fetch_all(&self.pool)
        .await
        .context("auth.zanzibar_tuples list")?;
        Ok(rows
            .into_iter()
            .map(|(ot, oid, rel, st, sid, srel)| Tuple {
                object_type: ot,
                object_id: oid,
                relation: rel,
                subject_type: st,
                subject_id: sid,
                subject_rel: srel,
            })
            .collect())
    }

    async fn check(
        &self,
        resource: &ObjectRef,
        permission: &str,
        subject: &SubjectRef,
        _consistency: Consistency,
    ) -> Result<CheckResult> {
        // No namespace defined → deny (the safe default). The full
        // permission algebra (union / intersect / exclude / arrow) is
        // composed by the backend-agnostic evaluator over the two
        // primitives this backend exposes (`check_relation_set` +
        // `arrow_targets`); a permission that flattens to a pure union
        // of relations still hits the single-CTE fast path inside it.
        let Some(schema) = self.get_namespace(&resource.object_type).await? else {
            return Ok(CheckResult::Denied);
        };
        evaluate(self, &schema, resource, permission, subject).await
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
                 AND w.subject_rel <> ''
                 AND t.relation = w.subject_rel
                WHERE w.depth < $4
                  AND NOT (t.subject_type || ':' || t.subject_id) = ANY(w.path)
            )
            SELECT DISTINCT subject_type, subject_id
            FROM walk
            WHERE subject_type = $5 AND subject_rel = ''
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

impl LeafCheck for PostgresZanzibarStore {
    fn check_relation_set<'a>(
        &'a self,
        object: &'a ObjectRef,
        relations: &'a [String],
        subject: &'a SubjectRef,
    ) -> Pin<Box<dyn Future<Output = Result<Verdict>> + Send + 'a>> {
        Box::pin(async move {
            if relations.is_empty() {
                return Ok(Verdict::Denied);
            }
            // subject_rel is NOT NULL — '' for direct subjects (the
            // terminal kind the leaf answers) and a relation name for
            // usersets the CTE walks through. Cycle guard via the in-CTE
            // `path` array — if the next subject is already on the path,
            // the join produces zero rows.
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
                     AND w.subject_rel <> ''
                     AND t.relation = w.subject_rel
                    WHERE w.depth < $4
                      AND NOT (t.subject_type || ':' || t.subject_id) = ANY(w.path)
                )
                SELECT CASE
                    WHEN EXISTS (
                        SELECT 1 FROM walk
                        WHERE subject_type = $5 AND subject_id = $6 AND subject_rel = ''
                    ) THEN 1
                    WHEN EXISTS (SELECT 1 FROM walk WHERE depth >= $4) THEN 2
                    ELSE 0
                END AS verdict
                "#,
            )
            .bind(&object.object_type)
            .bind(&object.object_id)
            .bind(relations)
            .bind(MAX_DEPTH as i32)
            .bind(&subject.subject_type)
            .bind(&subject.subject_id)
            .fetch_optional(&self.pool)
            .await
            .context("auth.zanzibar check CTE")?;

            Ok(match row.map(|(v,)| v).unwrap_or(0) {
                1 => Verdict::Allowed,
                2 => Verdict::DepthExceeded,
                _ => Verdict::Denied,
            })
        })
    }

    fn arrow_targets<'a>(
        &'a self,
        object: &'a ObjectRef,
        relation: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<ObjectRef>>> + Send + 'a>> {
        Box::pin(async move {
            // Only object-reference subjects (`subject_rel = ''`) are
            // the left edge of an arrow hop — a userset subject on the
            // tupleset relation has no object to re-evaluate against.
            let rows = sqlx::query(
                "SELECT subject_type, subject_id
                 FROM auth.zanzibar_tuples
                 WHERE object_type = $1 AND object_id = $2 AND relation = $3
                   AND subject_rel = ''",
            )
            .bind(&object.object_type)
            .bind(&object.object_id)
            .bind(relation)
            .fetch_all(&self.pool)
            .await
            .context("auth.zanzibar arrow targets")?;
            Ok(rows
                .into_iter()
                .map(|row| {
                    ObjectRef::new(
                        row.get::<String, _>("subject_type"),
                        row.get::<String, _>("subject_id"),
                    )
                })
                .collect())
        })
    }

    fn schema_for<'a>(
        &'a self,
        object_type: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Option<NamespaceSchema>>> + Send + 'a>> {
        Box::pin(async move { self.get_namespace(object_type).await })
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
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<UsersetTree>>> + Send + 'a>> {
    Box::pin(async move {
        if depth == 0 {
            return Ok(Vec::new());
        }
        let key = format!(
            "{}:{}#{}",
            resource.object_type, resource.object_id, relation
        );
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
            let sr: String = row.get("subject_rel");
            if sr.is_empty() {
                children.push(UsersetTree::Leaf {
                    subject: SubjectRef::direct(st, sid),
                });
            } else {
                let inner_resource = ObjectRef::new(st.clone(), sid.clone());
                let sub = expand_pg(store, &inner_resource, &sr, depth - 1, seen).await?;
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
