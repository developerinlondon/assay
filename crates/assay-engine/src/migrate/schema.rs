//! Reading the shape of both stores.
//!
//! Nothing here hard-codes a table list. The source's tables come from
//! each ATTACHed database's `sqlite_master`, the target's columns and
//! foreign keys from the Postgres catalog, so a module that adds a table
//! is carried without touching this file.

use anyhow::{Context, Result, bail};
use std::collections::{HashMap, HashSet};

/// A table to copy, named the same on both sides.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Table {
    pub schema: String,
    pub name: String,
}

impl std::fmt::Display for Table {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}", self.schema, self.name)
    }
}

/// One target column: its name and the Postgres type to bind for.
#[derive(Debug, Clone)]
pub struct Column {
    pub name: String,
    pub udt: String,
}

/// Every table in the SQLite store, in the order its module was attached.
pub async fn source_tables(pool: &sqlx::SqlitePool, modules: &[&str]) -> Result<Vec<Table>> {
    let mut tables = Vec::new();
    for module in modules {
        let sql = format!(
            "SELECT name FROM {module}.sqlite_master
             WHERE type = 'table' AND name NOT LIKE 'sqlite_%'
             ORDER BY name"
        );
        let names: Vec<(String,)> = sqlx::query_as(&sql)
            .fetch_all(pool)
            .await
            .with_context(|| format!("list tables in sqlite database {module}"))?;
        tables.extend(names.into_iter().map(|(name,)| Table {
            schema: (*module).to_string(),
            name,
        }));
    }
    Ok(tables)
}

/// Column names of a SQLite table, in declaration order. The second
/// argument to `pragma_table_info` selects the ATTACHed database.
pub async fn source_columns(pool: &sqlx::SqlitePool, table: &Table) -> Result<Vec<String>> {
    let rows: Vec<(String,)> = sqlx::query_as("SELECT name FROM pragma_table_info(?, ?)")
        .bind(&table.name)
        .bind(&table.schema)
        .fetch_all(pool)
        .await
        .with_context(|| format!("read columns of {table}"))?;
    Ok(rows.into_iter().map(|(n,)| n).collect())
}

/// Columns of a Postgres table, in ordinal order. Empty when the table
/// does not exist.
pub async fn target_columns(pool: &sqlx::PgPool, table: &Table) -> Result<Vec<Column>> {
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT column_name, udt_name FROM information_schema.columns
         WHERE table_schema = $1 AND table_name = $2
         ORDER BY ordinal_position",
    )
    .bind(&table.schema)
    .bind(&table.name)
    .fetch_all(pool)
    .await
    .with_context(|| format!("read postgres columns of {table}"))?;
    Ok(rows
        .into_iter()
        .map(|(name, udt)| Column { name, udt })
        .collect())
}

/// Tables in the target's engine schemas that already hold rows.
///
/// Run before any DDL: a target the engine has never touched has no such
/// schemas at all, and that is the only state a migration may write into.
pub async fn non_empty_tables(pool: &sqlx::PgPool, schemas: &[&str]) -> Result<Vec<(Table, i64)>> {
    let owned: Vec<String> = schemas.iter().map(|s| (*s).to_string()).collect();
    let existing: Vec<(String, String)> = sqlx::query_as(
        "SELECT table_schema, table_name FROM information_schema.tables
         WHERE table_schema = ANY($1) AND table_type = 'BASE TABLE'
         ORDER BY table_schema, table_name",
    )
    .bind(&owned)
    .fetch_all(pool)
    .await
    .context("list existing tables in the target")?;

    let mut occupied = Vec::new();
    for (schema, name) in existing {
        let table = Table { schema, name };
        let count = count_rows_pg(pool, &table).await?;
        if count > 0 {
            occupied.push((table, count));
        }
    }
    Ok(occupied)
}

