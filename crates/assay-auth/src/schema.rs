//! Auth module schema bootstrap + migration runner.
//!
//! Provides `migrate_*` entrypoints the engine boot path (in
//! `crates/assay-engine/src/init.rs`) calls when `engine.modules` shows
//! `auth` enabled. Each entrypoint:
//!
//! 1. Ensures the storage container exists. PG: `CREATE SCHEMA IF NOT
//!    EXISTS auth`. SQLite: relies on the engine boot having ATTACHed
//!    `data/auth.db` AS `auth` — the migration runs DDL into the
//!    attachment.
//! 2. Applies every DDL statement up to the current
//!    [`MIGRATION_VERSION`] for tables in this module.
//! 3. Records the applied version into `engine.migrations` with
//!    `module = MODULE_NAME` so subsequent boots skip already-applied
//!    versions.
//!
//! The migration is idempotent — every CREATE uses `IF NOT EXISTS`,
//! every INSERT into `engine.migrations` uses `ON CONFLICT DO NOTHING`.
//! Re-running on a healthy DB is a no-op.
//!
//! Tables created (per plan 12c with v0.1.2 schema-qualifying applied):
//!
//! - `auth.users` — authoritative user records (id, email,
//!   password_hash, …)
//! - `auth.user_upstream` — federated identity links (provider/subject
//!   tuples → user_id)
//! - `auth.passkeys` — WebAuthn credentials per user
//! - `auth.sessions` — opaque session ids + CSRF tokens + expiry
//! - `auth.jwks_keys` — rotated JWT signing keys (active + history)
//! - `auth.audit` — append-only compliance log (deferred to a later
//!   phase — see Phase 4 notes)
//!
//! Auth does NOT write to `engine.events`; auth's real-time signal (if
//! ever needed) goes through its own channel on `auth.audit`.

/// Stable name registered in `engine.modules.name` and used as the
/// `module` discriminant in `engine.migrations`. Matches the schema
/// (PG) / attached-database (SQLite) name 1:1 so SQL stays readable.
pub const MODULE_NAME: &str = "auth";

/// Highest migration version this build knows about. Bumped each time
/// a new DDL pack is appended below. The runner records every version
/// up to and including this one into `engine.migrations`.
pub const MIGRATION_VERSION: i32 = 1;

/// Postgres DDL for the auth schema, version 1.
///
/// All tables are schema-qualified (`auth.*`) so they live in the
/// `auth` schema regardless of the connection's `search_path`. The
/// CREATE SCHEMA IF NOT EXISTS is included here for completeness even
/// though engine boot also runs it — both paths must work
/// independently for tests that bootstrap the auth schema directly.
///
/// `auth.audit` is intentionally deferred — the table is part of plan
/// 12c phase 4 task 4.6 step 1 but no caller writes to it yet, and
/// shipping the DDL without a writer risks confusing operators.
/// Phase 5/6 will add it alongside the first auditable action.
pub const PG_DDL_V1: &str = r#"
CREATE SCHEMA IF NOT EXISTS auth;

CREATE TABLE IF NOT EXISTS auth.users (
    id              TEXT PRIMARY KEY,
    email           TEXT UNIQUE,
    email_verified  BOOLEAN NOT NULL DEFAULT FALSE,
    display_name    TEXT,
    password_hash   TEXT,
    created_at      DOUBLE PRECISION NOT NULL
);

CREATE TABLE IF NOT EXISTS auth.user_upstream (
    provider    TEXT NOT NULL,
    subject     TEXT NOT NULL,
    user_id     TEXT NOT NULL REFERENCES auth.users(id) ON DELETE CASCADE,
    PRIMARY KEY (provider, subject)
);
CREATE INDEX IF NOT EXISTS idx_auth_user_upstream_user
    ON auth.user_upstream (user_id);

