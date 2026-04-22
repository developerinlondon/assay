use crate::search::{SearchEngine, SearchResult};
use sqlx::sqlite::SqlitePoolOptions;
use sqlx::{Pool, Row, Sqlite};
use tokio::runtime::Runtime;

/// FTS5-backed search engine using SQLite's built-in full-text search.
///
/// Uses an in-memory SQLite database with FTS5 extension for BM25 ranking.
/// This is the high-quality search backend, enabled when the `db` feature is active.
///
/// Column weights for BM25 scoring:
/// - name: 2.0
/// - description: 1.0
/// - keywords: 3.0
/// - functions: 1.0
pub struct FTS5Index {
    pool: Pool<Sqlite>,
    rt: Runtime,
}

impl std::fmt::Debug for FTS5Index {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FTS5Index").finish_non_exhaustive()
    }
}

impl Default for FTS5Index {
    fn default() -> Self {
        Self::new()
    }
}

impl FTS5Index {
    /// Create a new FTS5 search index backed by an in-memory SQLite database.
    ///
    /// Initializes the FTS5 virtual table with columns for document fields
    /// and unicode61 tokenization.
    pub fn new() -> Self {
        let rt = Runtime::new().expect("tokio runtime");
        let pool = rt.block_on(async {
            SqlitePoolOptions::new()
                .max_connections(1)
                .connect("sqlite::memory:")
                .await
                .expect("sqlite in-memory connection")
        });

        rt.block_on(async {
            sqlx::query(
                "CREATE VIRTUAL TABLE IF NOT EXISTS modules USING fts5(\
                 doc_id UNINDEXED, \
                 name, \
                 description, \
                 keywords, \
                 functions, \
                 tokenize=\"unicode61\"\
                 )",
            )
            .execute(&pool)
            .await
            .expect("create FTS5 table");
        });

        Self { pool, rt }
    }
}

/// Sanitize a query string for FTS5 by quoting each alphanumeric token.
///
/// This prevents FTS5 syntax errors from special characters (*, :, ^)
/// and reserved keywords (OR, AND, NOT, NEAR).
fn sanitize_fts5_query(query: &str) -> String {
    query
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|s| !s.is_empty())
        .map(|s| format!("\"{s}\""))
        .collect::<Vec<_>>()
        .join(" ")
}

impl SearchEngine for FTS5Index {
    fn add_document(&mut self, id: &str, fields: &[(&str, &str, f64)]) {
        let mut name_val = String::new();
        let mut desc_val = String::new();
        let mut kw_val = String::new();
        let mut func_val = String::new();

        for &(field_name, field_value, _) in fields {
            match field_name {
                "name" => name_val = field_value.to_string(),
                "description" => desc_val = field_value.to_string(),
                "keywords" => kw_val = field_value.to_string(),
                _ => func_val = field_value.to_string(),
            }
        }

        self.rt.block_on(async {
            sqlx::query(
                "INSERT INTO modules(doc_id, name, description, keywords, functions) \
                 VALUES (?, ?, ?, ?, ?)",
            )
            .bind(id)
            .bind(&name_val)
            .bind(&desc_val)
            .bind(&kw_val)
            .bind(&func_val)
            .execute(&self.pool)
            .await
            .expect("insert document");
        });
    }

    fn search(&self, query: &str, limit: usize) -> Vec<SearchResult> {
        if query.trim().is_empty() {
            return Vec::new();
        }

        let sanitized = sanitize_fts5_query(query);
        if sanitized.is_empty() {
            return Vec::new();
        }

        self.rt.block_on(async {
            // bm25 weights: doc_id=0 (UNINDEXED), name=2, description=1, keywords=3, functions=1
            // bm25() returns negative values; more negative = better match.
            // ORDER BY rank (ascending) puts best matches first.
            let rows = sqlx::query(
                "SELECT doc_id, bm25(modules, 0.0, 2.0, 1.0, 3.0, 1.0) as rank \
                 FROM modules WHERE modules MATCH ? ORDER BY rank LIMIT ?",
            )
            .bind(&sanitized)
            .bind(limit as i64)
            .fetch_all(&self.pool)
            .await;

            match rows {
                Ok(rows) => rows
                    .iter()
                    .map(|row| {
                        let id: String = row.get("doc_id");
                        let rank: f64 = row.get("rank");
                        SearchResult {
                            id,
                            score: -rank, // negate: higher positive = better match
                        }
                    })
                    .collect(),
                Err(_) => Vec::new(),
            }
        })
    }
}
