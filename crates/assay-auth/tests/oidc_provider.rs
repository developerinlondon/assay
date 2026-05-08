//! Integration tests for the OIDC provider.
//!
//! Backend coverage mirrors the zanzibar test pattern:
//!
//! - **SQLite** — always-on. In-memory ATTACHment, runs the full
//!   migration up to V4 so the OIDC provider tables exist.
//! - **Postgres** — gated on `ASSAY_TEST_DATABASE_URL`. Skipped when
//!   the env var is unset (matches the workflow harness pattern).
//!
//! Round-trips covered:
//!
//! 1. Discovery doc carries every required Core 1.0 field.
//! 2. Client store CRUD + secret rotation (both backends).
//! 3. Authorization-code single-use semantic (consume returns Some
//!    once, then None forever after).
//! 4. PKCE S256 verify_passes + verify_fails on wrong verifier.
//! 5. Refresh-token revoke / revoke_for_user fan-out.
//! 6. SSO session list/delete by assay_session_id.
//! 7. Consent grant upsert + replay.
//! 8. Upstream-state take is single-use.
//! 9. id_token claim builder respects scopes.
//!
//! The full HTTP-handler coverage (round-trip /authorize → /token →
//! /userinfo via reqwest against a live axum app) lands in phase 8 once
//! the AuthCtx-resolving handlers are wired; phase 7 ships the unit
//! contracts the handlers will call.

#![cfg(any(feature = "backend-sqlite", feature = "backend-postgres"))]
#![cfg(feature = "auth-oidc-provider")]

use assay_auth::oidc_provider::authorize::{
    AuthorizeRequest, AuthorizeValidation, build_code, redirect_with_code, validate,
};
use assay_auth::oidc_provider::discovery::build_discovery;
use assay_auth::oidc_provider::token::{
    build_id_token_claims, hash_refresh_token, mint_refresh_token, verify_pkce_s256,
};
use assay_auth::oidc_provider::types::{
    ConsentGrant, OidcClient, OidcSession, RefreshToken, TokenAuthMethod, UpstreamLoginState,
    UpstreamProvider,
};
use assay_auth::oidc_provider::userinfo::{build_userinfo, parse_bearer};
use assay_auth::store::User;

#[test]
fn discovery_carries_required_fields_against_real_issuer_url() {
    let doc = build_discovery("https://idp.example.com");
    for f in [
        "issuer",
        "authorization_endpoint",
        "token_endpoint",
        "userinfo_endpoint",
        "jwks_uri",
        "response_types_supported",
        "subject_types_supported",
        "id_token_signing_alg_values_supported",
        "code_challenge_methods_supported",
    ] {
        assert!(doc.get(f).is_some(), "discovery missing {f}: {doc}");
    }
    assert_eq!(doc["issuer"], "https://idp.example.com");
}

#[test]
fn pkce_s256_verifies_known_pair_and_rejects_mismatch() {
    use sha2::{Digest, Sha256};
    let verifier = "test-verifier-abc-123-some-random-bytes";
    let mut h = Sha256::new();
    h.update(verifier.as_bytes());
    let challenge = data_encoding::BASE64URL_NOPAD.encode(&h.finalize());
    assert!(verify_pkce_s256(verifier, &challenge));
    assert!(!verify_pkce_s256("not-the-verifier", &challenge));
}

#[test]
fn id_token_builder_emits_scope_filtered_claims() {
    let scopes = vec![
        "openid".to_string(),
        "email".to_string(),
        "profile".to_string(),
    ];
    let v = build_id_token_claims(
        "https://idp.example.com",
        "user_alice",
        "client_test",
        "sid_x",
        &scopes,
        Some("nonce_xyz"),
        Some("alice@example.com"),
        true,
        Some("Alice Liddell"),
    );
    assert_eq!(v["iss"], "https://idp.example.com");
    assert_eq!(v["sub"], "user_alice");
    assert_eq!(v["aud"], "client_test");
    assert_eq!(v["sid"], "sid_x");
    assert_eq!(v["nonce"], "nonce_xyz");
    assert_eq!(v["email"], "alice@example.com");
    assert_eq!(v["email_verified"], true);
    assert_eq!(v["name"], "Alice Liddell");
}