CREATE TABLE IF NOT EXISTS auth.passkeys (
    credential_id   BYTEA PRIMARY KEY,
    user_id         TEXT NOT NULL REFERENCES auth.users(id) ON DELETE CASCADE,
    public_key      BYTEA NOT NULL,
    sign_count      INTEGER NOT NULL DEFAULT 0,
    transports      TEXT NOT NULL,
    created_at      DOUBLE PRECISION NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_auth_passkeys_user
    ON auth.passkeys (user_id);

CREATE TABLE IF NOT EXISTS auth.sessions (
    id                  TEXT PRIMARY KEY,
    user_id             TEXT NOT NULL REFERENCES auth.users(id) ON DELETE CASCADE,
    csrf_token          TEXT NOT NULL,
    created_at          DOUBLE PRECISION NOT NULL,
    expires_at          DOUBLE PRECISION NOT NULL,
    ip_hash             TEXT,
    user_agent_hash     TEXT
);
CREATE INDEX IF NOT EXISTS idx_auth_sessions_user
    ON auth.sessions (user_id);
CREATE INDEX IF NOT EXISTS idx_auth_sessions_expires
    ON auth.sessions (expires_at);

CREATE TABLE IF NOT EXISTS auth.jwks_keys (
    kid                     TEXT PRIMARY KEY,
    alg                     TEXT NOT NULL,
    public_jwk              JSONB NOT NULL,
    private_pem_encrypted   BYTEA,
    created_at              DOUBLE PRECISION NOT NULL,
    rotated_at              DOUBLE PRECISION,
    expires_at              DOUBLE PRECISION
);
CREATE INDEX IF NOT EXISTS idx_auth_jwks_keys_active
    ON auth.jwks_keys (rotated_at) WHERE rotated_at IS NULL;
"#;

/// SQLite DDL for the auth schema, version 1.
///
/// Caller must have ATTACHed `data/auth.db` AS `auth` before running
/// this — engine boot is responsible for that wiring (matches the
/// pattern already used for the engine + workflow attachments). The
/// DDL itself uses unqualified table names because SQLite CREATE
/// TABLE doesn't accept the `schema.table` form for the table itself
/// when CREATE INDEX … ON table is used unqualified; we therefore
/// build per-statement queries that prefix the schema explicitly.
///
/// Mirrors the PG layout: `BYTEA` → `BLOB`, `BOOLEAN` → `INTEGER`,
/// `JSONB` → `TEXT` (JSON-encoded), `DOUBLE PRECISION` → `REAL`.
pub const SQLITE_DDL_V1: &[(&str, &str)] = &[
    (
        "users",
        "CREATE TABLE IF NOT EXISTS auth.users (
            id              TEXT PRIMARY KEY,
            email           TEXT UNIQUE,
            email_verified  INTEGER NOT NULL DEFAULT 0,
            display_name    TEXT,
            password_hash   TEXT,
            created_at      REAL NOT NULL
        )",
    ),
    (
        "user_upstream",
        "CREATE TABLE IF NOT EXISTS auth.user_upstream (
            provider    TEXT NOT NULL,
            subject     TEXT NOT NULL,
            user_id     TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
            PRIMARY KEY (provider, subject)
        )",
    ),
    (
        "idx_user_upstream_user",
        "CREATE INDEX IF NOT EXISTS auth.idx_auth_user_upstream_user \
         ON user_upstream (user_id)",
    ),
    (
        "passkeys",
        "CREATE TABLE IF NOT EXISTS auth.passkeys (
            credential_id   BLOB PRIMARY KEY,
            user_id         TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
            public_key      BLOB NOT NULL,
            sign_count      INTEGER NOT NULL DEFAULT 0,
            transports      TEXT NOT NULL,
            created_at      REAL NOT NULL
        )",
    ),
    (
        "idx_passkeys_user",
        "CREATE INDEX IF NOT EXISTS auth.idx_auth_passkeys_user ON passkeys (user_id)",
    ),
    (
        "sessions",
        "CREATE TABLE IF NOT EXISTS auth.sessions (
            id                  TEXT PRIMARY KEY,
            user_id             TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
            csrf_token          TEXT NOT NULL,
            created_at          REAL NOT NULL,
            expires_at          REAL NOT NULL,
            ip_hash             TEXT,
            user_agent_hash     TEXT
        )",
    ),
    (
        "idx_sessions_user",
        "CREATE INDEX IF NOT EXISTS auth.idx_auth_sessions_user ON sessions (user_id)",
    ),
    (
        "idx_sessions_expires",
        "CREATE INDEX IF NOT EXISTS auth.idx_auth_sessions_expires ON sessions (expires_at)",
    ),
    (
        "jwks_keys",
        "CREATE TABLE IF NOT EXISTS auth.jwks_keys (
            kid                     TEXT PRIMARY KEY,
            alg                     TEXT NOT NULL,
            public_jwk              TEXT NOT NULL,
            private_pem_encrypted   BLOB,
            created_at              REAL NOT NULL,
            rotated_at              REAL,
            expires_at              REAL
        )",
    ),
    (
        "idx_jwks_keys_active",
        "CREATE INDEX IF NOT EXISTS auth.idx_auth_jwks_keys_active \
         ON jwks_keys (rotated_at) WHERE rotated_at IS NULL",
    ),
];

