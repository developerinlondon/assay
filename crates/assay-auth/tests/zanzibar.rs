//! Integration tests for the Zanzibar layer.
//!
//! Backend coverage:
//!
//! - **SQLite** — always-on. Uses an in-memory `auth` ATTACHment so
//!   the engine.migrations + auth.migrate path runs end-to-end. Cheap
//!   enough for every developer's `cargo test` to exercise.
//! - **Postgres** — gated on `ASSAY_TEST_DATABASE_URL`. Skipped when
//!   the env var is unset (matches the workflow harness pattern).
//!   Boots a fresh schema, runs the migration, and exercises every
//!   public method against a real PG18 instance.
//!
//! Each test focuses on one production-relevant scenario:
//!
//! 1. Round-trip: define namespace, write tuples, check Allowed.
//! 2. Indirect via userset: family#member → circle:viewer hop.
//! 3. Depth bound: 51-deep chain → DepthExceeded.
//! 4. Cycle detection: a→b→a stays sane.
//! 5. Reverse `lookup_subjects` walks transitively.
//! 6. Forward `lookup_resources` filters per subject.
//! 7. Schema parser snapshot — the canonical plan-12c example.
//!
//! Test ergonomics: a small `setup_sqlite` helper builds a pool with
//! the auth db ATTACHed in shared-cache memory mode (matches what
//! `assay-engine::init::sqlite_boot` produces in tests), runs the
//! engine + auth migrations, then hands back a ready
//! `SqliteZanzibarStore`.

#![cfg(any(feature = "backend-sqlite", feature = "backend-postgres"))]
#![cfg(feature = "auth-zanzibar")]

use std::collections::BTreeSet;

use assay_auth::zanzibar::{
    CheckResult, Consistency, NamespaceSchema, ObjectRef, PermissionExpr, RelationDef,
    RelationKind, SubjectRef, Tuple, TypeRef, ZanzibarStore, parse_schema,
};

// ---------- Sqlite-only test setup ----------

#[cfg(feature = "backend-sqlite")]
mod sqlite_tests {
    use super::*;
    use assay_auth::zanzibar::SqliteZanzibarStore;
    use sqlx::SqlitePool;
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
    use std::str::FromStr;
    use std::sync::atomic::{AtomicU64, Ordering};

    static SEQ: AtomicU64 = AtomicU64::new(0);

    /// Build a SqlitePool that mirrors the engine boot's ATTACH layout:
    /// `engine` + `auth` databases backed by named in-memory shared-cache
    /// stores. Runs the engine migration to create `engine.migrations`
    /// (auth's migrate fn writes to it), then runs the auth migration
    /// up to V3 — yielding `auth.zanzibar_*` tables ready for tests.
    pub async fn setup_sqlite() -> SqliteZanzibarStore {
        let suffix = format!(
            "{}_{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        );
        let engine_uri = format!("file:assay_eng_{suffix}?mode=memory&cache=shared");
        let auth_uri = format!("file:assay_auth_{suffix}?mode=memory&cache=shared");

        let opts = SqliteConnectOptions::from_str("sqlite::memory:")
            .unwrap()
            .create_if_missing(true);

        let pool: SqlitePool = SqlitePoolOptions::new()
            .max_connections(1)
            .after_connect(move |conn, _meta| {
                let engine_uri = engine_uri.clone();
                let auth_uri = auth_uri.clone();
                Box::pin(async move {
                    use sqlx::Executor;
                    conn.execute(format!("ATTACH DATABASE '{engine_uri}' AS engine").as_str())
                        .await?;
                    conn.execute(format!("ATTACH DATABASE '{auth_uri}' AS auth").as_str())
                        .await?;
                    Ok(())
                })
            })
            .connect_with(opts)
            .await
            .expect("connect sqlite pool");

        // Minimal `engine.migrations` table — we don't need the full
        // engine schema for these tests, just the row receiver.
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS engine.migrations (
                module TEXT NOT NULL,
                version INTEGER NOT NULL,
                PRIMARY KEY (module, version)
            )",
        )
        .execute(&pool)
        .await
        .expect("create engine.migrations");

        assay_auth::schema::migrate_sqlite(&pool)
            .await
            .expect("run auth migration");

        SqliteZanzibarStore::new(pool)
    }

