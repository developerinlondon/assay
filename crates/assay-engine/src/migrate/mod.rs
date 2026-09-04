//! Moving the engine's store from SQLite to Postgres.
//!
//! The store holds the workflow runtime, the auth module's users and
//! keys, and the vault — master KEK included — so all of it is copied
//! with ids and timestamps preserved. A target that already holds rows
//! is refused: merging would have to reconcile ids unique to one side.

mod copy;
mod schema;
mod value;

use anyhow::{Context, Result, bail};

pub use schema::Table;

/// Where the store is moving from and to.
pub struct Plan {
    pub source_dir: String,
    pub target_url: String,
    pub dry_run: bool,
}

/// What a run copied, per table.
pub struct Report {
    pub counts: Vec<TableCount>,
    /// Source tables Postgres has no counterpart for. `engine.lock` is
    /// the standing case: SQLite serialises single-instance access
    /// through a table, Postgres through an advisory lock.
    pub skipped: Vec<TableCount>,
    pub sequences: usize,
    pub dry_run: bool,
}

pub struct TableCount {
    pub table: Table,
    pub source: i64,
    pub target: i64,
}

/// Accept `sqlite:<dir>`, `sqlite://<dir>` or a bare directory path, so
/// the flag takes either the config's backend URL or the data directory
/// an operator can see on the volume.
pub fn parse_source(raw: &str) -> Result<String> {
    let dir = raw
        .strip_prefix("sqlite://")
        .or_else(|| raw.strip_prefix("sqlite:"))
        .unwrap_or(raw);
    if dir.is_empty() {
        bail!("--from needs a data directory, e.g. sqlite:/var/lib/assay/data");
    }
    Ok(dir.to_string())
}

pub fn parse_target(raw: &str) -> Result<String> {
    if !raw.starts_with("postgres://") && !raw.starts_with("postgresql://") {
        bail!("--to must be a postgres:// URL; got {raw}");
    }
    Ok(raw.to_string())
}

pub async fn run(plan: Plan) -> Result<Report> {
    let source = crate::init::sqlite_pool(&plan.source_dir).await?;
    let tables = schema::source_tables(&source, crate::init::SQLITE_MODULE_DBS).await?;
    if tables.is_empty() {
        bail!(
            "{} holds no engine tables — check the data directory",
            plan.source_dir
        );
    }

    let target = sqlx::PgPool::connect(&plan.target_url)
        .await
        .context("connect to the target postgres")?;
    refuse_non_empty(&target).await?;

    if plan.dry_run {
        let mut counts = Vec::new();
        for table in tables {
            let source = schema::count_rows_sqlite(&source, &table).await?;
            counts.push(TableCount {
                table,
                source,
                target: 0,
            });
        }
        return Ok(Report {
            counts,
            skipped: Vec::new(),
            sequences: 0,
            dry_run: true,
        });
    }

    prepare_target(&target, &modules_holding_tables(&tables)).await?;

    let mut migratable = Vec::new();
    let mut skipped = Vec::new();
    for table in tables {
        if schema::target_columns(&target, &table).await?.is_empty() {
            let source = schema::count_rows_sqlite(&source, &table).await?;
            skipped.push(TableCount {
                table,
                source,
                target: 0,
            });
        } else {
            migratable.push(table);
        }
    }

    let ordered = schema::insert_order(&target, &migratable).await?;
    clear_bootstrap_rows(&target, &ordered).await?;
    for table in &ordered {
        copy::copy_table(&source, &target, table).await?;
    }
    let sequences = schema::resync_sequences(&target, &ordered).await?;

    let mut counts = Vec::new();
    for table in &ordered {
        let source_rows = schema::count_rows_sqlite(&source, table).await?;
        let target_rows = schema::count_rows_pg(&target, table).await?;
        if source_rows != target_rows {
            bail!("{table}: copied {target_rows} rows but the source holds {source_rows}");
        }
        counts.push(TableCount {
            table: table.clone(),
            source: source_rows,
            target: target_rows,
        });
    }
    counts.sort_by_key(|c| c.table.to_string());

    Ok(Report {
        counts,
        skipped,
        sequences,
        dry_run: false,
    })
}

/// Modules that have tables in the source. Deliberately not
/// `engine.modules`: a module disabled there can still hold rows from
/// when it was on, which the manifest would leave behind.
fn modules_holding_tables(tables: &[Table]) -> Vec<String> {
    let mut modules: Vec<String> = tables
        .iter()
        .map(|t| t.schema.clone())
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
    modules.sort();
    modules
}