/// Postgres migration runner.
///
/// Splits [`PG_DDL_V1`] into individual statements (sqlx requires one
/// statement per `query`), executes each, then records
/// `(MODULE_NAME, MIGRATION_VERSION)` into `engine.migrations`.
#[cfg(feature = "backend-postgres")]
pub async fn migrate_postgres(pool: &sqlx::PgPool) -> anyhow::Result<()> {
    use anyhow::Context;
    for stmt in split_pg_statements(PG_DDL_V1) {
        sqlx::query(&stmt)
            .execute(pool)
            .await
            .with_context(|| format!("auth pg migrate: {}", first_line(&stmt)))?;
    }
    sqlx::query(
        "INSERT INTO engine.migrations (module, version) VALUES ($1, $2) \
         ON CONFLICT DO NOTHING",
    )
    .bind(MODULE_NAME)
    .bind(MIGRATION_VERSION)
    .execute(pool)
    .await
    .context("record auth migration in engine.migrations")?;
    Ok(())
}

/// SQLite migration runner.
///
/// Caller must have ATTACHed the auth database as `auth` before
/// calling. Each DDL chunk is executed as its own statement; the
/// per-table failure context names the table that broke so engine
/// boot logs are actionable.
#[cfg(feature = "backend-sqlite")]
pub async fn migrate_sqlite(pool: &sqlx::SqlitePool) -> anyhow::Result<()> {
    use anyhow::Context;
    for (label, stmt) in SQLITE_DDL_V1 {
        sqlx::query(stmt)
            .execute(pool)
            .await
            .with_context(|| format!("auth sqlite migrate: {label}"))?;
    }
    sqlx::query(
        "INSERT OR IGNORE INTO engine.migrations (module, version) VALUES (?, ?)",
    )
    .bind(MODULE_NAME)
    .bind(MIGRATION_VERSION)
    .execute(pool)
    .await
    .context("record auth migration in engine.migrations")?;
    Ok(())
}

/// Split a PG DDL chunk into individual statements. Drops pure-comment
/// lines first so a `--`-introduced semicolon doesn't fragment a real
/// statement (mirrors the same trick `assay-workflow::store::postgres`
/// uses for its larger SCHEMA constant).
#[cfg(feature = "backend-postgres")]
fn split_pg_statements(schema: &str) -> Vec<String> {
    let cleaned: String = schema
        .lines()
        .filter(|line| !line.trim_start().starts_with("--"))
        .collect::<Vec<_>>()
        .join("\n");
    cleaned
        .split(';')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

#[cfg(feature = "backend-postgres")]
fn first_line(stmt: &str) -> String {
    stmt.lines()
        .next()
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_name_is_stable() {
        // Locked-in: appears in `engine.modules`, in `engine.migrations.module`,
        // and as the SQLite ATTACH alias / PG schema name. Renaming it is a
        // breaking storage change; gate it behind a real version bump.
        assert_eq!(MODULE_NAME, "auth");
    }

    #[cfg(feature = "backend-postgres")]
    #[test]
    fn pg_split_drops_pure_comment_lines_and_empty_fragments() {
        let sql = "-- top\nCREATE TABLE a(x INT);\n-- mid\nCREATE INDEX i ON a(x);\n";
        let stmts = split_pg_statements(sql);
        assert_eq!(stmts.len(), 2);
        assert!(stmts[0].starts_with("CREATE TABLE"));
        assert!(stmts[1].starts_with("CREATE INDEX"));
    }

    #[cfg(feature = "backend-postgres")]
    #[test]
    fn pg_ddl_v1_split_is_nonempty() {
        let stmts = split_pg_statements(PG_DDL_V1);
        // Sanity: schema + 5 tables + several indexes worth of statements.
        assert!(stmts.len() >= 6, "got {} statements", stmts.len());
        assert!(stmts.iter().any(|s| s.starts_with("CREATE SCHEMA")));
        assert!(stmts.iter().any(|s| s.contains("auth.users")));
        assert!(stmts.iter().any(|s| s.contains("auth.sessions")));
        assert!(stmts.iter().any(|s| s.contains("auth.jwks_keys")));
    }
}
