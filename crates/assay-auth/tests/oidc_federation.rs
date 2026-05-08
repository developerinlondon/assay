//! Integration tests for the provider-agnostic federation pack:
//! per-IdP scopes/auth_params, RFC 9207 `iss` callback verification,
//! cookie-bound CSRF binding, and discovery-time issuer hardening.
//!
//! Backend: SQLite in-memory only — Postgres-equivalent coverage rides
//! on the round-trip already in `oidc_provider.rs`. The hardening pack
//! is logically backend-agnostic; SQLite is enough to exercise the
//! store + handler glue.

#![cfg(all(feature = "backend-sqlite", feature = "auth-oidc-provider"))]

use std::collections::BTreeMap;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use assay_auth::oidc::{DEFAULT_UPSTREAM_SCOPES, OidcRegistry};
use assay_auth::oidc_provider::auth_params;
use assay_auth::oidc_provider::binding;
use assay_auth::oidc_provider::federation::{
    UPSTREAM_STATE_LIFETIME_SECS, complete_upstream_login,
};
use assay_auth::oidc_provider::issuer_validation::validate_issuer;
use assay_auth::oidc_provider::store::{
    OidcUpstreamStateStore, OidcUpstreamStore, SqliteOidcUpstreamStateStore,
    SqliteOidcUpstreamStore,
};
use assay_auth::oidc_provider::types::{UpstreamLoginState, UpstreamProvider};
use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

static SEQ: AtomicU64 = AtomicU64::new(0);

async fn setup_sqlite() -> SqlitePool {
    let suffix = format!(
        "{}_{}_{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed),
        now_secs() as u64
    );
    let engine_uri = format!("file:fed_eng_{suffix}?mode=memory&cache=shared");
    let auth_uri = format!("file:fed_auth_{suffix}?mode=memory&cache=shared");
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
        .expect("migrate sqlite v5");
    pool
}

fn now_secs() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

// =====================================================================
//   auth_params — admin-write whitelist
// =====================================================================

#[test]
fn auth_params_validate_accepts_whitelisted() {
    for k in auth_params::ALLOWED_KEYS {
        assert!(auth_params::validate_pair(k, "v").is_ok(), "{k}");
    }
}

#[test]
fn auth_params_validate_accepts_idp_prefix() {
    assert!(auth_params::validate_pair("idp_resource", "v").is_ok());
}

#[test]
fn auth_params_validate_rejects_framework_owned() {
    for k in auth_params::REJECTED_KEYS {
        assert!(
            matches!(
                auth_params::validate_pair(k, "v"),
                Err(auth_params::AuthParamError::RejectedKey(_))
            ),
            "{k} must be rejected"
        );
    }
}

#[test]
fn auth_params_validate_rejects_oversize_value() {
    let big = "x".repeat(auth_params::MAX_VALUE_LEN + 1);
    let err = auth_params::validate_pair("prompt", &big).unwrap_err();
    assert!(matches!(err, auth_params::AuthParamError::ValueTooLong(_)));
}

// =====================================================================
//   issuer_validation — admin-time and boot-time
// =====================================================================

#[test]
fn issuer_validation_accepts_https_public() {
    assert!(validate_issuer("https://accounts.google.com", false).is_ok());
}

#[test]
fn issuer_validation_rejects_http_without_flag() {
    assert!(validate_issuer("http://idp.example.com", false).is_err());
}

#[test]
fn issuer_validation_rejects_userinfo() {
    assert!(validate_issuer("https://user:pass@idp.example.com", false).is_err());
}

#[test]
fn issuer_validation_rejects_fragment() {
    assert!(validate_issuer("https://idp.example.com/#x", false).is_err());
}

#[test]
fn issuer_validation_rejects_private_v4() {
    for ip in [
        "https://192.168.1.1",
        "https://10.0.0.1",
        "https://127.0.0.1",
    ] {
        assert!(validate_issuer(ip, false).is_err(), "{ip}");
    }
}

#[test]
fn issuer_validation_allow_insecure_passes_localhost() {
    assert!(validate_issuer("http://localhost:8080", false).is_ok());
    assert!(validate_issuer("http://192.168.1.1", true).is_ok());
}

// =====================================================================
//   binding — random + verify
// =====================================================================

#[test]
fn binding_generate_pairs_differ() {
    let (raw1, hash1) = binding::generate();
    let (raw2, hash2) = binding::generate();
    assert_ne!(raw1, raw2);
    assert_ne!(hash1, hash2);
}

#[test]
fn binding_verify_matches_pair() {
    let (raw, hash) = binding::generate();
    assert!(binding::verify(&raw, &hash));
    assert!(!binding::verify("not_the_token", &hash));
}

// =====================================================================
//   federation::complete — three negatives + sentinel skip
// =====================================================================

