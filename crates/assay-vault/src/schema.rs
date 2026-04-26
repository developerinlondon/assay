//! Vault module schema bootstrap + migration runner.
//!
//! Provides `migrate_*` entrypoints the engine boot path calls when
//! `engine.modules` shows `vault` enabled. Each entrypoint:
//!
//! 1. Ensures the storage container exists. PG: `CREATE SCHEMA IF NOT
//!    EXISTS vault`. SQLite: relies on the engine boot having ATTACHed
//!    `data/vault.db` AS `vault` — the migration runs DDL into the
//!    attachment.
//! 2. Applies every DDL statement up to the current
//!    [`MIGRATION_VERSION`].
//! 3. Records the applied version into `engine.migrations` with
//!    `module = MODULE_NAME` so subsequent boots skip already-applied
//!    versions.
//!
//! The migration is idempotent — every CREATE uses `IF NOT EXISTS`,
//! every INSERT into `engine.migrations` uses `ON CONFLICT DO NOTHING`.
//! Re-running on a healthy DB is a no-op.
//!
//! Tables created (per plan 17 § "Schema — tables in `vault.*`"):
//!
//! - `vault.kv_meta`           — KV path metadata (latest version, custom md, deletion state)
//! - `vault.kv`                — versioned KV blobs (ciphertext, wrapped DEK, kek_kid)
//! - `vault.transit_keys`      — transit master keys (name + algo + latest version)
//! - `vault.transit_versions`  — per-key version material (rotated)
//! - `vault.leases`            — dynamic-creds lease tracking
//! - `vault.vaults`            — per-user personal vault registry
//! - `vault.collections`       — shared collections (org-scoped)
//! - `vault.collection_members`— (collection, user) → wrapped collection key + role
//! - `vault.items`             — encrypted items (vault_id XOR collection_id, folder_id optional)
//! - `vault.folders`           — Bitwarden-compat folders (vault- or collection-scoped)
//! - `vault.share_revoked`     — biscuit `key_id`s revoked
//! - `vault.kek_metadata`      — sealing method, kid, unseal state
//! - `vault.unseal_shares`     — Shamir SSS shares for KEK init unseal
//! - `vault.audit_sinks`       — forwarding configs (syslog/S3/webhook)
//!
//! Schema is intentionally still loose in Phase 0: enough to apply
//! cleanly and let the smoke test exercise an insert/read round-trip.
//! The per-feature phases (1-7) tighten constraints and add indexes as
//! the storage paths solidify.

/// Stable name registered in `engine.modules.name` and used as the
/// `module` discriminant in `engine.migrations`. Matches the schema
/// (PG) / attached-database (SQLite) name 1:1 so SQL stays readable.
pub const MODULE_NAME: &str = "vault";

/// Highest migration version this build knows about. Bumped each time
/// a new DDL pack is appended below.
///
/// V1: full plan-17 table set — see module-level docs for the list.
///     Phase 0 ships with V1 only; subsequent phases bump as new
///     storage shapes land.
pub const MIGRATION_VERSION: i32 = 1;

/// Postgres DDL for the vault schema, version 1.
///
/// All tables are schema-qualified (`vault.*`) so they live in the
/// `vault` schema regardless of the connection's `search_path`.
/// `CREATE SCHEMA IF NOT EXISTS` is included for completeness — engine
/// boot also runs it, but tests that bootstrap the vault schema
/// directly need both paths to work.
pub const PG_DDL_V1: &str = r#"
CREATE SCHEMA IF NOT EXISTS vault;

-- ── S1: KV v2 ─────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS vault.kv_meta (
    path           TEXT PRIMARY KEY,
    latest_version BIGINT NOT NULL DEFAULT 0,
    custom_md      JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at     DOUBLE PRECISION NOT NULL DEFAULT EXTRACT(EPOCH FROM NOW()),
    updated_at     DOUBLE PRECISION NOT NULL DEFAULT EXTRACT(EPOCH FROM NOW())
);

