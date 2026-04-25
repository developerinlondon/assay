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
///
/// V1 (phase 4): users / sessions / passkeys / user_upstream / jwks_keys.
/// V2 (phase 5): adds `auth.biscuit_root_keys` for the always-on
///               biscuit capability-token root key bootstrap.
/// V3 (phase 6): adds `auth.zanzibar_namespaces` + `auth.zanzibar_tuples`
///               for ReBAC. Recursive-CTE walk + reverse index for
///               Keto/SpiceDB-equivalent permission checks.
/// V4 (phase 7): adds the OIDC provider tables — `auth.oidc_clients`,
///               `auth.upstream_providers`, `auth.oidc_authorization_codes`,
///               `auth.oidc_refresh_tokens`, `auth.oidc_sessions`,
///               `auth.oidc_consents`, and `auth.oidc_upstream_states`.
///               Together they make `assay-engine` a conformant OIDC
///               provider (Hydra equivalent).
pub const MIGRATION_VERSION: i32 = 4;

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

/// Postgres DDL for the auth schema, version 2 — adds
/// `auth.biscuit_root_keys` for the always-on biscuit root key
/// bootstrap. The `private_pem` column is plaintext today; secret-at-rest
/// envelope is a later phase (matches the `auth.jwks_keys.private_pem_encrypted`
/// shape — same TODO surface).
pub const PG_DDL_V2: &str = r#"
CREATE TABLE IF NOT EXISTS auth.biscuit_root_keys (
    kid             TEXT PRIMARY KEY,
    private_pem     BYTEA NOT NULL,
    public_pem      TEXT NOT NULL,
    created_at      DOUBLE PRECISION NOT NULL,
    rotated_at      DOUBLE PRECISION
);
CREATE INDEX IF NOT EXISTS idx_auth_biscuit_root_keys_active
    ON auth.biscuit_root_keys (rotated_at) WHERE rotated_at IS NULL;
"#;

/// Postgres DDL for the auth schema, version 3 — Zanzibar / ReBAC.
///
/// Two tables:
///
/// - `auth.zanzibar_namespaces` — JSON-serialised
///   [`crate::zanzibar::NamespaceSchema`], one row per namespace
///   (`document`, `group`, `user`, …). The schema parser writes here on
///   `define_namespace`.
/// - `auth.zanzibar_tuples` — the relation-tuple table, the canonical
///   Zanzibar/Keto data model. Composite PK supports the forward
///   `(object, relation, *)` index for `check`; the auxiliary
///   `idx_auth_zanzibar_tuples_rev` covers
///   `(subject_type, subject_id, relation)` for reverse lookups
///   (`lookup_resources`, expand-from-subject paths).
///
/// `subject_rel` is `TEXT NULL` because direct subjects (e.g. a user)
/// have no relation, while userset subjects (e.g. `family:foo#member`)
/// carry one. PG treats NULL as distinct in PK comparisons, so the PK
/// alone is *not* a uniqueness guarantee for direct tuples — paired
/// with a partial unique index on the NULL case to get the same
/// dedup behaviour the SpiceDB schema mandates.
pub const PG_DDL_V3: &str = r#"
CREATE TABLE IF NOT EXISTS auth.zanzibar_namespaces (
    name        TEXT PRIMARY KEY,
    schema_json JSONB NOT NULL,
    updated_at  DOUBLE PRECISION NOT NULL DEFAULT EXTRACT(EPOCH FROM NOW())
);

CREATE TABLE IF NOT EXISTS auth.zanzibar_tuples (
    object_type  TEXT NOT NULL,
    object_id    TEXT NOT NULL,
    relation     TEXT NOT NULL,
    subject_type TEXT NOT NULL,
    subject_id   TEXT NOT NULL,
    subject_rel  TEXT,
    created_at   DOUBLE PRECISION NOT NULL DEFAULT EXTRACT(EPOCH FROM NOW()),
    PRIMARY KEY (object_type, object_id, relation, subject_type, subject_id, subject_rel)
);
CREATE INDEX IF NOT EXISTS idx_auth_zanzibar_tuples_rev
    ON auth.zanzibar_tuples (subject_type, subject_id, relation);
