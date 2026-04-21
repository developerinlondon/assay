//! Migration runner for the SurrealDB backend.
//!
//! Migrations are embedded at compile time via `include_str!`. Each migration
//! is applied exactly once; the `_assay_migrations` table tracks which names
//! have been applied.

use super::SurrealDbStore;

const MIGRATIONS: &[(&str, &str)] = &[
    (
        "00_init",
        include_str!("../../../migrations/surrealdb/00_init.surql"),
    ),
    (
        "01_workflow_fields",
        include_str!("../../../migrations/surrealdb/01_workflow_fields.surql"),
    ),
];

impl SurrealDbStore {
    pub(crate) async fn run_migrations(&self) -> anyhow::Result<()> {
        // Ensure the tracker table exists.
        self.db
            .query(
                "DEFINE TABLE IF NOT EXISTS _assay_migrations SCHEMAFULL;
                 DEFINE FIELD IF NOT EXISTS name ON _assay_migrations TYPE string;
                 DEFINE FIELD IF NOT EXISTS applied_at ON _assay_migrations TYPE datetime DEFAULT time::now();
                 DEFINE INDEX IF NOT EXISTS name_unique ON _assay_migrations COLUMNS name UNIQUE;",
            )
            .await?;

        for (name, sql) in MIGRATIONS {
            // Check whether this migration was already applied.
            let applied: Vec<serde_json::Value> = self
                .db
                .query("SELECT name FROM _assay_migrations WHERE name = $name LIMIT 1")
                .bind(("name", name.to_string()))
                .await?
                .take(0)?;
            if !applied.is_empty() {
                continue;
            }

            // Apply the migration.
            self.db.query(*sql).await?;

            // Record it.
            self.db
                .query("CREATE _assay_migrations SET name = $name")
                .bind(("name", name.to_string()))
                .await?;
        }

        Ok(())
    }
}
