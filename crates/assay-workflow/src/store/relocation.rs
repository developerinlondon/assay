//! Moving a v0.13.1 store into the `workflow` and `engine` schemas.
//!
//! `ALTER TABLE ... SET SCHEMA` cannot be undone by reverting a deploy,
//! and `workflows`, `namespaces` and `api_keys` are names any
//! application might own, so nothing moves until the database proves
//! the tables are the engine's own.

use anyhow::Result;
use sqlx::PgConnection;
use tracing::{info, warn};

/// A table whose v0.13.1 name is unmistakably the engine's. No
/// application is plausibly holding `public.workflow_activities`.
const PREFIXED: &[(&str, &str, &str)] = &[
    ("workflow_events", "workflow", "events"),
    ("workflow_activities", "workflow", "activities"),
    ("workflow_timers", "workflow", "timers"),
    ("workflow_signals", "workflow", "signals"),
    ("workflow_snapshots", "workflow", "snapshots"),
    ("workflow_schedules", "workflow", "schedules"),
    ("workflow_workers", "workflow", "workers"),
    ("engine_events", "engine", "events"),
];

/// Tables whose v0.13.1 name is one an application might also use, with
/// the columns the engine's own version of that table always had. Both
/// the provenance marker and this column set must hold before the table
/// moves.
const AMBIGUOUS: &[(&str, &str, &str, &[&str])] = &[
    (
        "workflows",
        "workflow",
        "workflows",
        &["id", "run_id", "workflow_type", "task_queue", "status"],
    ),
    ("namespaces", "workflow", "namespaces", &["name", "created_at"]),
];

/// Presence of this table is what marks the database as a v0.13.1 engine
/// store. Every such store has it, its name belongs to no one else, and
/// it moves in the same transaction as the ambiguous tables — so it is
/// still present whenever any of them are.
const PROVENANCE_MARKER: &str = "workflow_events";

/// Legacy tables that no longer have a home. Reported, never dropped:
/// an orphaned table costs nothing, and dropping one the engine did not
/// create cannot be undone.
const ORPHANED: &[&str] = &["api_keys"];

async fn exists(conn: &mut PgConnection, table: &str) -> Result<bool> {
    let found: Option<String> =
        sqlx::query_scalar(&format!("SELECT to_regclass('public.{table}')::text"))
            .fetch_one(&mut *conn)
            .await?;
    Ok(found.is_some())
}

async fn columns(conn: &mut PgConnection, table: &str) -> Result<Vec<String>> {
    Ok(sqlx::query_scalar(
        "SELECT column_name FROM information_schema.columns
         WHERE table_schema = 'public' AND table_name = $1",
    )
    .bind(table)
    .fetch_all(&mut *conn)
    .await?)
}

/// Relocate a v0.13.1 store, or leave the database alone and say why.
pub async fn run(conn: &mut PgConnection) -> Result<()> {
    if !exists(&mut *conn, PROVENANCE_MARKER).await? {
        report_untouched(&mut *conn).await?;
        return Ok(());
    }
    info!(
        target: "assay-workflow",
        "v0.13.1 store detected; relocating its tables into the workflow and engine schemas"
    );

    for (old, schema, new) in PREFIXED {
        relocate(&mut *conn, old, schema, new).await?;
    }
    for (old, schema, new, required) in AMBIGUOUS {
        if !exists(&mut *conn, old).await? {
            continue;
        }
        let present = columns(&mut *conn, old).await?;
        let missing: Vec<&str> = required
            .iter()
            .copied()
            .filter(|c| !present.iter().any(|p| p == c))
            .collect();
        if missing.is_empty() {
            relocate(&mut *conn, old, schema, new).await?;
        } else {
            warn!(
                target: "assay-workflow",
                table = %old,
                missing = ?missing,
                "public.{old} is not this engine's table — leaving it alone"
            );
        }
    }
    for old in ORPHANED {
        if exists(&mut *conn, old).await? {
            warn!(
                target: "assay-workflow",
                "public.{old} is a retired engine table; it is no longer used and is left in place"
            );
        }
    }
    Ok(())
}

/// Nothing here is the engine's. Name any table that shares a legacy
/// name so an operator reading the log knows it was seen and spared.
async fn report_untouched(conn: &mut PgConnection) -> Result<()> {
    for (old, ..) in AMBIGUOUS {
        if exists(&mut *conn, old).await? {
            warn!(
                target: "assay-workflow",
                "public.{old} exists and is not this engine's; it is left untouched. \
                 Run the engine in its own database to avoid the name collision"
            );
        }
    }
    Ok(())
}

/// Drop the empty twin the schema bootstrap just created, then move the
/// legacy table into its place. CASCADE because the fresh twins carry
/// the foreign keys between them.
async fn relocate(conn: &mut PgConnection, old: &str, schema: &str, new: &str) -> Result<()> {
    if !exists(&mut *conn, old).await? {
        return Ok(());
    }
    sqlx::query(&format!("DROP TABLE IF EXISTS {schema}.{new} CASCADE"))
        .execute(&mut *conn)
        .await?;
    sqlx::query(&format!("ALTER TABLE public.{old} SET SCHEMA {schema}"))
        .execute(&mut *conn)
        .await?;
    if old != new {
        sqlx::query(&format!("ALTER TABLE {schema}.{old} RENAME TO {new}"))
            .execute(&mut *conn)
            .await?;
    }
    info!(target: "assay-workflow", "relocated public.{old} to {schema}.{new}");
    Ok(())
}