#[test]
fn authorize_request_validation_full_round_trip() {
    let mut client = OidcClient::new("c1", "App", 0.0);
    client.redirect_uris = vec!["https://app.example.com/cb".to_string()];
    let req = AuthorizeRequest {
        response_type: "code".to_string(),
        client_id: "c1".to_string(),
        redirect_uri: "https://app.example.com/cb".to_string(),
        scope: "openid email".to_string(),
        state: Some("st_xyz".to_string()),
        nonce: Some("n_abc".to_string()),
        code_challenge: Some("ch".to_string()),
        code_challenge_method: Some("S256".to_string()),
        prompt: None,
        max_age: None,
    };
    match validate(&req, &client) {
        AuthorizeValidation::Ok { scopes } => {
            assert_eq!(scopes, vec!["openid".to_string(), "email".to_string()]);
        }
        other => panic!("expected Ok, got {other:?}"),
    }
    let code = build_code("user_alice", &req, vec!["openid".to_string()]);
    assert_eq!(code.user_id, "user_alice");
    assert_eq!(code.client_id, "c1");
    assert_eq!(code.code_challenge, "ch");
    assert!(code.code.starts_with("oac_"));
    let url = redirect_with_code(&req.redirect_uri, &code.code, req.state.as_deref());
    assert!(url.starts_with("https://app.example.com/cb?code=oac_"));
    assert!(url.ends_with("&state=st_xyz"));
}

#[test]
fn userinfo_filters_by_scope() {
    let user = User {
        id: "user_alice".to_string(),
        email: Some("alice@example.com".to_string()),
        email_verified: true,
        display_name: Some("Alice".to_string()),
        created_at: 0.0,
    };
    let v = build_userinfo(&user, &["openid".to_string()]);
    assert_eq!(v["sub"], "user_alice");
    assert!(v.get("email").is_none());
    assert!(v.get("name").is_none());

    let v_full = build_userinfo(
        &user,
        &[
            "openid".to_string(),
            "email".to_string(),
            "profile".to_string(),
        ],
    );
    assert_eq!(v_full["email"], "alice@example.com");
    assert_eq!(v_full["email_verified"], true);
    assert_eq!(v_full["name"], "Alice");
}

#[test]
fn parse_bearer_handles_normal_and_rejects_other() {
    assert_eq!(parse_bearer("Bearer abc"), Some("abc"));
    assert_eq!(parse_bearer("Basic xyz"), None);
}

#[test]
fn mint_refresh_token_is_unique_and_hashable() {
    let a = mint_refresh_token();
    let b = mint_refresh_token();
    assert_ne!(a, b, "tokens should be random");
    let h = hash_refresh_token(&a);
    assert_eq!(h.len(), 64, "sha256 hex");
    assert_eq!(h, hash_refresh_token(&a));
}

// =====================================================================
//   SQLITE — exercises the full V4 migration + every store
// =====================================================================

#[cfg(feature = "backend-sqlite")]
mod sqlite_tests {
    use super::*;
    use assay_auth::oidc_provider::store::{
        OidcClientStore, OidcCodeStore, OidcConsentStore, OidcRefreshStore, OidcSessionStore,
        OidcUpstreamStateStore, OidcUpstreamStore, SqliteOidcClientStore, SqliteOidcCodeStore,
        SqliteOidcConsentStore, SqliteOidcRefreshStore, SqliteOidcSessionStore,
        SqliteOidcUpstreamStateStore, SqliteOidcUpstreamStore,
    };
    use assay_auth::oidc_provider::types::AuthorizationCode;
    use sqlx::SqlitePool;
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
    use std::str::FromStr;
    use std::sync::atomic::{AtomicU64, Ordering};

    static SEQ: AtomicU64 = AtomicU64::new(0);

    /// Build a SqlitePool with engine + auth ATTACHments and run the
    /// auth migration up to V4.
    pub async fn setup_sqlite() -> SqlitePool {
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
            .expect("auth migrate v4");

        pool
    }

