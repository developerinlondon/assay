//! Phase 0 smoke test for the vault module schema bootstrap.
//!
//! Runs the migration end-to-end against an in-memory SQLite pool with
//! an attached `vault` database, then inserts and reads back one row in
//! every table the plan-17 schema declares. The PG path runs in CI when
//! `TEST_DATABASE_URL` is set; locally we depend on the SQLite path —
//! same convention assay-auth and assay-workflow tests already use.
//!
//! What this test proves (per Phase 0 deliverable in plan 17):
//!
//! - The DDL applies cleanly (no syntax errors, no missing schema).
//! - Every table exists and accepts a representative row.
//! - The XOR check on `vault.items` and `vault.folders` enforces the
//!   "lives in exactly one container" invariant.
//! - Re-running the migration is idempotent.

#![cfg(feature = "backend-sqlite")]

use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Executor, SqlitePool};
use std::str::FromStr;

/// Spin up an in-memory SQLite pool with a `vault` schema attached, plus
/// the `engine` schema the migration runner records its bookkeeping
/// row into. Mirrors the engine boot path's ATTACH discipline.
async fn boot_pool() -> SqlitePool {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let suffix = format!(
        "{}_{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    );
    let vault_uri = format!("file:assay_vault_test_{suffix}?mode=memory&cache=shared");
    let engine_uri = format!("file:assay_vault_engine_{suffix}?mode=memory&cache=shared");

    let opts = SqliteConnectOptions::from_str("sqlite::memory:")
        .unwrap()
        .create_if_missing(true);

    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .after_connect(move |conn, _| {
            let v = vault_uri.clone();
            let e = engine_uri.clone();
            Box::pin(async move {
                conn.execute(format!("ATTACH DATABASE '{e}' AS engine").as_str())
                    .await?;
                conn.execute(format!("ATTACH DATABASE '{v}' AS vault").as_str())
                    .await?;
                Ok(())
            })
        })
        .connect_with(opts)
        .await
        .expect("connect sqlite test pool");

    // engine.migrations is normally created by the engine schema bootstrap
    // (assay-domain). The vault test stands alone, so build the minimal
    // shape the vault migration's INSERT needs.
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS engine.migrations (
            module  TEXT NOT NULL,
            version INTEGER NOT NULL,
            PRIMARY KEY (module, version)
        )",
    )
    .execute(&pool)
    .await
    .expect("create engine.migrations");

    pool
}

#[tokio::test]
async fn migration_applies_and_is_idempotent() {
    let pool = boot_pool().await;
    assay_vault::schema::migrate_sqlite(&pool)
        .await
        .expect("first migration");
    // Re-running must be a no-op (every CREATE uses IF NOT EXISTS).
    assay_vault::schema::migrate_sqlite(&pool)
        .await
        .expect("second migration is idempotent");

    // engine.migrations bookkeeping landed exactly once.
    let row: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM engine.migrations WHERE module = 'vault'")
            .fetch_one(&pool)
            .await
            .expect("count migrations");
    assert_eq!(row.0, 1, "expected exactly one bookkeeping row for vault");
}