#[tokio::test]
async fn complete_rejects_when_binding_missing() {
    let pool = setup_sqlite().await;
    let store: Arc<dyn OidcUpstreamStateStore> = Arc::new(SqliteOidcUpstreamStateStore::new(pool));
    let (_raw, hash) = binding::generate();
    let now = now_secs();
    store
        .create(&UpstreamLoginState {
            state: "s_missing".into(),
            provider_slug: "google".into(),
            nonce: "n".into(),
            pkce_verifier: "v".into(),
            return_to: None,
            created_at: now,
            expires_at: now + UPSTREAM_STATE_LIFETIME_SECS,
            binding_hash: hash,
        })
        .await
        .unwrap();
    let registry = OidcRegistry::new();
    let result = complete_upstream_login(&registry, &store, "code", "s_missing", None, None).await;
    let msg = format!("{:?}", result.unwrap_err());
    assert!(msg.contains("binding missing"), "got {msg}");
}

#[tokio::test]
async fn complete_rejects_when_binding_mismatched() {
    let pool = setup_sqlite().await;
    let store: Arc<dyn OidcUpstreamStateStore> = Arc::new(SqliteOidcUpstreamStateStore::new(pool));
    let (_raw, hash) = binding::generate();
    let now = now_secs();
    store
        .create(&UpstreamLoginState {
            state: "s_bad".into(),
            provider_slug: "google".into(),
            nonce: "n".into(),
            pkce_verifier: "v".into(),
            return_to: None,
            created_at: now,
            expires_at: now + UPSTREAM_STATE_LIFETIME_SECS,
            binding_hash: hash,
        })
        .await
        .unwrap();
    let registry = OidcRegistry::new();
    let result = complete_upstream_login(
        &registry,
        &store,
        "code",
        "s_bad",
        Some("wrong_token"),
        None,
    )
    .await;
    let msg = format!("{:?}", result.unwrap_err());
    assert!(msg.contains("binding mismatch"), "got {msg}");
}

#[tokio::test]
async fn complete_skips_check_when_binding_hash_empty() {
    let pool = setup_sqlite().await;
    let store: Arc<dyn OidcUpstreamStateStore> = Arc::new(SqliteOidcUpstreamStateStore::new(pool));
    let now = now_secs();
    store
        .create(&UpstreamLoginState {
            state: "s_sentinel".into(),
            provider_slug: "ghost".into(),
            nonce: "n".into(),
            pkce_verifier: "v".into(),
            return_to: None,
            created_at: now,
            expires_at: now + UPSTREAM_STATE_LIFETIME_SECS,
            binding_hash: String::new(),
        })
        .await
        .unwrap();
    let registry = OidcRegistry::new();
    let result = complete_upstream_login(&registry, &store, "code", "s_sentinel", None, None).await;
    let msg = format!("{:?}", result.unwrap_err());
    // Reaches the registry-lookup branch, proving the binding step
    // was skipped on the empty-hash sentinel.
    assert!(msg.contains("unknown upstream provider"), "got {msg}");
}

// =====================================================================
//   store round-trip — V5 columns persist
// =====================================================================

#[tokio::test]
async fn upstream_round_trip_persists_scopes_and_auth_params() {
    let pool = setup_sqlite().await;
    let store = SqliteOidcUpstreamStore::new(pool);
    let mut params = BTreeMap::new();
    params.insert("prompt".into(), "consent".into());
    params.insert("hd".into(), "example.com".into());
    let row = UpstreamProvider {
        slug: "google".into(),
        issuer: "https://accounts.google.com".into(),
        client_id: "ci".into(),
        client_secret: "cs".into(),
        display_name: "Google".into(),
        icon_url: None,
        enabled: true,
        scopes: vec!["openid".into(), "email".into(), "profile".into()],
        auth_params: params.clone(),
    };
    store.upsert(&row).await.unwrap();
    let loaded = store.get("google").await.unwrap().unwrap();
    assert_eq!(loaded.scopes, row.scopes);
    assert_eq!(loaded.auth_params, params);
}

#[tokio::test]
async fn upstream_state_round_trip_persists_binding_hash() {
    let pool = setup_sqlite().await;
    let store = SqliteOidcUpstreamStateStore::new(pool);
    let now = now_secs();
    let s = UpstreamLoginState {
        state: "s1".into(),
        provider_slug: "google".into(),
        nonce: "n".into(),
        pkce_verifier: "v".into(),
        return_to: None,
        created_at: now,
        expires_at: now + 300.0,
        binding_hash: "deadbeef".into(),
    };
    store.create(&s).await.unwrap();
    let loaded = store.take("s1").await.unwrap().unwrap();
    assert_eq!(loaded.binding_hash, "deadbeef");
}

// =====================================================================
//   wiremock-backed discovery — boot hydration + fail-soft
// =====================================================================