CREATE UNIQUE INDEX IF NOT EXISTS uq_auth_zanzibar_tuples_direct
    ON auth.zanzibar_tuples (object_type, object_id, relation, subject_type, subject_id)
    WHERE subject_rel IS NULL;
"#;

/// Postgres DDL for the auth schema, version 4 — full OIDC provider.
///
/// Seven tables together implement a conformant Authorization-Code +
/// PKCE OIDC provider with refresh tokens, RP-initiated logout via SSO
/// session registry, per-(user, client) consent records, and upstream
/// federation state for the assay-as-RP path:
///
/// - `auth.oidc_clients` — registered consumer apps (client_id +
///   secret hash + redirect URIs + auth method + default scopes +
///   consent toggle).
/// - `auth.upstream_providers` — federated identity providers
///   (Google / Apple / GitHub / any OIDC IdP); used by the
///   `auth.oidc.OidcRegistry` to seed itself on boot.
/// - `auth.oidc_authorization_codes` — single-use codes issued at the
///   end of `/authorize`; consumed at `/token` exchange.
/// - `auth.oidc_refresh_tokens` — long-lived bearer tokens stored as
///   SHA-256 hashes (the bearer never round-trips the DB in plaintext).
/// - `auth.oidc_sessions` — SSO session registry; one row per issued
///   id_token. Carries the `sid` claim so `/logout` can fan out
///   back-channel logout to every consumer.
/// - `auth.oidc_consents` — per-(user, client) consent grants so the
///   consent screen only shows on first authorize for a given pair.
/// - `auth.oidc_upstream_states` — short-lived per-login rows for the
///   federation flow (state + nonce + pkce_verifier + return_to).
pub const PG_DDL_V4: &str = r#"
CREATE TABLE IF NOT EXISTS auth.oidc_clients (
    client_id                       TEXT PRIMARY KEY,
    client_secret_hash              TEXT,
    redirect_uris                   TEXT NOT NULL,
    name                            TEXT NOT NULL,
    logo_url                        TEXT,
    token_endpoint_auth_method      TEXT NOT NULL,
    default_scopes                  TEXT NOT NULL,
    require_consent                 BOOLEAN NOT NULL DEFAULT TRUE,
    grant_types                     TEXT NOT NULL DEFAULT '["authorization_code","refresh_token"]',
    response_types                  TEXT NOT NULL DEFAULT '["code"]',
    pkce_required                   BOOLEAN NOT NULL DEFAULT TRUE,
    backchannel_logout_uri          TEXT,
    created_at                      DOUBLE PRECISION NOT NULL
);

CREATE TABLE IF NOT EXISTS auth.upstream_providers (
    slug            TEXT PRIMARY KEY,
    issuer          TEXT NOT NULL,
    client_id       TEXT NOT NULL,
    client_secret   TEXT NOT NULL,
    display_name    TEXT NOT NULL,
    icon_url        TEXT,
    enabled         BOOLEAN NOT NULL DEFAULT TRUE
);

CREATE TABLE IF NOT EXISTS auth.oidc_authorization_codes (
    code                    TEXT PRIMARY KEY,
    client_id               TEXT NOT NULL,
    user_id                 TEXT NOT NULL,
    redirect_uri            TEXT NOT NULL,
    scopes                  TEXT NOT NULL,
    code_challenge          TEXT NOT NULL,
    code_challenge_method   TEXT NOT NULL,
    nonce                   TEXT,
    state                   TEXT,
    issued_at               DOUBLE PRECISION NOT NULL,
    expires_at              DOUBLE PRECISION NOT NULL,
    consumed                BOOLEAN NOT NULL DEFAULT FALSE
);

