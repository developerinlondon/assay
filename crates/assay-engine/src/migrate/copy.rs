//! Copying one table's rows from the SQLite store into Postgres.

use anyhow::{Context, Result, bail};
use sqlx::Row;
use std::collections::HashMap;

use super::schema::{Column, Table, quoted, source_columns, target_columns};
use super::value::{Bound, coerce, pg_cast, read_cell};

/// Postgres refuses more than 65535 bind parameters in one statement.
/// Rows per batch are derived from the column count to stay well under
/// it whatever the table's width.
const PARAMS_PER_BATCH: usize = 30_000;
const MAX_ROWS_PER_BATCH: usize = 1_000;

/// Copy every row of `table`, preserving ids and timestamps. Returns the
/// number of rows written.
pub async fn copy_table(
    source: &sqlx::SqlitePool,
    target: &sqlx::PgPool,
    table: &Table,
) -> Result<i64> {
    let columns = shared_columns(source, target, table).await?;
    if columns.is_empty() {
        bail!("{table} has no columns in common between the two stores");
    }

    let rows_per_batch =
        (PARAMS_PER_BATCH / columns.len()).clamp(1, MAX_ROWS_PER_BATCH);
    let select = format!(
        "SELECT rowid, {} FROM {} WHERE rowid > ? ORDER BY rowid LIMIT {}",
        columns
            .iter()
            .map(|c| format!(r#""{}""#, c.name))
            .collect::<Vec<_>>()
            .join(", "),
        quoted(table),
        rows_per_batch,
    );

    let mut cursor: i64 = 0;
    let mut copied: i64 = 0;
    loop {
        let rows = sqlx::query(&select)
            .bind(cursor)
            .fetch_all(source)
            .await
            .with_context(|| format!("read rows of {table}"))?;
        if rows.is_empty() {
            return Ok(copied);
        }

        let mut bounds: Vec<Bound> = Vec::with_capacity(rows.len() * columns.len());
        for row in &rows {
            cursor = row.try_get::<i64, _>(0)?;
            for (offset, column) in columns.iter().enumerate() {
                let cell = read_cell(row, offset + 1)
                    .with_context(|| format!("{table}.{}", column.name))?;
                bounds.push(coerce(cell, &column.udt, &format!("{table}.{}", column.name))?);
            }
        }

        let statement = insert_statement(table, &columns, rows.len());
        let mut query = sqlx::query(&statement);
        for bound in bounds {
            query = match bound {
                Bound::Text(v) => query.bind(v),
                Bound::Bool(v) => query.bind(v),
                Bound::I32(v) => query.bind(v),
                Bound::I64(v) => query.bind(v),
                Bound::F64(v) => query.bind(v),
                Bound::Bytes(v) => query.bind(v),
                Bound::TextArray(v) => query.bind(v),
            };
        }
        query
            .execute(target)
            .await
            .with_context(|| format!("insert into {table}"))?;
        copied += rows.len() as i64;
    }
}

/// Columns present on both sides, in the source's order.
///
/// A column the source has and the target does not would be dropped in
/// silence, so it is an error. The reverse is fine: a target column the
/// source predates takes its default.
async fn shared_columns(
    source: &sqlx::SqlitePool,
    target: &sqlx::PgPool,
    table: &Table,
) -> Result<Vec<Column>> {
    let src = source_columns(source, table).await?;
    let dst = target_columns(target, table).await?;
    if dst.is_empty() {
        bail!("{table} exists in the source store but not in the target");
    }
    let by_name: HashMap<&str, &Column> = dst.iter().map(|c| (c.name.as_str(), c)).collect();

    let mut shared = Vec::with_capacity(src.len());
    let mut missing = Vec::new();
    for name in &src {
        match by_name.get(name.as_str()) {
            Some(column) => shared.push((*column).clone()),
            None => missing.push(name.clone()),
        }
    }
    if !missing.is_empty() {
        bail!(
            "{table}: the target has no column {} — migrating would drop it",
            missing.join(", ")
        );
    }
    Ok(shared)
}

fn insert_statement(table: &Table, columns: &[Column], rows: usize) -> String {
    let names = columns
        .iter()
        .map(|c| format!(r#""{}""#, c.name))
        .collect::<Vec<_>>()
        .join(", ");
    let mut placeholder = 0;
    let tuples: Vec<String> = (0..rows)
        .map(|_| {
            let cells: Vec<String> = columns
                .iter()
                .map(|c| {
                    placeholder += 1;
                    format!("${placeholder}::{}", pg_cast(&c.udt))
                })
                .collect();
            format!("({})", cells.join(", "))
        })
        .collect();
    format!(
        "INSERT INTO {} ({names}) VALUES {}",
        quoted(table),
        tuples.join(", ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn column(name: &str, udt: &str) -> Column {
        Column {
            name: name.to_string(),
            udt: udt.to_string(),
        }
    }

    #[test]
    fn placeholders_are_numbered_across_every_row_of_the_batch() {
        let table = Table {
            schema: "auth".to_string(),
            name: "users".to_string(),
        };
        let columns = vec![column("id", "text"), column("created_at", "float8")];
        let sql = insert_statement(&table, &columns, 2);
        assert_eq!(
            sql,
            r#"INSERT INTO "auth"."users" ("id", "created_at") VALUES ($1::text, $2::float8), ($3::text, $4::float8)"#
        );
    }

    #[test]
    fn array_columns_keep_their_bracket_cast() {
        let table = Table {
            schema: "engine".to_string(),
            name: "instances".to_string(),
        };
        let sql = insert_statement(&table, &[column("namespaces", "_text")], 1);
        assert!(sql.ends_with("VALUES ($1::text[])"), "{sql}");
    }
}