/// Schema-qualified, quoted table reference. Identical syntax on both
/// backends because the SQLite store ATTACHes one database per schema.
pub fn quoted(table: &Table) -> String {
    format!(r#""{}"."{}""#, table.schema, table.name)
}

fn count_sql(table: &Table) -> String {
    format!("SELECT COUNT(*) FROM {}", quoted(table))
}

pub async fn count_rows_pg(pool: &sqlx::PgPool, table: &Table) -> Result<i64> {
    let count = sqlx::query_scalar(&count_sql(table)).fetch_one(pool).await;
    count.with_context(|| format!("count rows of {table}"))
}

pub async fn count_rows_sqlite(pool: &sqlx::SqlitePool, table: &Table) -> Result<i64> {
    let count = sqlx::query_scalar(&count_sql(table)).fetch_one(pool).await;
    count.with_context(|| format!("count rows of {table}"))
}

/// Order `tables` so that every table follows the tables it references.
///
/// Insert order has to respect foreign keys, and the graph is read from
/// the target catalog rather than declared here so a new reference does
/// not need a matching edit.
pub async fn insert_order(pool: &sqlx::PgPool, tables: &[Table]) -> Result<Vec<Table>> {
    let schemas: Vec<String> = tables
        .iter()
        .map(|t| t.schema.clone())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    let edges: Vec<(String, String, String, String)> = sqlx::query_as(
        "SELECT ns.nspname, cl.relname, fns.nspname, fcl.relname
         FROM pg_constraint c
         JOIN pg_class cl ON cl.oid = c.conrelid
         JOIN pg_namespace ns ON ns.oid = cl.relnamespace
         JOIN pg_class fcl ON fcl.oid = c.confrelid
         JOIN pg_namespace fns ON fns.oid = fcl.relnamespace
         WHERE c.contype = 'f' AND ns.nspname = ANY($1)",
    )
    .bind(&schemas)
    .fetch_all(pool)
    .await
    .context("read foreign keys from the target")?;

    let present: HashSet<Table> = tables.iter().cloned().collect();
    let mut blockers: HashMap<Table, HashSet<Table>> =
        tables.iter().map(|t| (t.clone(), HashSet::new())).collect();
    for (schema, name, ref_schema, ref_name) in edges {
        let child = Table { schema, name };
        let parent = Table {
            schema: ref_schema,
            name: ref_name,
        };
        if child == parent || !present.contains(&child) || !present.contains(&parent) {
            continue;
        }
        blockers.entry(child).or_default().insert(parent);
    }

    let mut ordered: Vec<Table> = Vec::with_capacity(tables.len());
    let mut placed: HashSet<Table> = HashSet::new();
    while ordered.len() < tables.len() {
        let ready: Vec<Table> = tables
            .iter()
            .filter(|t| !placed.contains(*t))
            .filter(|t| blockers[*t].iter().all(|p| placed.contains(p)))
            .cloned()
            .collect();
        if ready.is_empty() {
            let stuck: Vec<String> = tables
                .iter()
                .filter(|t| !placed.contains(*t))
                .map(|t| t.to_string())
                .collect();
            bail!("foreign keys form a cycle across {}", stuck.join(", "));
        }
        for table in ready {
            placed.insert(table.clone());
            ordered.push(table);
        }
    }
    Ok(ordered)
}

/// Re-point every sequence at the ids that were just copied in.
///
/// Copying preserves ids, so a sequence still sitting at 1 would hand
/// the next insert an id the migrated rows already own.
pub async fn resync_sequences(pool: &sqlx::PgPool, tables: &[Table]) -> Result<usize> {
    let mut resynced = 0;
    for table in tables {
        for column in target_columns(pool, table).await? {
            let qualified = format!(r#""{}"."{}""#, table.schema, table.name);
            let sequence: Option<String> = sqlx::query_scalar("SELECT pg_get_serial_sequence($1, $2)")
                .bind(&qualified)
                .bind(&column.name)
                .fetch_one(pool)
                .await
                .with_context(|| format!("resolve sequence for {table}.{}", column.name))?;
            let Some(sequence) = sequence else { continue };
            let sql = format!(
                r#"SELECT setval('{sequence}', COALESCE((SELECT MAX("{}") FROM {qualified}), 0) + 1, false)"#,
                column.name
            );
            sqlx::query(&sql)
                .execute(pool)
                .await
                .with_context(|| format!("resync sequence {sequence}"))?;
            resynced += 1;
        }
    }
    Ok(resynced)
}