CREATE TABLE IF NOT EXISTS auth.oidc_refresh_tokens (
    token_hash      TEXT PRIMARY KEY,
    client_id       TEXT NOT NULL,
    user_id         TEXT NOT NULL,
    scopes          TEXT NOT NULL,
    issued_at       DOUBLE PRECISION NOT NULL,
    expires_at      DOUBLE PRECISION NOT NULL,
    revoked         BOOLEAN NOT NULL DEFAULT FALSE
);
CREATE INDEX IF NOT EXISTS idx_auth_oidc_refresh_user
    ON auth.oidc_refresh_tokens (user_id);

CREATE TABLE IF NOT EXISTS auth.oidc_sessions (
    sid                     TEXT PRIMARY KEY,
    user_id                 TEXT NOT NULL,
    client_id               TEXT NOT NULL,
    assay_session_id        TEXT,
    issued_at               DOUBLE PRECISION NOT NULL,
    backchannel_logout_uri  TEXT
);
CREATE INDEX IF NOT EXISTS idx_auth_oidc_sessions_user
    ON auth.oidc_sessions (user_id);
CREATE INDEX IF NOT EXISTS idx_auth_oidc_sessions_assay
    ON auth.oidc_sessions (assay_session_id);

CREATE TABLE IF NOT EXISTS auth.oidc_consents (
    user_id     TEXT NOT NULL,
    client_id   TEXT NOT NULL,
    scopes      TEXT NOT NULL,
    granted_at  DOUBLE PRECISION NOT NULL,
    PRIMARY KEY (user_id, client_id)
);

CREATE TABLE IF NOT EXISTS auth.oidc_upstream_states (
    state           TEXT PRIMARY KEY,
    provider_slug   TEXT NOT NULL,
    nonce           TEXT NOT NULL,
    pkce_verifier   TEXT NOT NULL,
    return_to       TEXT,
    created_at      DOUBLE PRECISION NOT NULL,
    expires_at      DOUBLE PRECISION NOT NULL
);
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

/// SQLite DDL for the auth schema, version 2 — biscuit root keys.
/// Mirrors [`PG_DDL_V2`] with `BYTEA` → `BLOB` and `DOUBLE PRECISION` →
/// `REAL`.
pub const SQLITE_DDL_V2: &[(&str, &str)] = &[
    (
        "biscuit_root_keys",
        "CREATE TABLE IF NOT EXISTS auth.biscuit_root_keys (
            kid             TEXT PRIMARY KEY,
            private_pem     BLOB NOT NULL,
            public_pem      TEXT NOT NULL,
            created_at      REAL NOT NULL,
            rotated_at      REAL
        )",
    ),
    (
        "idx_biscuit_root_keys_active",
        "CREATE INDEX IF NOT EXISTS auth.idx_auth_biscuit_root_keys_active \
         ON biscuit_root_keys (rotated_at) WHERE rotated_at IS NULL",
    ),
];