    #[tokio::test]
    async fn roundtrip_define_write_check() {
        let store = setup_sqlite().await;
        seed_doc_namespace(&store).await;

        let alice = SubjectRef::direct("user", "alice");
        let doc = ObjectRef::new("document", "x");

        // Write a direct tuple: alice is owner.
        store
            .write_tuple(&Tuple::direct(doc.clone(), "owner", alice.clone()))
            .await
            .expect("write tuple");

        // Owner permission resolves via `view = owner + viewer`.
        let res = store
            .check(&doc, "view", &alice, Consistency::Minimum)
            .await
            .expect("check");
        assert!(matches!(res, CheckResult::Allowed { .. }), "{res:?}");

        // A non-owner is denied.
        let bob = SubjectRef::direct("user", "bob");
        let res = store
            .check(&doc, "view", &bob, Consistency::Minimum)
            .await
            .expect("check bob");
        assert_eq!(res, CheckResult::Denied);
    }

    // A computed-userset arrow (`parent->up`) must resolve a hierarchy of
    // DIRECT object references, re-evaluating the target permission at each
    // hop — the standard Zanzibar arrow. Nodes may carry their own account
    // *and* a parent at every level (the case a hand-threaded userset chain
    // cannot express).
    #[tokio::test]
    async fn computed_userset_arrow_resolves_hierarchy() {
        let store = setup_sqlite().await;
        store
            .define_namespace(&NamespaceSchema::new("user"))
            .await
            .expect("ns user");
        store
            .define_namespace(
                &NamespaceSchema::new("node")
                    .with_relation(
                        "account",
                        RelationDef::relation("account", vec![TypeRef::direct("user")]),
                    )
                    .with_relation(
                        "parent",
                        RelationDef::relation("parent", vec![TypeRef::direct("node")]),
                    )
                    .with_relation(
                        "up",
                        RelationDef::permission(
                            "up",
                            PermissionExpr::union(
                                PermissionExpr::direct("account"),
                                PermissionExpr::arrow("parent", "up"),
                            ),
                        ),
                    ),
            )
            .await
            .expect("ns node");

        // c.parent = b, b.parent = a. Every level has its own account.
        // One DIRECT object tuple per parent edge (no hand-threaded usersets).
        store
            .write_tuples(&[
                Tuple::direct(
                    ObjectRef::new("node", "a"),
                    "account",
                    SubjectRef::direct("user", "ua"),
                ),
                Tuple::direct(
                    ObjectRef::new("node", "b"),
                    "account",
                    SubjectRef::direct("user", "ub"),
                ),
                Tuple::direct(
                    ObjectRef::new("node", "c"),
                    "account",
                    SubjectRef::direct("user", "uc"),
                ),
                Tuple::direct(
                    ObjectRef::new("node", "b"),
                    "parent",
                    SubjectRef::direct("node", "a"),
                ),
                Tuple::direct(
                    ObjectRef::new("node", "c"),
                    "parent",
                    SubjectRef::direct("node", "b"),
                ),
            ])
            .await
            .expect("seed");

        // ua is the account on `a`, c's grandparent: up(c) must include ua via c->b->a.
        let r_grand = store
            .check(
                &ObjectRef::new("node", "c"),
                "up",
                &SubjectRef::direct("user", "ua"),
                Consistency::Minimum,
            )
            .await
            .expect("check ua");
        assert!(
            matches!(r_grand, CheckResult::Allowed { .. }),
            "ua (grandparent) must be in up(c): {r_grand:?}"
        );

        // ub is the account on `b` — a node with BOTH an account and a parent.
        let r_mid = store
            .check(
                &ObjectRef::new("node", "c"),
                "up",
                &SubjectRef::direct("user", "ub"),
                Consistency::Minimum,
            )
            .await
            .expect("check ub");
        assert!(
            matches!(r_mid, CheckResult::Allowed { .. }),
            "ub (parent, has account) must be in up(c): {r_mid:?}"
        );

        // An unrelated user is denied.
        let r_no = store
            .check(
                &ObjectRef::new("node", "c"),
                "up",
                &SubjectRef::direct("user", "nobody"),
                Consistency::Minimum,
            )
            .await
            .expect("check nobody");
        assert_eq!(
            r_no,
            CheckResult::Denied,
            "unrelated user must be denied: {r_no:?}"
        );
    }