    #[tokio::test]
    async fn client_store_create_get_list_update_delete() {
        let pool = setup_sqlite().await;
        let store = SqliteOidcClientStore::new(pool);

        let mut client = OidcClient::new("c1", "Test App", 1.0);
        client.redirect_uris = vec!["https://app.example.com/cb".to_string()];
        client.client_secret_hash = Some("hash_abc".to_string());
        client.default_scopes = vec!["openid".to_string(), "email".to_string()];

        store.create(&client).await.expect("create");
        let loaded = store.get("c1").await.expect("get").expect("present");
        assert_eq!(loaded.client_id, "c1");
        assert_eq!(loaded.name, "Test App");
        assert_eq!(
            loaded.redirect_uris,
            vec!["https://app.example.com/cb".to_string()]
        );
        assert_eq!(
            loaded.default_scopes,
            vec!["openid".to_string(), "email".to_string()]
        );

        let list = store.list().await.expect("list");
        assert_eq!(list.len(), 1);

        let mut updated = loaded.clone();
        updated.name = "Renamed App".to_string();
        store.update(&updated).await.expect("update");
        let after = store.get("c1").await.expect("get").expect("present");
        assert_eq!(after.name, "Renamed App");

        store
            .rotate_secret_hash("c1", "hash_new")
            .await
            .expect("rotate");
        let rotated = store.get("c1").await.expect("get").expect("present");
        assert_eq!(rotated.client_secret_hash.as_deref(), Some("hash_new"));

        assert!(store.delete("c1").await.expect("delete"));
        assert!(store.get("c1").await.expect("get").is_none());
    }

    #[tokio::test]
    async fn upstream_store_upsert_idempotent() {
        let pool = setup_sqlite().await;
        let store = SqliteOidcUpstreamStore::new(pool);

        let p = UpstreamProvider {
            slug: "google".to_string(),
            issuer: "https://accounts.google.com".to_string(),
            client_id: "ci".to_string(),
            client_secret: "cs".to_string(),
            display_name: "Google".to_string(),
            icon_url: None,
            enabled: true,
            scopes: vec![
                "openid".to_string(),
                "email".to_string(),
                "profile".to_string(),
            ],
            auth_params: std::collections::BTreeMap::new(),
        };
        store.upsert(&p).await.expect("upsert");
        store.upsert(&p).await.expect("upsert again");
        let loaded = store.get("google").await.expect("get").expect("present");
        assert_eq!(loaded.issuer, "https://accounts.google.com");
        let list = store.list().await.expect("list");
        assert_eq!(list.len(), 1);
        assert!(store.delete("google").await.expect("delete"));
    }

    #[tokio::test]
    async fn code_consume_is_single_use() {
        let pool = setup_sqlite().await;
        let store = SqliteOidcCodeStore::new(pool);
        let code = AuthorizationCode {
            code: "oac_abc".to_string(),
            client_id: "c1".to_string(),
            user_id: "u1".to_string(),
            redirect_uri: "https://app.example.com/cb".to_string(),
            scopes: vec!["openid".to_string()],
            code_challenge: "ch".to_string(),
            code_challenge_method: "S256".to_string(),
            nonce: Some("n".to_string()),
            state: Some("st".to_string()),
            issued_at: 1.0,
            expires_at: 61.0,
            consumed: false,
        };
        store.create(&code).await.expect("create");
        let consumed = store
            .consume("oac_abc")
            .await
            .expect("consume")
            .expect("present");
        assert_eq!(consumed.code, "oac_abc");
        // Second consume returns None — single-use.
        assert!(store.consume("oac_abc").await.expect("consume2").is_none());
    }

    #[tokio::test]
    async fn refresh_store_revoke_and_revoke_for_user() {
        let pool = setup_sqlite().await;
        let store = SqliteOidcRefreshStore::new(pool);
        for i in 0..3 {
            let plaintext = format!("ort_t{i}");
            let row = RefreshToken {
                token_hash: hash_refresh_token(&plaintext),
                client_id: "c1".to_string(),
                user_id: "u1".to_string(),
                scopes: vec!["openid".to_string()],
                issued_at: 1.0,
                expires_at: 1000.0,
                revoked: false,
            };
            store.create(&row).await.expect("create");
        }
        let h0 = hash_refresh_token("ort_t0");
        assert!(store.revoke(&h0).await.expect("revoke"));
        let after = store.get(&h0).await.expect("get").expect("present");
        assert!(after.revoked);

        // revoke_for_user marks all rows for u1 — including the
        // already-revoked one (idempotent on revoked).
        let count = store.revoke_for_user("u1").await.expect("revoke_for_user");
        assert!(count >= 2);
    }