-- Per-record DEK envelope. `wrapped_dek` is the data-encryption key
-- (32 random bytes) encrypted by the master KEK identified by `kek_kid`;
-- `ciphertext` is the payload encrypted by that DEK with AES-256-GCM-SIV
-- and `nonce`. The path-and-version pair binds the AEAD's AAD so a
-- ciphertext relocated to a different row fails to authenticate.
CREATE TABLE IF NOT EXISTS vault.kv (
    path        TEXT NOT NULL,
    version     BIGINT NOT NULL,
    ciphertext  BYTEA NOT NULL,
    nonce       BYTEA NOT NULL,
    wrapped_dek BYTEA NOT NULL,
    kek_kid     TEXT NOT NULL,
    deleted_at  DOUBLE PRECISION,
    destroyed   BOOLEAN NOT NULL DEFAULT FALSE,
    created_at  DOUBLE PRECISION NOT NULL DEFAULT EXTRACT(EPOCH FROM NOW()),
    PRIMARY KEY (path, version)
);
CREATE INDEX IF NOT EXISTS idx_vault_kv_path
    ON vault.kv (path);

-- ── S2: transit ───────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS vault.transit_keys (
    name         TEXT PRIMARY KEY,
    latest_ver   BIGINT NOT NULL DEFAULT 1,
    algo         TEXT NOT NULL DEFAULT 'aes256-gcm-siv',
    created_at   DOUBLE PRECISION NOT NULL DEFAULT EXTRACT(EPOCH FROM NOW())
);

CREATE TABLE IF NOT EXISTS vault.transit_versions (
    name        TEXT NOT NULL,
    version     BIGINT NOT NULL,
    key_wrapped BYTEA NOT NULL,
    kek_kid     TEXT NOT NULL,
    created_at  DOUBLE PRECISION NOT NULL DEFAULT EXTRACT(EPOCH FROM NOW()),
    PRIMARY KEY (name, version),
    FOREIGN KEY (name) REFERENCES vault.transit_keys(name) ON DELETE CASCADE
);

-- ── S3: dynamic-creds leases ─────────────────────────────────────
CREATE TABLE IF NOT EXISTS vault.leases (
    id           TEXT PRIMARY KEY,
    provider     TEXT NOT NULL,
    role         TEXT NOT NULL,
    issued_at    DOUBLE PRECISION NOT NULL DEFAULT EXTRACT(EPOCH FROM NOW()),
    expires_at   DOUBLE PRECISION NOT NULL,
    revoked_at   DOUBLE PRECISION,
    metadata     JSONB NOT NULL DEFAULT '{}'::jsonb
);
CREATE INDEX IF NOT EXISTS idx_vault_leases_expires
    ON vault.leases (expires_at)
    WHERE revoked_at IS NULL;

-- ── S4: per-user personal vaults ─────────────────────────────────
-- Exactly one personal vault per user (UNIQUE owner_user). Auto-created
-- on signup by the assay-auth user-create path in Phase 3. The X25519
-- public key is stored here so collection-key envelopes can be wrapped
-- to it offline.
CREATE TABLE IF NOT EXISTS vault.vaults (
    id           TEXT PRIMARY KEY,
    owner_user   TEXT NOT NULL UNIQUE,
    public_key   BYTEA NOT NULL,
    created_at   DOUBLE PRECISION NOT NULL DEFAULT EXTRACT(EPOCH FROM NOW())
);

-- ── S4: shared collections ───────────────────────────────────────
-- Bitwarden-equivalent of organizations' "Collections". Org-scoped via
-- the optional `org_id` (NULL for personal-team collections that don't
-- belong to a parent org). Membership lives in `collection_members`;
-- access checks ride on top via Zanzibar tuples.
CREATE TABLE IF NOT EXISTS vault.collections (
    id           TEXT PRIMARY KEY,
    org_id       TEXT,
    name         TEXT NOT NULL,
    created_by   TEXT NOT NULL,
    created_at   DOUBLE PRECISION NOT NULL DEFAULT EXTRACT(EPOCH FROM NOW())
);
CREATE INDEX IF NOT EXISTS idx_vault_collections_org
    ON vault.collections (org_id) WHERE org_id IS NOT NULL;

-- Per-member envelope: the collection's symmetric key wrapped to this
-- member's X25519 public key (lifted from `vault.vaults.public_key`).
-- Decryption is client-side; the server never sees plaintext.
CREATE TABLE IF NOT EXISTS vault.collection_members (
    collection_id TEXT NOT NULL REFERENCES vault.collections(id) ON DELETE CASCADE,
    user_id       TEXT NOT NULL,
    wrapped_key   BYTEA NOT NULL,
    role          TEXT NOT NULL DEFAULT 'viewer',
    added_at      DOUBLE PRECISION NOT NULL DEFAULT EXTRACT(EPOCH FROM NOW()),
    PRIMARY KEY (collection_id, user_id)
);