fn discovery_doc(issuer: &str, jwks_uri: &str) -> serde_json::Value {
    serde_json::json!({
        "issuer": issuer,
        "authorization_endpoint": format!("{issuer}/authorize"),
        "token_endpoint": format!("{issuer}/token"),
        "userinfo_endpoint": format!("{issuer}/userinfo"),
        "jwks_uri": jwks_uri,
        "response_types_supported": ["code"],
        "subject_types_supported": ["public"],
        "id_token_signing_alg_values_supported": ["RS256"],
        "scopes_supported": ["openid", "email", "profile"],
        "token_endpoint_auth_methods_supported": ["client_secret_basic"],
        "claims_supported": ["sub", "iss", "name", "email"],
        "code_challenge_methods_supported": ["S256"],
    })
}

fn empty_jwks() -> serde_json::Value {
    serde_json::json!({"keys": []})
}

async fn mount_discovery(server: &MockServer, issuer: &str) {
    let jwks_uri = format!("{}/.well-known/jwks.json", server.uri());
    Mock::given(method("GET"))
        .and(path("/.well-known/openid-configuration"))
        .respond_with(ResponseTemplate::new(200).set_body_json(discovery_doc(issuer, &jwks_uri)))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/.well-known/jwks.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(empty_jwks()))
        .mount(server)
        .await;
}

#[tokio::test]
async fn boot_hydration_loads_only_enabled_rows() {
    let pool = setup_sqlite().await;
    let upstream_store = SqliteOidcUpstreamStore::new(pool);

    let server = MockServer::start().await;
    mount_discovery(&server, &server.uri()).await;

    upstream_store
        .upsert(&UpstreamProvider {
            slug: "good_a".into(),
            issuer: server.uri(),
            client_id: "ci".into(),
            client_secret: "cs".into(),
            display_name: "GoodA".into(),
            icon_url: None,
            enabled: true,
            scopes: DEFAULT_UPSTREAM_SCOPES
                .iter()
                .map(|s| s.to_string())
                .collect(),
            auth_params: BTreeMap::new(),
        })
        .await
        .unwrap();
    upstream_store
        .upsert(&UpstreamProvider {
            slug: "good_b".into(),
            issuer: server.uri(),
            client_id: "ci".into(),
            client_secret: "cs".into(),
            display_name: "GoodB".into(),
            icon_url: None,
            enabled: true,
            scopes: vec!["openid".into()],
            auth_params: BTreeMap::new(),
        })
        .await
        .unwrap();
    upstream_store
        .upsert(&UpstreamProvider {
            slug: "off".into(),
            issuer: server.uri(),
            client_id: "ci".into(),
            client_secret: "cs".into(),
            display_name: "Off".into(),
            icon_url: None,
            enabled: false,
            scopes: vec![],
            auth_params: BTreeMap::new(),
        })
        .await
        .unwrap();

    let registry = OidcRegistry::new();
    let public_url = url::Url::parse("https://app.example.com/").unwrap();
    let rows = upstream_store.list().await.unwrap();
    for row in rows {
        assay_auth::oidc_provider::sync_upstream_to_registry(&registry, &row, &public_url).await;
    }
    let mut slugs = registry.slugs();
    slugs.sort();
    assert_eq!(slugs, vec!["good_a".to_string(), "good_b".to_string()]);
}

#[tokio::test]
async fn boot_hydration_is_fail_soft_when_discovery_fails() {
    let pool = setup_sqlite().await;
    let upstream_store = SqliteOidcUpstreamStore::new(pool);

    let working_server = MockServer::start().await;
    mount_discovery(&working_server, &working_server.uri()).await;

    let broken_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/.well-known/openid-configuration"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&broken_server)
        .await;

    upstream_store
        .upsert(&UpstreamProvider {
            slug: "broken".into(),
            issuer: broken_server.uri(),
            client_id: "ci".into(),
            client_secret: "cs".into(),
            display_name: "Broken".into(),
            icon_url: None,
            enabled: true,
            scopes: vec![],
            auth_params: BTreeMap::new(),
        })
        .await
        .unwrap();
    upstream_store
        .upsert(&UpstreamProvider {
            slug: "ok".into(),
            issuer: working_server.uri(),
            client_id: "ci".into(),
            client_secret: "cs".into(),
            display_name: "Ok".into(),
            icon_url: None,
            enabled: true,
            scopes: vec![],
            auth_params: BTreeMap::new(),
        })
        .await
        .unwrap();

    let registry = OidcRegistry::new();
    let public_url = url::Url::parse("https://app.example.com/").unwrap();
    let rows = upstream_store.list().await.unwrap();
    for row in rows {
        assay_auth::oidc_provider::sync_upstream_to_registry(&registry, &row, &public_url).await;
    }
    // Working provider lands; broken one warns + skips.
    assert!(registry.client("ok").is_some());
    assert!(registry.client("broken").is_none());
}