/// SQLite DDL for the auth schema, version 3 — Zanzibar / ReBAC.
///
/// Mirrors [`PG_DDL_V3`] with `JSONB` → `TEXT` (caller round-trips
/// via `serde_json`), `DOUBLE PRECISION` → `REAL`. SQLite's
/// `default CURRENT_TIMESTAMP` returns a string, not a unix epoch
/// double, so the SQLite store binds the timestamp explicitly on
/// every insert (matches the rest of the auth schema's discipline).
///
/// Treats `subject_rel` exactly as PG does: NULL for direct subjects,
/// some relation name for usersets. SQLite considers two NULL values
/// distinct in `PRIMARY KEY` comparisons just like PG, so the partial
/// unique index reproduces the same dedup semantics.
pub const SQLITE_DDL_V3: &[(&str, &str)] = &[
    (
        "zanzibar_namespaces",
        "CREATE TABLE IF NOT EXISTS auth.zanzibar_namespaces (
            name        TEXT PRIMARY KEY,
            schema_json TEXT NOT NULL,
            updated_at  REAL NOT NULL
        )",
    ),
    (
        "zanzibar_tuples",
        "CREATE TABLE IF NOT EXISTS auth.zanzibar_tuples (
            object_type  TEXT NOT NULL,
            object_id    TEXT NOT NULL,
            relation     TEXT NOT NULL,
            subject_type TEXT NOT NULL,
            subject_id   TEXT NOT NULL,
            subject_rel  TEXT,
            created_at   REAL NOT NULL,
            PRIMARY KEY (object_type, object_id, relation, subject_type, subject_id, subject_rel)
        )",
    ),
    (
        "idx_zanzibar_tuples_rev",
        "CREATE INDEX IF NOT EXISTS auth.idx_auth_zanzibar_tuples_rev \
         ON zanzibar_tuples (subject_type, subject_id, relation)",
    ),
    (
        "uq_zanzibar_tuples_direct",
        "CREATE UNIQUE INDEX IF NOT EXISTS auth.uq_auth_zanzibar_tuples_direct \
         ON zanzibar_tuples (object_type, object_id, relation, subject_type, subject_id) \
         WHERE subject_rel IS NULL",
    ),
];

/// SQLite DDL for the auth schema, version 4 — full OIDC provider.
///
/// Mirrors [`PG_DDL_V4`] with `BOOLEAN` → `INTEGER` and `DOUBLE PRECISION`
/// → `REAL`. JSON arrays (redirect_uris, default_scopes, scopes, …) ride
/// in `TEXT` columns and round-trip via `serde_json` in the store layer
/// — same convention `auth.zanzibar_namespaces` uses for `schema_json`.
pub const SQLITE_DDL_V4: &[(&str, &str)] = &[
    (
        "oidc_clients",
        "CREATE TABLE IF NOT EXISTS auth.oidc_clients (
            client_id                       TEXT PRIMARY KEY,
            client_secret_hash              TEXT,
            redirect_uris                   TEXT NOT NULL,
            name                            TEXT NOT NULL,
            logo_url                        TEXT,
            token_endpoint_auth_method      TEXT NOT NULL,
            default_scopes                  TEXT NOT NULL,
            require_consent                 INTEGER NOT NULL DEFAULT 1,
            grant_types                     TEXT NOT NULL DEFAULT '[\"authorization_code\",\"refresh_token\"]',
            response_types                  TEXT NOT NULL DEFAULT '[\"code\"]',
            pkce_required                   INTEGER NOT NULL DEFAULT 1,
            backchannel_logout_uri          TEXT,
            created_at                      REAL NOT NULL
        )",
    ),
    (
        "upstream_providers",
        "CREATE TABLE IF NOT EXISTS auth.upstream_providers (
            slug            TEXT PRIMARY KEY,
            issuer          TEXT NOT NULL,
            client_id       TEXT NOT NULL,
            client_secret   TEXT NOT NULL,
            display_name    TEXT NOT NULL,
            icon_url        TEXT,
            enabled         INTEGER NOT NULL DEFAULT 1
        )",
    ),
    (
        "oidc_authorization_codes",
        "CREATE TABLE IF NOT EXISTS auth.oidc_authorization_codes (
            code                    TEXT PRIMARY KEY,
            client_id               TEXT NOT NULL,
            user_id                 TEXT NOT NULL,
            redirect_uri            TEXT NOT NULL,
            scopes                  TEXT NOT NULL,
            code_challenge          TEXT NOT NULL,
            code_challenge_method   TEXT NOT NULL,
            nonce                   TEXT,
            state                   TEXT,
            issued_at               REAL NOT NULL,
            expires_at              REAL NOT NULL,
            consumed                INTEGER NOT NULL DEFAULT 0
        )",
    ),
    (
        "oidc_refresh_tokens",
        "CREATE TABLE IF NOT EXISTS auth.oidc_refresh_tokens (
            token_hash      TEXT PRIMARY KEY,
            client_id       TEXT NOT NULL,
            user_id         TEXT NOT NULL,
            scopes          TEXT NOT NULL,
            issued_at       REAL NOT NULL,
            expires_at      REAL NOT NULL,
            revoked         INTEGER NOT NULL DEFAULT 0
        )",
    ),
    (
        "idx_oidc_refresh_user",
        "CREATE INDEX IF NOT EXISTS auth.idx_auth_oidc_refresh_user \
         ON oidc_refresh_tokens (user_id)",
    ),
    (
        "oidc_sessions",
        "CREATE TABLE IF NOT EXISTS auth.oidc_sessions (
            sid                     TEXT PRIMARY KEY,
            user_id                 TEXT NOT NULL,
            client_id               TEXT NOT NULL,
            assay_session_id        TEXT,
            issued_at               REAL NOT NULL,
            backchannel_logout_uri  TEXT
        )",
    ),
    (
        "idx_oidc_sessions_user",
        "CREATE INDEX IF NOT EXISTS auth.idx_auth_oidc_sessions_user \
         ON oidc_sessions (user_id)",
    ),
    (
        "idx_oidc_sessions_assay",
        "CREATE INDEX IF NOT EXISTS auth.idx_auth_oidc_sessions_assay \
         ON oidc_sessions (assay_session_id)",
    ),
    (
        "oidc_consents",
        "CREATE TABLE IF NOT EXISTS auth.oidc_consents (
            user_id     TEXT NOT NULL,
            client_id   TEXT NOT NULL,
            scopes      TEXT NOT NULL,
            granted_at  REAL NOT NULL,
            PRIMARY KEY (user_id, client_id)
        )",
    ),
    (
        "oidc_upstream_states",
        "CREATE TABLE IF NOT EXISTS auth.oidc_upstream_states (
            state           TEXT PRIMARY KEY,
            provider_slug   TEXT NOT NULL,
            nonce           TEXT NOT NULL,
            pkce_verifier   TEXT NOT NULL,
            return_to       TEXT,
            created_at      REAL NOT NULL,
            expires_at      REAL NOT NULL
        )",
    ),
];