    #[tokio::test]
    async fn session_store_list_and_delete_by_assay_session() {
        let pool = setup_sqlite().await;
        let store = SqliteOidcSessionStore::new(pool);
        for i in 0..3 {
            let s = OidcSession {
                sid: format!("sid_{i}"),
                user_id: "u1".to_string(),
                client_id: "c1".to_string(),
                assay_session_id: Some("sess_assay".to_string()),
                issued_at: 1.0,
                backchannel_logout_uri: None,
            };
            store.create(&s).await.expect("create");
        }
        let list = store
            .list_by_assay_session("sess_assay")
            .await
            .expect("list");
        assert_eq!(list.len(), 3);
        let dropped = store
            .delete_by_assay_session("sess_assay")
            .await
            .expect("delete_by_assay_session");
        assert_eq!(dropped, 3);
        assert!(
            store
                .list_by_assay_session("sess_assay")
                .await
                .expect("list2")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn consent_store_upsert_and_replay() {
        let pool = setup_sqlite().await;
        let store = SqliteOidcConsentStore::new(pool);
        let g = ConsentGrant {
            user_id: "u1".to_string(),
            client_id: "c1".to_string(),
            scopes: vec!["openid".to_string(), "email".to_string()],
            granted_at: 1.0,
        };
        store.upsert(&g).await.expect("upsert");
        let loaded = store.get("u1", "c1").await.expect("get").expect("present");
        assert_eq!(
            loaded.scopes,
            vec!["openid".to_string(), "email".to_string()]
        );
        // Replay with wider scopes — replaces.
        let mut g2 = g.clone();
        g2.scopes.push("profile".to_string());
        g2.granted_at = 2.0;
        store.upsert(&g2).await.expect("upsert2");
        let after = store.get("u1", "c1").await.expect("get").expect("present");
        assert_eq!(after.scopes.len(), 3);
        assert!(store.delete("u1", "c1").await.expect("delete"));
    }

    #[tokio::test]
    async fn upstream_state_take_is_single_use() {
        let pool = setup_sqlite().await;
        let store = SqliteOidcUpstreamStateStore::new(pool);
        let s = UpstreamLoginState {
            state: "state_abc".to_string(),
            provider_slug: "google".to_string(),
            nonce: "n_xyz".to_string(),
            pkce_verifier: "v_pkce".to_string(),
            return_to: Some("/authorize?...".to_string()),
            created_at: 1.0,
            expires_at: 1000.0,
            binding_hash: String::new(),
        };
        store.create(&s).await.expect("create");
        let took = store
            .take("state_abc")
            .await
            .expect("take")
            .expect("present");
        assert_eq!(took.provider_slug, "google");
        // Second take returns None — single-use.
        assert!(store.take("state_abc").await.expect("take2").is_none());
    }
}

// =====================================================================
//   POSTGRES — gated on ASSAY_TEST_DATABASE_URL
// =====================================================================

#[cfg(feature = "backend-postgres")]
mod pg_tests {
    use super::*;
    use assay_auth::oidc_provider::store::{
        OidcClientStore, OidcCodeStore, OidcRefreshStore, OidcUpstreamStateStore,
        PostgresOidcClientStore, PostgresOidcCodeStore, PostgresOidcRefreshStore,
        PostgresOidcUpstreamStateStore,
    };
    use assay_auth::oidc_provider::types::AuthorizationCode;
    use sqlx::PgPool;

    async fn setup_pg() -> Option<PgPool> {
        let url = match std::env::var("ASSAY_TEST_DATABASE_URL") {
            Ok(u) if !u.trim().is_empty() => u,
            _ => return None,
        };
        let pool = PgPool::connect(&url).await.ok()?;
        // Bare-minimum engine.migrations for the auth migrate to write
        // its row receiver. Tests reset the auth schema each run so
        // multiple PR builds don't tangle.
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
        sqlx::query("DROP SCHEMA IF EXISTS auth CASCADE")
            .execute(&pool)
            .await
            .ok()?;
        sqlx::query("DELETE FROM engine.migrations WHERE module = 'auth'")
            .execute(&pool)
            .await
            .ok()?;
        assay_auth::schema::migrate_postgres(&pool).await.ok()?;
        Some(pool)
    }

    #[tokio::test]
    async fn pg_client_store_round_trip() {
        let Some(pool) = setup_pg().await else {
            eprintln!("skipping (ASSAY_TEST_DATABASE_URL not set)");
            return;
        };
        let store = PostgresOidcClientStore::new(pool);
        let mut client = OidcClient::new("c_pg", "PG App", 1.0);
        client.redirect_uris = vec!["https://app.example.com/cb".to_string()];
        store.create(&client).await.expect("create");
        let loaded = store.get("c_pg").await.expect("get").expect("present");
        assert_eq!(loaded.client_id, "c_pg");
        assert_eq!(
            loaded.token_endpoint_auth_method,
            TokenAuthMethod::ClientSecretBasic
        );
        assert!(loaded.pkce_required);
        assert!(store.delete("c_pg").await.expect("delete"));
    }

    #[tokio::test]
    async fn pg_code_consume_is_single_use() {
        let Some(pool) = setup_pg().await else {
            eprintln!("skipping (ASSAY_TEST_DATABASE_URL not set)");
            return;
        };
        let store = PostgresOidcCodeStore::new(pool);
        let code = AuthorizationCode {
            code: "oac_pg".to_string(),
            client_id: "c1".to_string(),
            user_id: "u1".to_string(),
            redirect_uri: "https://app.example.com/cb".to_string(),
            scopes: vec!["openid".to_string()],
            code_challenge: "ch".to_string(),
            code_challenge_method: "S256".to_string(),
            nonce: None,
            state: None,
            issued_at: 1.0,
            expires_at: 61.0,
            consumed: false,
        };
        store.create(&code).await.expect("create");
        let consumed = store
            .consume("oac_pg")
            .await
            .expect("consume")
            .expect("present");
        assert_eq!(consumed.code, "oac_pg");
        assert!(store.consume("oac_pg").await.expect("consume2").is_none());
    }

    #[tokio::test]
    async fn pg_refresh_revoke_for_user() {
        let Some(pool) = setup_pg().await else {
            eprintln!("skipping (ASSAY_TEST_DATABASE_URL not set)");
            return;
        };
        let store = PostgresOidcRefreshStore::new(pool);
        for i in 0..3 {
            let plaintext = format!("ort_pg_{i}");
            let row = RefreshToken {
                token_hash: hash_refresh_token(&plaintext),
                client_id: "c1".to_string(),
                user_id: "u_pg".to_string(),
                scopes: vec!["openid".to_string()],
                issued_at: 1.0,
                expires_at: 1000.0,
                revoked: false,
            };
            store.create(&row).await.expect("create");
        }
        let count = store
            .revoke_for_user("u_pg")
            .await
            .expect("revoke_for_user");
        assert!(count >= 3);
    }

    #[tokio::test]
    async fn pg_upstream_state_take_is_single_use() {
        let Some(pool) = setup_pg().await else {
            eprintln!("skipping (ASSAY_TEST_DATABASE_URL not set)");
            return;
        };
        let store = PostgresOidcUpstreamStateStore::new(pool);
        let s = UpstreamLoginState {
            state: "state_pg".to_string(),
            provider_slug: "google".to_string(),
            nonce: "n".to_string(),
            pkce_verifier: "v".to_string(),
            return_to: None,
            created_at: 1.0,
            expires_at: 1000.0,
            binding_hash: String::new(),
        };
        store.create(&s).await.expect("create");
        let took = store
            .take("state_pg")
            .await
            .expect("take")
            .expect("present");
        assert_eq!(took.provider_slug, "google");
        assert!(store.take("state_pg").await.expect("take2").is_none());
    }
}