#[tokio::test]
async fn complete_rejects_iss_mismatch() {
    // Register a discovered provider, then call complete with an iss
    // value that doesn't match — exercises the RFC 9207 reject path
    // before the upstream code-exchange runs.
    let server = MockServer::start().await;
    mount_discovery(&server, &server.uri()).await;

    let registry = OidcRegistry::new();
    let provider = assay_auth::oidc::UpstreamProvider {
        slug: "google".into(),
        issuer: server.uri(),
        client_id: "ci".into(),
        client_secret: "cs".into(),
        scopes: vec!["openid".into()],
        auth_params: BTreeMap::new(),
    };
    let redirect =
        url::Url::parse("https://app.example.com/oidc/upstream/google/callback").unwrap();
    registry.add(provider, redirect).await.unwrap();

    let pool = setup_sqlite().await;
    let store: Arc<dyn OidcUpstreamStateStore> = Arc::new(SqliteOidcUpstreamStateStore::new(pool));
    let (raw, hash) = binding::generate();
    let now = now_secs();
    store
        .create(&UpstreamLoginState {
            state: "s_iss".into(),
            provider_slug: "google".into(),
            nonce: "n".into(),
            pkce_verifier: "v".into(),
            return_to: None,
            created_at: now,
            expires_at: now + UPSTREAM_STATE_LIFETIME_SECS,
            binding_hash: hash,
        })
        .await
        .unwrap();

    let result = complete_upstream_login(
        &registry,
        &store,
        "code",
        "s_iss",
        Some(&raw),
        Some("https://wrong.example.com"),
    )
    .await;
    let msg = format!("{:?}", result.unwrap_err());
    assert!(msg.contains("issuer mismatch"), "got {msg}");
}

#[tokio::test]
async fn auth_params_round_trip_through_authorize_url() {
    // End-to-end: row carries auth_params → registry add → start_login
    // → URL contains the params. Wiremock-backed discovery so the
    // openidconnect client actually constructs an authorize URL.
    let server = MockServer::start().await;
    mount_discovery(&server, &server.uri()).await;

    let registry = OidcRegistry::new();
    let mut params = BTreeMap::new();
    params.insert("prompt".to_string(), "consent".to_string());
    params.insert("hd".to_string(), "example.com".to_string());
    let provider = assay_auth::oidc::UpstreamProvider {
        slug: "google".into(),
        issuer: server.uri(),
        client_id: "ci".into(),
        client_secret: "cs".into(),
        scopes: vec!["openid".into(), "email".into()],
        auth_params: params,
    };
    let redirect =
        url::Url::parse("https://app.example.com/oidc/upstream/google/callback").unwrap();
    registry.add(provider, redirect).await.expect("discover");

    let client = registry.client("google").expect("registered");
    let started = client.start_login(openidconnect::CsrfToken::new_random());
    let url_str = started.url.to_string();
    assert!(url_str.contains("prompt=consent"), "got {url_str}");
    assert!(url_str.contains("hd=example.com"), "got {url_str}");
    assert!(url_str.contains("scope="), "got {url_str}");
}

#[tokio::test]
async fn registry_add_twice_replaces_inner_client() {
    // Calling add() for the same slug must rotate the cached client —
    // covers the upsert-with-changed-issuer / rotated-client-id path.
    let first = MockServer::start().await;
    let second = MockServer::start().await;
    mount_discovery(&first, &first.uri()).await;
    mount_discovery(&second, &second.uri()).await;

    let registry = OidcRegistry::new();
    let redirect =
        url::Url::parse("https://app.example.com/oidc/upstream/google/callback").unwrap();

    let p1 = assay_auth::oidc::UpstreamProvider {
        slug: "google".into(),
        issuer: first.uri(),
        client_id: "first".into(),
        client_secret: "cs".into(),
        scopes: vec!["openid".into()],
        auth_params: BTreeMap::new(),
    };
    registry.add(p1, redirect.clone()).await.unwrap();
    assert_eq!(
        registry.client("google").unwrap().provider().issuer,
        first.uri()
    );
    assert_eq!(
        registry.client("google").unwrap().provider().client_id,
        "first"
    );

    let p2 = assay_auth::oidc::UpstreamProvider {
        slug: "google".into(),
        issuer: second.uri(),
        client_id: "second".into(),
        client_secret: "cs".into(),
        scopes: vec!["openid".into()],
        auth_params: BTreeMap::new(),
    };
    registry.add(p2, redirect).await.unwrap();
    assert_eq!(
        registry.client("google").unwrap().provider().issuer,
        second.uri()
    );
    assert_eq!(
        registry.client("google").unwrap().provider().client_id,
        "second"
    );
    assert_eq!(registry.len(), 1);
}