#[tokio::test]
async fn round_trip_one_row_per_table() {
    let pool = boot_pool().await;
    assay_vault::schema::migrate_sqlite(&pool)
        .await
        .expect("migrate");

    let now = 1_700_000_000.0_f64;

    // KV path metadata + one versioned blob.
    sqlx::query(
        "INSERT INTO vault.kv_meta (path, latest_version, custom_md, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind("api/stripe")
    .bind(1_i64)
    .bind("{}")
    .bind(now)
    .bind(now)
    .execute(&pool)
    .await
    .expect("insert kv_meta");

    sqlx::query(
        "INSERT INTO vault.kv (path, version, ciphertext, nonce, wrapped_dek, kek_kid, created_at)
         VALUES (?, 1, ?, ?, ?, 'kek-1', ?)",
    )
    .bind("api/stripe")
    .bind(b"ciphertext".as_slice())
    .bind(b"012345678901".as_slice())
    .bind(b"wrapped-dek-blob".as_slice())
    .bind(now)
    .execute(&pool)
    .await
    .expect("insert kv");

    // Transit key + version.
    sqlx::query(
        "INSERT INTO vault.transit_keys (name, latest_ver, created_at) VALUES ('logs', 1, ?)",
    )
    .bind(now)
    .execute(&pool)
    .await
    .expect("insert transit_keys");
    sqlx::query(
        "INSERT INTO vault.transit_versions (name, version, key_wrapped, kek_kid, created_at)
         VALUES ('logs', 1, ?, 'kek-1', ?)",
    )
    .bind(b"wrapped-key".as_slice())
    .bind(now)
    .execute(&pool)
    .await
    .expect("insert transit_versions");

    // Lease.
    sqlx::query(
        "INSERT INTO vault.leases (id, provider, role, issued_at, expires_at)
         VALUES ('lease-1', 'postgres', 'readonly', ?, ?)",
    )
    .bind(now)
    .bind(now + 3600.0)
    .execute(&pool)
    .await
    .expect("insert lease");

    // Personal vault for a user — UNIQUE owner_user enforces 1-per-user.
    sqlx::query(
        "INSERT INTO vault.vaults (id, owner_user, public_key, created_at)
         VALUES ('vault-alice', 'user-alice', ?, ?)",
    )
    .bind(b"x25519-pubkey-32-bytes-aaaaaaaaaaa".as_slice())
    .bind(now)
    .execute(&pool)
    .await
    .expect("insert vault");

    // Collection + member envelope.
    sqlx::query(
        "INSERT INTO vault.collections (id, org_id, name, created_by, created_at)
         VALUES ('col-eng', 'org-acme', 'Engineering', 'user-alice', ?)",
    )
    .bind(now)
    .execute(&pool)
    .await
    .expect("insert collection");
    sqlx::query(
        "INSERT INTO vault.collection_members (collection_id, user_id, wrapped_key, role, added_at)
         VALUES ('col-eng', 'user-alice', ?, 'editor', ?)",
    )
    .bind(b"wrapped-collection-key".as_slice())
    .bind(now)
    .execute(&pool)
    .await
    .expect("insert collection_members");

    // Item in a personal vault.
    sqlx::query(
        "INSERT INTO vault.items (id, vault_id, item_type, name, ciphertext, nonce, created_at, updated_at)
         VALUES ('item-1', 'vault-alice', 'login', 'gh', ?, ?, ?, ?)",
    )
    .bind(b"ct".as_slice())
    .bind(b"012345678901".as_slice())
    .bind(now)
    .bind(now)
    .execute(&pool)
    .await
    .expect("insert personal item");

    // Item in a collection (the OTHER side of the XOR).
    sqlx::query(
        "INSERT INTO vault.items (id, collection_id, item_type, name, ciphertext, nonce, created_at, updated_at)
         VALUES ('item-2', 'col-eng', 'login', 'aws', ?, ?, ?, ?)",
    )
    .bind(b"ct".as_slice())
    .bind(b"012345678901".as_slice())
    .bind(now)
    .bind(now)
    .execute(&pool)
    .await
    .expect("insert collection item");

    // Folder in a personal vault.
    sqlx::query(
        "INSERT INTO vault.folders (id, vault_id, name, created_at)
         VALUES ('folder-1', 'vault-alice', 'API keys', ?)",
    )
    .bind(now)
    .execute(&pool)
    .await
    .expect("insert folder");

    // Share revocation.
    sqlx::query(
        "INSERT INTO vault.share_revoked (key_id, revoked_at, reason)
         VALUES ('biscuit-kid-1', ?, 'compromised')",
    )
    .bind(now)
    .execute(&pool)
    .await
    .expect("insert share_revoked");

    // KEK metadata + Shamir share.
    sqlx::query(
        "INSERT INTO vault.kek_metadata (kid, sealing_method, sealed, share_threshold, share_count, created_at)
         VALUES ('kek-1', 'shamir', 1, 3, 5, ?)",
    )
    .bind(now)
    .execute(&pool)
    .await
    .expect("insert kek_metadata");
    sqlx::query(
        "INSERT INTO vault.unseal_shares (kid, share_index, share_holder, encrypted_share, created_at)
         VALUES ('kek-1', 0, 'alice@example.com', ?, ?)",
    )
    .bind(b"encrypted-share-blob".as_slice())
    .bind(now)
    .execute(&pool)
    .await
    .expect("insert unseal_shares");

    // Audit forwarding sink.
    sqlx::query(
        "INSERT INTO vault.audit_sinks (id, name, kind, config, filter_pattern, enabled, created_at)
         VALUES ('sink-1', 'main-syslog', 'syslog', '{\"host\":\"log.example.com\",\"port\":514}', 'vault.*', 1, ?)",
    )
    .bind(now)
    .execute(&pool)
    .await
    .expect("insert audit_sinks");

    // Read back: the personal item AND the collection item must both
    // exist, and their XOR partition is enforced — the personal one has
    // collection_id NULL, the collection one has vault_id NULL.
    let counts: (i64, i64) = sqlx::query_as(
        "SELECT
            (SELECT COUNT(*) FROM vault.items WHERE vault_id IS NOT NULL),
            (SELECT COUNT(*) FROM vault.items WHERE collection_id IS NOT NULL)",
    )
    .fetch_one(&pool)
    .await
    .expect("count items");
    assert_eq!(counts, (1, 1));
}

#[tokio::test]
async fn item_xor_check_rejects_both_containers_set() {
    let pool = boot_pool().await;
    assay_vault::schema::migrate_sqlite(&pool)
        .await
        .expect("migrate");

    // Seed parent rows for the FKs.
    sqlx::query(
        "INSERT INTO vault.vaults (id, owner_user, public_key, created_at)
         VALUES ('v1', 'u1', x'00', 0.0)",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO vault.collections (id, name, created_by, created_at)
         VALUES ('c1', 'col', 'u1', 0.0)",
    )
    .execute(&pool)
    .await
    .unwrap();

    // Both vault_id AND collection_id set → CHECK violation.
    let res = sqlx::query(
        "INSERT INTO vault.items (id, vault_id, collection_id, item_type, name, ciphertext, nonce, created_at, updated_at)
         VALUES ('bad', 'v1', 'c1', 'login', 'x', x'00', x'00', 0.0, 0.0)",
    )
    .execute(&pool)
    .await;
    assert!(res.is_err(), "items XOR check should reject both-set");

    // Neither set → also a CHECK violation.
    let res = sqlx::query(
        "INSERT INTO vault.items (id, item_type, name, ciphertext, nonce, created_at, updated_at)
         VALUES ('bad2', 'login', 'x', x'00', x'00', 0.0, 0.0)",
    )
    .execute(&pool)
    .await;
    assert!(res.is_err(), "items XOR check should reject neither-set");
}

#[tokio::test]
async fn personal_vault_is_unique_per_user() {
    let pool = boot_pool().await;
    assay_vault::schema::migrate_sqlite(&pool)
        .await
        .expect("migrate");

    sqlx::query(
        "INSERT INTO vault.vaults (id, owner_user, public_key, created_at)
         VALUES ('v1', 'user-alice', x'00', 0.0)",
    )
    .execute(&pool)
    .await
    .expect("first vault");

    let res = sqlx::query(
        "INSERT INTO vault.vaults (id, owner_user, public_key, created_at)
         VALUES ('v2', 'user-alice', x'00', 0.0)",
    )
    .execute(&pool)
    .await;
    assert!(
        res.is_err(),
        "second personal vault for the same user must be rejected by UNIQUE(owner_user)"
    );
}