-- ── S4: items + folders ──────────────────────────────────────────
-- Item lives in EXACTLY ONE of (a personal vault, a collection). The
-- CHECK enforces the XOR; folder_id is optional for visual organization
-- (see `vault.folders` — Bitwarden-compat, not an access boundary).
CREATE TABLE IF NOT EXISTS vault.items (
    id            TEXT PRIMARY KEY,
    vault_id      TEXT REFERENCES vault.vaults(id) ON DELETE CASCADE,
    collection_id TEXT REFERENCES vault.collections(id) ON DELETE CASCADE,
    folder_id     TEXT,
    item_type     TEXT NOT NULL,
    name          TEXT NOT NULL,
    ciphertext    BYTEA NOT NULL,
    nonce         BYTEA NOT NULL,
    created_at    DOUBLE PRECISION NOT NULL DEFAULT EXTRACT(EPOCH FROM NOW()),
    updated_at    DOUBLE PRECISION NOT NULL DEFAULT EXTRACT(EPOCH FROM NOW()),
    CHECK ((vault_id IS NULL) <> (collection_id IS NULL))
);
CREATE INDEX IF NOT EXISTS idx_vault_items_vault
    ON vault.items (vault_id) WHERE vault_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_vault_items_collection
    ON vault.items (collection_id) WHERE collection_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_vault_items_folder
    ON vault.items (folder_id) WHERE folder_id IS NOT NULL;

-- Folders are pure visual organisation (Bitwarden-compat). Same XOR
-- constraint: a folder lives inside exactly one container. `parent_id`
-- supports nested folders — Phase 3 may flatten this if BW clients only
-- emit a single level; carrying the column is cheap and forward-safe.
CREATE TABLE IF NOT EXISTS vault.folders (
    id            TEXT PRIMARY KEY,
    vault_id      TEXT REFERENCES vault.vaults(id) ON DELETE CASCADE,
    collection_id TEXT REFERENCES vault.collections(id) ON DELETE CASCADE,
    parent_id     TEXT,
    name          TEXT NOT NULL,
    created_at    DOUBLE PRECISION NOT NULL DEFAULT EXTRACT(EPOCH FROM NOW()),
    CHECK ((vault_id IS NULL) <> (collection_id IS NULL))
);

-- ── S5: biscuit share revocations ─────────────────────────────────
CREATE TABLE IF NOT EXISTS vault.share_revoked (
    key_id       TEXT PRIMARY KEY,
    revoked_at   DOUBLE PRECISION NOT NULL DEFAULT EXTRACT(EPOCH FROM NOW()),
    reason       TEXT NOT NULL DEFAULT ''
);

-- ── S7: sealing — KEK metadata + Shamir SSS shares ───────────────
-- The active KEK is the row with the most recent `created_at`. KEK
-- rotation appends a new row; the previous KEK stays so existing
-- wrapped DEKs remain decryptable until the operator-driven re-wrap
-- finishes.
-- `sealed_blob` holds the master KEK material at rest. The interpretation
-- depends on `sealing_method`:
--   plaintext       — Phase 1 placeholder; blob IS the raw 32-byte KEK.
--                     Tracked in kek_metadata so Phase 2 can re-wrap.
--   shamir          — blob is empty; the KEK is split into rows in
--                     vault.unseal_shares and reconstituted on unseal.
--   kms-aws / kms-gcp — blob is the cloud-KMS-encrypted KEK; auto-unseal
--                     calls the cloud KMS Decrypt API on boot.
--   hsm             — blob is the PKCS#11-wrapped KEK (opt-in feature).
CREATE TABLE IF NOT EXISTS vault.kek_metadata (
    kid              TEXT PRIMARY KEY,
    sealing_method   TEXT NOT NULL,
    sealed           BOOLEAN NOT NULL DEFAULT TRUE,
    sealed_blob      BYTEA NOT NULL DEFAULT ''::bytea,
    share_threshold  INTEGER,
    share_count      INTEGER,
    sealed_at        DOUBLE PRECISION,
    unsealed_at      DOUBLE PRECISION,
    created_at       DOUBLE PRECISION NOT NULL DEFAULT EXTRACT(EPOCH FROM NOW())
);

-- Shamir-Secret-Sharing shares for the init-unseal flow. Encrypted at
-- rest with the share-holder's identity key (X25519) so a stolen DB
-- snapshot doesn't yield the KEK.
CREATE TABLE IF NOT EXISTS vault.unseal_shares (
    kid              TEXT NOT NULL REFERENCES vault.kek_metadata(kid) ON DELETE CASCADE,
    share_index      INTEGER NOT NULL,
    share_holder     TEXT NOT NULL,
    encrypted_share  BYTEA NOT NULL,
    created_at       DOUBLE PRECISION NOT NULL DEFAULT EXTRACT(EPOCH FROM NOW()),
    PRIMARY KEY (kid, share_index)
);