    #[tokio::test]
    async fn indirect_via_userset() {
        let store = setup_sqlite().await;
        // Two namespaces — `family` (with `member`) and `circle` (with
        // `viewer` granted to `family#member`).
        store
            .define_namespace(&NamespaceSchema::new("user"))
            .await
            .expect("ns user");
        store
            .define_namespace(&NamespaceSchema::new("family").with_relation(
                "member",
                RelationDef::relation("member", vec![TypeRef::direct("user")]),
            ))
            .await
            .expect("ns family");
        store
            .define_namespace(
                &NamespaceSchema::new("circle")
                    .with_relation(
                        "viewer",
                        RelationDef::relation("viewer", vec![TypeRef::userset("family", "member")]),
                    )
                    .with_relation(
                        "view",
                        RelationDef::permission("view", PermissionExpr::direct("viewer")),
                    ),
            )
            .await
            .expect("ns circle");

        // Tuples:
        //   family:ahmed#member @ user:alice
        //   circle:immediate#viewer @ family:ahmed#member
        store
            .write_tuples(&[
                Tuple::direct(
                    ObjectRef::new("family", "ahmed"),
                    "member",
                    SubjectRef::direct("user", "alice"),
                ),
                Tuple::direct(
                    ObjectRef::new("circle", "immediate"),
                    "viewer",
                    SubjectRef::userset("family", "ahmed", "member"),
                ),
            ])
            .await
            .expect("write");

        let res = store
            .check(
                &ObjectRef::new("circle", "immediate"),
                "view",
                &SubjectRef::direct("user", "alice"),
                Consistency::Minimum,
            )
            .await
            .expect("check");
        assert!(
            matches!(res, CheckResult::Allowed { .. }),
            "alice should view via family hop, got {res:?}"
        );

        // Bob isn't in the family — denied.
        let res = store
            .check(
                &ObjectRef::new("circle", "immediate"),
                "view",
                &SubjectRef::direct("user", "bob"),
                Consistency::Minimum,
            )
            .await
            .expect("check");
        assert_eq!(res, CheckResult::Denied);
    }

    #[tokio::test]
    async fn depth_bound_triggers_depth_exceeded() {
        let store = setup_sqlite().await;
        // Single namespace `chain` with `member` direct + `view` perm.
        store
            .define_namespace(&NamespaceSchema::new("user"))
            .await
            .expect("ns");
        store
            .define_namespace(
                &NamespaceSchema::new("chain")
                    .with_relation(
                        "member",
                        RelationDef::relation(
                            "member",
                            vec![TypeRef::direct("user"), TypeRef::userset("chain", "member")],
                        ),
                    )
                    .with_relation(
                        "view",
                        RelationDef::permission("view", PermissionExpr::direct("member")),
                    ),
            )
            .await
            .expect("ns chain");

        // Build a 60-deep chain of usersets — each hop is
        //   chain:n#member @ chain:(n+1)#member
        // The last hop terminates at user:alice.
        let mut tuples = Vec::new();
        for n in 0..60u32 {
            tuples.push(Tuple::direct(
                ObjectRef::new("chain", n.to_string()),
                "member",
                SubjectRef::userset("chain", (n + 1).to_string(), "member"),
            ));
        }
        // Terminal: chain:60#member @ user:alice.
        tuples.push(Tuple::direct(
            ObjectRef::new("chain", "60"),
            "member",
            SubjectRef::direct("user", "alice"),
        ));
        store.write_tuples(&tuples).await.expect("seed chain");

        // chain:0 -> alice is 61 hops; max depth is 50.
        let res = store
            .check(
                &ObjectRef::new("chain", "0"),
                "view",
                &SubjectRef::direct("user", "alice"),
                Consistency::Minimum,
            )
            .await
            .expect("check");
        assert_eq!(
            res,
            CheckResult::DepthExceeded,
            "expected DepthExceeded for 61-hop chain"
        );
    }

    #[tokio::test]
    async fn cycle_detection_is_safe() {
        let store = setup_sqlite().await;
        store
            .define_namespace(&NamespaceSchema::new("user"))
            .await
            .expect("ns");
        store
            .define_namespace(
                &NamespaceSchema::new("ring")
                    .with_relation(
                        "member",
                        RelationDef::relation(
                            "member",
                            vec![TypeRef::direct("user"), TypeRef::userset("ring", "member")],
                        ),
                    )
                    .with_relation(
                        "view",
                        RelationDef::permission("view", PermissionExpr::direct("member")),
                    ),
            )
            .await
            .expect("ns ring");

        // Two-node cycle: a member of b, b member of a, neither
        // terminating at a real user.
        store
            .write_tuples(&[
                Tuple::direct(
                    ObjectRef::new("ring", "a"),
                    "member",
                    SubjectRef::userset("ring", "b", "member"),
                ),
                Tuple::direct(
                    ObjectRef::new("ring", "b"),
                    "member",
                    SubjectRef::userset("ring", "a", "member"),
                ),
            ])
            .await
            .expect("seed cycle");

        // Carol is unreachable — the walk must terminate (not loop)
        // and return Denied.
        let res = store
            .check(
                &ObjectRef::new("ring", "a"),
                "view",
                &SubjectRef::direct("user", "carol"),
                Consistency::Minimum,
            )
            .await
            .expect("check");
        assert_eq!(res, CheckResult::Denied);
    }