async fn refuse_non_empty(target: &sqlx::PgPool) -> Result<()> {
    let occupied = schema::non_empty_tables(target, crate::init::SQLITE_MODULE_DBS).await?;
    if occupied.is_empty() {
        return Ok(());
    }
    let listed = occupied
        .iter()
        .map(|(table, rows)| format!("  {table} ({rows} rows)"))
        .collect::<Vec<_>>()
        .join("\n");
    bail!(
        "the target already holds engine data and will not be migrated into:\n{listed}\n\
         Point --to at an empty database."
    );
}

/// Create every schema and table the target needs, exactly as a first
/// boot would, so the copy has somewhere to land.
async fn prepare_target(target: &sqlx::PgPool, modules: &[String]) -> Result<()> {
    let schema = assay_domain::engine::PgEngineSchema::new(target.clone());
    schema
        .migrate()
        .await
        .map_err(|e| anyhow::anyhow!("engine schema migrate: {e}"))?;

    let mut tx = target.begin().await.context("begin schema tx")?;
    assay_domain::engine::acquire_schema_lock(&mut tx).await?;
    for name in modules {
        sqlx::query(&format!("CREATE SCHEMA IF NOT EXISTS {name}"))
            .execute(&mut *tx)
            .await
            .with_context(|| format!("create schema {name}"))?;
    }
    tx.commit().await.context("commit schema tx")?;

    if modules.iter().any(|m| m == "workflow") {
        assay_workflow::PostgresStore::from_pool(target.clone())
            .await
            .map_err(|e| anyhow::anyhow!("workflow schema migrate: {e}"))?;
    }
    if modules.iter().any(|m| m == "auth") {
        assay_auth::schema::migrate_postgres(target)
            .await
            .context("auth schema migrate")?;
    }
    if modules.iter().any(|m| m == "vault") {
        prepare_vault(target).await?;
    }
    Ok(())
}

#[cfg(feature = "vault")]
async fn prepare_vault(target: &sqlx::PgPool) -> Result<()> {
    assay_vault::schema::migrate_postgres(target)
        .await
        .context("vault schema migrate")
}

/// Refusing here rather than skipping: the vault holds the store's
/// secrets, and a build without it cannot carry them.
#[cfg(not(feature = "vault"))]
async fn prepare_vault(_target: &sqlx::PgPool) -> Result<()> {
    bail!(
        "the source store holds vault tables but this binary was \
         built without vault support; migrate with a build that has it"
    )
}

/// Empty the rows the schema bootstrap seeded — the `main` namespace and
/// the migration ledger — so the source's own copies land on a clean
/// table and the row counts can be compared.
async fn clear_bootstrap_rows(target: &sqlx::PgPool, tables: &[Table]) -> Result<()> {
    let list = tables
        .iter()
        .map(schema::quoted)
        .collect::<Vec<_>>()
        .join(", ");
    sqlx::query(&format!("TRUNCATE {list} RESTART IDENTITY CASCADE"))
        .execute(target)
        .await
        .context("clear the freshly created target tables")?;
    Ok(())
}

impl Report {
    /// Per-table counts, widest column first so the output lines up.
    pub fn render(&self) -> String {
        let width = self
            .counts
            .iter()
            .map(|c| c.table.to_string().len())
            .max()
            .unwrap_or(5)
            .max(5);
        let mut out = String::new();
        let header = if self.dry_run { "rows" } else { "copied" };
        out.push_str(&format!("{:<width$}  {:>10}\n", "table", header));
        out.push_str(&format!("{}  {}\n", "-".repeat(width), "-".repeat(10)));
        let mut total = 0;
        for count in &self.counts {
            let rows = if self.dry_run { count.source } else { count.target };
            total += rows;
            out.push_str(&format!(
                "{:<width$}  {rows:>10}\n",
                count.table.to_string()
            ));
        }
        out.push_str(&format!("{}  {}\n", "-".repeat(width), "-".repeat(10)));
        out.push_str(&format!("{:<width$}  {total:>10}\n", "total"));
        for skip in &self.skipped {
            out.push_str(&format!(
                "\nskipped {} ({} rows): no Postgres counterpart.\n",
                skip.table, skip.source
            ));
        }
        if self.dry_run {
            out.push_str(
                "\nDry run: nothing was written. Source tables with no Postgres \
                 counterpart are skipped on the real run.\n",
            );
        } else {
            out.push_str(&format!(
                "\n{} sequences re-pointed past the copied ids.\n",
                self.sequences
            ));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_accepts_a_url_or_a_bare_directory() {
        assert_eq!(parse_source("sqlite:/var/lib/assay").unwrap(), "/var/lib/assay");
        assert_eq!(parse_source("sqlite:///var/lib/assay").unwrap(), "/var/lib/assay");
        assert_eq!(parse_source("./data").unwrap(), "./data");
    }

    #[test]
    fn target_must_be_postgres() {
        let err = parse_target("sqlite:/tmp/other").unwrap_err();
        assert!(err.to_string().contains("postgres://"), "{err}");
    }
}