-- ── S8: audit forwarding sinks ────────────────────────────────────
-- One row per configured forwarder. `kind` ∈ {syslog, s3, webhook};
-- `config` carries kind-specific JSON (host/port for syslog, bucket /
-- prefix / region for s3, url / headers for webhook). `filter_pattern`
-- is a glob over event names so an operator can scope a sink to e.g.
-- `vault.*` or `auth.login.*`.
CREATE TABLE IF NOT EXISTS vault.audit_sinks (
    id              TEXT PRIMARY KEY,
    name            TEXT NOT NULL UNIQUE,
    kind            TEXT NOT NULL,
    config          JSONB NOT NULL DEFAULT '{}'::jsonb,
    filter_pattern  TEXT NOT NULL DEFAULT '*',
    enabled         BOOLEAN NOT NULL DEFAULT TRUE,
    created_at      DOUBLE PRECISION NOT NULL DEFAULT EXTRACT(EPOCH FROM NOW())
);
"#;

/// SQLite DDL for the vault schema, version 1.
///
/// Caller must have ATTACHed `data/vault.db` AS `vault` before running
/// this — engine boot is responsible for that wiring (matches the
/// pattern already used for the engine + workflow + auth attachments).
///
/// Mirrors the PG layout:
///   `BYTEA`           → `BLOB`
///   `BOOLEAN`         → `INTEGER` (0/1)
///   `JSONB`           → `TEXT` (caller round-trips via `serde_json`)
///   `DOUBLE PRECISION`→ `REAL` (caller binds the timestamp explicitly)
///
/// SQLite's `default CURRENT_TIMESTAMP` returns a text string, not a
/// unix-epoch double, so the SQLite store binds timestamps explicitly
/// on every insert (matches the discipline assay-auth uses).
pub const SQLITE_DDL_V1: &[(&str, &str)] = &[
    // ── S1: KV v2 ────────────────────────────────────────────────
    (
        "kv_meta",
        "CREATE TABLE IF NOT EXISTS vault.kv_meta (
            path           TEXT PRIMARY KEY,
            latest_version INTEGER NOT NULL DEFAULT 0,
            custom_md      TEXT NOT NULL DEFAULT '{}',
            created_at     REAL NOT NULL,
            updated_at     REAL NOT NULL
        )",
    ),
    (
        "kv",
        "CREATE TABLE IF NOT EXISTS vault.kv (
            path        TEXT NOT NULL,
            version     INTEGER NOT NULL,
            ciphertext  BLOB NOT NULL,
            nonce       BLOB NOT NULL,
            wrapped_dek BLOB NOT NULL,
            kek_kid     TEXT NOT NULL,
            deleted_at  REAL,
            destroyed   INTEGER NOT NULL DEFAULT 0,
            created_at  REAL NOT NULL,
            PRIMARY KEY (path, version)
        )",
    ),
    (
        "idx_kv_path",
        "CREATE INDEX IF NOT EXISTS vault.idx_vault_kv_path ON kv (path)",
    ),
    // ── S2: transit ──────────────────────────────────────────────
    (
        "transit_keys",
        "CREATE TABLE IF NOT EXISTS vault.transit_keys (
            name         TEXT PRIMARY KEY,
            latest_ver   INTEGER NOT NULL DEFAULT 1,
            algo         TEXT NOT NULL DEFAULT 'aes256-gcm-siv',
            created_at   REAL NOT NULL
        )",
    ),
    (
        "transit_versions",
        "CREATE TABLE IF NOT EXISTS vault.transit_versions (
            name        TEXT NOT NULL,
            version     INTEGER NOT NULL,
            key_wrapped BLOB NOT NULL,
            kek_kid     TEXT NOT NULL,
            created_at  REAL NOT NULL,
            PRIMARY KEY (name, version),
            FOREIGN KEY (name) REFERENCES transit_keys(name) ON DELETE CASCADE
        )",
    ),
    // ── S3: dynamic-creds leases ─────────────────────────────────
    (
        "leases",
        "CREATE TABLE IF NOT EXISTS vault.leases (
            id           TEXT PRIMARY KEY,
            provider     TEXT NOT NULL,
            role         TEXT NOT NULL,
            issued_at    REAL NOT NULL,
            expires_at   REAL NOT NULL,
            revoked_at   REAL,
            metadata     TEXT NOT NULL DEFAULT '{}'
        )",
    ),
    (
        "idx_leases_expires",
        "CREATE INDEX IF NOT EXISTS vault.idx_vault_leases_expires \
         ON leases (expires_at) WHERE revoked_at IS NULL",
    ),
    // ── S4: vaults / collections / members / items / folders ─────
    (
        "vaults",
        "CREATE TABLE IF NOT EXISTS vault.vaults (
            id           TEXT PRIMARY KEY,
            owner_user   TEXT NOT NULL UNIQUE,
            public_key   BLOB NOT NULL,
            created_at   REAL NOT NULL
        )",
    ),
    (
        "collections",
        "CREATE TABLE IF NOT EXISTS vault.collections (
            id           TEXT PRIMARY KEY,
            org_id       TEXT,
            name         TEXT NOT NULL,
            created_by   TEXT NOT NULL,
            created_at   REAL NOT NULL
        )",
    ),
    (
        "idx_collections_org",
        "CREATE INDEX IF NOT EXISTS vault.idx_vault_collections_org \
         ON collections (org_id) WHERE org_id IS NOT NULL",
    ),
    (
        "collection_members",
        "CREATE TABLE IF NOT EXISTS vault.collection_members (
            collection_id TEXT NOT NULL REFERENCES collections(id) ON DELETE CASCADE,
            user_id       TEXT NOT NULL,
            wrapped_key   BLOB NOT NULL,
            role          TEXT NOT NULL DEFAULT 'viewer',
            added_at      REAL NOT NULL,
            PRIMARY KEY (collection_id, user_id)
        )",
    ),
    (
        "items",
        "CREATE TABLE IF NOT EXISTS vault.items (
            id            TEXT PRIMARY KEY,
            vault_id      TEXT REFERENCES vaults(id) ON DELETE CASCADE,
            collection_id TEXT REFERENCES collections(id) ON DELETE CASCADE,
            folder_id     TEXT,
            item_type     TEXT NOT NULL,
            name          TEXT NOT NULL,
            ciphertext    BLOB NOT NULL,
            nonce         BLOB NOT NULL,
            created_at    REAL NOT NULL,
            updated_at    REAL NOT NULL,
            CHECK ((vault_id IS NULL) <> (collection_id IS NULL))
        )",
    ),
    (
        "idx_items_vault",
        "CREATE INDEX IF NOT EXISTS vault.idx_vault_items_vault \
         ON items (vault_id) WHERE vault_id IS NOT NULL",
    ),
    (
        "idx_items_collection",
        "CREATE INDEX IF NOT EXISTS vault.idx_vault_items_collection \
         ON items (collection_id) WHERE collection_id IS NOT NULL",
    ),
    (
        "idx_items_folder",
        "CREATE INDEX IF NOT EXISTS vault.idx_vault_items_folder \
         ON items (folder_id) WHERE folder_id IS NOT NULL",
    ),
    (
        "folders",
        "CREATE TABLE IF NOT EXISTS vault.folders (
            id            TEXT PRIMARY KEY,
            vault_id      TEXT REFERENCES vaults(id) ON DELETE CASCADE,
            collection_id TEXT REFERENCES collections(id) ON DELETE CASCADE,
            parent_id     TEXT,
            name          TEXT NOT NULL,
            created_at    REAL NOT NULL,
            CHECK ((vault_id IS NULL) <> (collection_id IS NULL))
        )",
    ),
    // ── S5: biscuit share revocations ────────────────────────────
    (
        "share_revoked",
        "CREATE TABLE IF NOT EXISTS vault.share_revoked (
            key_id       TEXT PRIMARY KEY,
            revoked_at   REAL NOT NULL,
            reason       TEXT NOT NULL DEFAULT ''
        )",
    ),
    // ── S7: sealing ──────────────────────────────────────────────
    (
        "kek_metadata",
        "CREATE TABLE IF NOT EXISTS vault.kek_metadata (
            kid              TEXT PRIMARY KEY,
            sealing_method   TEXT NOT NULL,
            sealed           INTEGER NOT NULL DEFAULT 1,
            sealed_blob      BLOB NOT NULL DEFAULT x'',
            share_threshold  INTEGER,
            share_count      INTEGER,
            sealed_at        REAL,
            unsealed_at      REAL,
            created_at       REAL NOT NULL
        )",
    ),
    (
        "unseal_shares",
        "CREATE TABLE IF NOT EXISTS vault.unseal_shares (
            kid              TEXT NOT NULL REFERENCES kek_metadata(kid) ON DELETE CASCADE,
            share_index      INTEGER NOT NULL,
            share_holder     TEXT NOT NULL,
            encrypted_share  BLOB NOT NULL,
            created_at       REAL NOT NULL,
            PRIMARY KEY (kid, share_index)
        )",
    ),
    // ── S8: audit forwarding sinks ───────────────────────────────
    (
        "audit_sinks",
        "CREATE TABLE IF NOT EXISTS vault.audit_sinks (
            id              TEXT PRIMARY KEY,
            name            TEXT NOT NULL UNIQUE,
            kind            TEXT NOT NULL,
            config          TEXT NOT NULL DEFAULT '{}',
            filter_pattern  TEXT NOT NULL DEFAULT '*',
            enabled         INTEGER NOT NULL DEFAULT 1,
            created_at      REAL NOT NULL
        )",
    ),
];