    #[tokio::test]
    async fn lookup_subjects_walks_transitively() {
        let store = setup_sqlite().await;
        seed_doc_namespace(&store).await;
        // Add a group with bob as member, then grant the group viewer.
        store
            .define_namespace(&NamespaceSchema::new("user"))
            .await
            .expect("ns user");
        store
            .define_namespace(&NamespaceSchema::new("group").with_relation(
                "member",
                RelationDef::relation("member", vec![TypeRef::direct("user")]),
            ))
            .await
            .expect("ns group");

        store
            .write_tuples(&[
                Tuple::direct(
                    ObjectRef::new("document", "y"),
                    "owner",
                    SubjectRef::direct("user", "alice"),
                ),
                Tuple::direct(
                    ObjectRef::new("group", "g1"),
                    "member",
                    SubjectRef::direct("user", "bob"),
                ),
                Tuple::direct(
                    ObjectRef::new("document", "y"),
                    "viewer",
                    SubjectRef::userset("group", "g1", "member"),
                ),
            ])
            .await
            .expect("seed lookups");

        let subjects = store
            .lookup_subjects("user", &ObjectRef::new("document", "y"), "view")
            .await
            .expect("lookup");
        let ids: BTreeSet<String> = subjects.iter().map(|s| s.subject_id.clone()).collect();
        assert!(ids.contains("alice"), "expected alice; got {ids:?}");
        assert!(
            ids.contains("bob"),
            "expected bob via group hop; got {ids:?}"
        );
    }

    #[tokio::test]
    async fn lookup_resources_returns_only_allowed() {
        let store = setup_sqlite().await;
        seed_doc_namespace(&store).await;
        // Three docs: alice owns 1, charlie owns 3, both own 2.
        store
            .write_tuples(&[
                Tuple::direct(
                    ObjectRef::new("document", "1"),
                    "owner",
                    SubjectRef::direct("user", "alice"),
                ),
                Tuple::direct(
                    ObjectRef::new("document", "2"),
                    "owner",
                    SubjectRef::direct("user", "alice"),
                ),
                Tuple::direct(
                    ObjectRef::new("document", "2"),
                    "owner",
                    SubjectRef::direct("user", "charlie"),
                ),
                Tuple::direct(
                    ObjectRef::new("document", "3"),
                    "owner",
                    SubjectRef::direct("user", "charlie"),
                ),
            ])
            .await
            .expect("seed");

        let alice_docs = store
            .lookup_resources("document", "view", &SubjectRef::direct("user", "alice"))
            .await
            .expect("lookup");
        let ids: BTreeSet<String> = alice_docs.iter().map(|o| o.object_id.clone()).collect();
        assert_eq!(ids, BTreeSet::from(["1".into(), "2".into()]), "got {ids:?}");
    }

    #[tokio::test]
    async fn delete_tuple_round_trips() {
        let store = setup_sqlite().await;
        seed_doc_namespace(&store).await;
        let t = Tuple::direct(
            ObjectRef::new("document", "z"),
            "owner",
            SubjectRef::direct("user", "alice"),
        );
        store.write_tuple(&t).await.expect("write");
        assert!(store.delete_tuple(&t).await.expect("delete first"));
        assert!(!store.delete_tuple(&t).await.expect("delete twice"));
    }

    /// Re-defining a namespace overwrites the previous schema.
    #[tokio::test]
    async fn define_namespace_is_idempotent() {
        let store = setup_sqlite().await;
        let ns = NamespaceSchema::new("ns").with_relation(
            "r",
            RelationDef::relation("r", vec![TypeRef::direct("user")]),
        );
        store.define_namespace(&ns).await.expect("first");
        store.define_namespace(&ns).await.expect("second");
        let fetched = store
            .get_namespace("ns")
            .await
            .expect("get")
            .expect("present");
        assert_eq!(fetched, ns);
    }

    /// Helper — defines the canonical `document` namespace shared by
    /// most round-trip tests. `view = owner + viewer`, `edit = owner`.
    async fn seed_doc_namespace(store: &SqliteZanzibarStore) {
        let ns = NamespaceSchema::new("document")
            .with_relation(
                "owner",
                RelationDef::relation("owner", vec![TypeRef::direct("user")]),
            )
            .with_relation(
                "viewer",
                RelationDef::relation(
                    "viewer",
                    vec![TypeRef::direct("user"), TypeRef::userset("group", "member")],
                ),
            )
            .with_relation(
                "view",
                RelationDef::permission(
                    "view",
                    PermissionExpr::union(
                        PermissionExpr::direct("owner"),
                        PermissionExpr::direct("viewer"),
                    ),
                ),
            )
            .with_relation(
                "edit",
                RelationDef::permission("edit", PermissionExpr::direct("owner")),
            );
        store.define_namespace(&ns).await.expect("seed doc ns");
    }
}