/// Postgres migration runner.
///
/// Applies every DDL pack up to and including the current
/// [`MIGRATION_VERSION`] (V1 then V2 today). Splits each pack into
/// individual statements (sqlx requires one statement per `query`),
/// executes each, then records `(MODULE_NAME, MIGRATION_VERSION)` into
/// `engine.migrations`. Idempotent — every CREATE uses `IF NOT EXISTS`.
#[cfg(feature = "backend-postgres")]
pub async fn migrate_postgres(pool: &sqlx::PgPool) -> anyhow::Result<()> {
    use anyhow::Context;
    for ddl in [PG_DDL_V1, PG_DDL_V2, PG_DDL_V3, PG_DDL_V4] {
        for stmt in split_pg_statements(ddl) {
            sqlx::query(&stmt)
                .execute(pool)
                .await
                .with_context(|| format!("auth pg migrate: {}", first_line(&stmt)))?;
        }
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
/// calling. Applies every DDL pack up to and including
/// [`MIGRATION_VERSION`] (V1 then V2 today). Each DDL chunk is
/// executed as its own statement; the per-table failure context names
/// the table that broke so engine boot logs are actionable.
#[cfg(feature = "backend-sqlite")]
pub async fn migrate_sqlite(pool: &sqlx::SqlitePool) -> anyhow::Result<()> {
    use anyhow::Context;
    for pack in [SQLITE_DDL_V1, SQLITE_DDL_V2, SQLITE_DDL_V3, SQLITE_DDL_V4] {
        for (label, stmt) in pack {
            sqlx::query(stmt)
                .execute(pool)
                .await
                .with_context(|| format!("auth sqlite migrate: {label}"))?;
        }
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