/// Postgres migration runner.
///
/// Applies every DDL pack up to and including [`MIGRATION_VERSION`],
/// then records `(MODULE_NAME, MIGRATION_VERSION)` into
/// `engine.migrations`. Each pack is split into individual statements
/// (sqlx requires one statement per `query`); per-statement failure
/// context names the first line so engine boot logs are actionable.
#[cfg(feature = "backend-postgres")]
pub async fn migrate_postgres(pool: &sqlx::PgPool) -> anyhow::Result<()> {
    use anyhow::Context;
    for ddl in [PG_DDL_V1] {
        for stmt in split_pg_statements(ddl) {
            sqlx::query(&stmt)
                .execute(pool)
                .await
                .with_context(|| format!("vault pg migrate: {}", first_line(&stmt)))?;
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
    .context("record vault migration in engine.migrations")?;
    Ok(())
}

/// SQLite migration runner.
///
/// Caller must have ATTACHed the vault database as `vault` before
/// calling. Each DDL chunk is executed as its own statement; the
/// per-table failure context names the table that broke.
#[cfg(feature = "backend-sqlite")]
pub async fn migrate_sqlite(pool: &sqlx::SqlitePool) -> anyhow::Result<()> {
    use anyhow::Context;
    for pack in [SQLITE_DDL_V1] {
        for (label, stmt) in pack {
            sqlx::query(stmt)
                .execute(pool)
                .await
                .with_context(|| format!("vault sqlite migrate: {label}"))?;
        }
    }
    sqlx::query("INSERT OR IGNORE INTO engine.migrations (module, version) VALUES (?, ?)")
        .bind(MODULE_NAME)
        .bind(MIGRATION_VERSION)
        .execute(pool)
        .await
        .context("record vault migration in engine.migrations")?;
    Ok(())
}

/// Split a PG DDL chunk into individual statements. Drops pure-comment
/// lines first so a `--`-introduced semicolon doesn't fragment a real
/// statement (mirrors the same trick `assay-auth::schema` uses).
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
        assert_eq!(MODULE_NAME, "vault");
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
    fn pg_ddl_v1_split_covers_every_locked_table() {
        let stmts = split_pg_statements(PG_DDL_V1);
        // Plan 17 §"Schema — tables in `vault.*`": 14 tables.
        for table in [
            "vault.kv_meta",
            "vault.kv",
            "vault.transit_keys",
            "vault.transit_versions",
            "vault.leases",
            "vault.vaults",
            "vault.collections",
            "vault.collection_members",
            "vault.items",
            "vault.folders",
            "vault.share_revoked",
            "vault.kek_metadata",
            "vault.unseal_shares",
            "vault.audit_sinks",
        ] {
            assert!(
                stmts.iter().any(|s| s.contains(table)),
                "no DDL statement for {table}; got {} statements",
                stmts.len()
            );
        }
    }

    #[test]
    fn sqlite_ddl_v1_covers_every_locked_table() {
        // Same 14-table coverage check on the SQLite side. Labels are
        // unqualified (no `vault.` prefix) since they index into the
        // attached schema by name.
        for label in [
            "kv_meta",
            "kv",
            "transit_keys",
            "transit_versions",
            "leases",
            "vaults",
            "collections",
            "collection_members",
            "items",
            "folders",
            "share_revoked",
            "kek_metadata",
            "unseal_shares",
            "audit_sinks",
        ] {
            assert!(
                SQLITE_DDL_V1.iter().any(|(l, _)| *l == label),
                "no SQLite DDL pack entry for {label}"
            );
        }
    }
}