// ---------- Postgres tests (gated on env) ----------

#[cfg(feature = "backend-postgres")]
mod postgres_tests {
    use super::*;
    use assay_auth::zanzibar::PostgresZanzibarStore;
    use sqlx::PgPool;

    /// Skip helper — returns `Some(PgPool)` if `ASSAY_TEST_DATABASE_URL`
    /// is set and the engine + auth schemas migrate cleanly. Returns
    /// `None` to skip the test on dev machines without docker/PG.
    async fn maybe_setup_pg() -> Option<PostgresZanzibarStore> {
        let url = std::env::var("ASSAY_TEST_DATABASE_URL").ok()?;
        let pool = PgPool::connect(&url).await.ok()?;
        // Minimal `engine` schema scaffold (the auth migrate writes
        // into `engine.migrations`).
        sqlx::query("CREATE SCHEMA IF NOT EXISTS engine")
            .execute(&pool)
            .await
            .ok()?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS engine.migrations (
                module TEXT NOT NULL,
                version INTEGER NOT NULL,
                PRIMARY KEY (module, version)
            )",
        )
        .execute(&pool)
        .await
        .ok()?;
        // Drop any prior auth schema so each run starts fresh.
        sqlx::query("DROP SCHEMA IF EXISTS auth CASCADE")
            .execute(&pool)
            .await
            .ok()?;
        assay_auth::schema::migrate_postgres(&pool).await.ok()?;
        Some(PostgresZanzibarStore::new(pool))
    }

    #[tokio::test]
    async fn pg_roundtrip_check() {
        let Some(store) = maybe_setup_pg().await else {
            eprintln!("ASSAY_TEST_DATABASE_URL unset — skipping pg_roundtrip_check");
            return;
        };
        let ns = NamespaceSchema::new("document")
            .with_relation(
                "owner",
                RelationDef::relation("owner", vec![TypeRef::direct("user")]),
            )
            .with_relation(
                "viewer",
                RelationDef::relation("viewer", vec![TypeRef::direct("user")]),
            )
            .with_relation(
                "view",
                RelationDef::permission(
                    "view",
                    PermissionExpr::union(
                        PermissionExpr::direct("owner"),
                        PermissionExpr::direct("viewer"),
                    ),
                ),
            );
        store.define_namespace(&ns).await.expect("ns");

        let doc = ObjectRef::new("document", "pg-x");
        let alice = SubjectRef::direct("user", "alice");
        store
            .write_tuple(&Tuple::direct(doc.clone(), "owner", alice.clone()))
            .await
            .expect("write");

        let res = store
            .check(&doc, "view", &alice, Consistency::Minimum)
            .await
            .expect("check");
        assert!(matches!(res, CheckResult::Allowed { .. }));

        let bob = SubjectRef::direct("user", "bob");
        assert_eq!(
            store
                .check(&doc, "view", &bob, Consistency::Minimum)
                .await
                .expect("check"),
            CheckResult::Denied
        );
    }
}

// ---------- Schema parser snapshot (no backend needed) ----------

/// Parses the canonical example from plan 12c lines 1097-1110 and
/// asserts each definition / relation lands in the parsed AST.
#[test]
fn parser_snapshot_matches_plan_example() {
    let src = r#"
        definition user {}

        definition group {
            relation member: user
        }

        definition document {
            relation owner: user
            relation viewer: user | group#member
            permission view = owner + viewer
            permission edit = owner
        }
    "#;
    let nss = parse_schema(src).expect("parse");
    let by_name: std::collections::BTreeMap<&str, &NamespaceSchema> =
        nss.iter().map(|n| (n.name.as_str(), n)).collect();
    assert_eq!(by_name.len(), 3);

    let group = by_name.get("group").expect("group ns");
    let member = group.definitions.get("member").expect("member rel");
    assert!(matches!(member.kind, RelationKind::Direct(_)));

    let doc = by_name.get("document").expect("document ns");
    let viewer = doc.definitions.get("viewer").expect("viewer rel");
    match &viewer.kind {
        RelationKind::Direct(types) => {
            assert_eq!(types.len(), 2);
            assert_eq!(types[1].relation.as_deref(), Some("member"));
        }
        _ => panic!("viewer should be Direct"),
    }
    let view = doc.definitions.get("view").expect("view perm");
    assert!(matches!(view.kind, RelationKind::Permission(_)));
}
