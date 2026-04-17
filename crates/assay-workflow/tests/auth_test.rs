use assay_workflow::api::auth::{generate_api_key, hash_api_key, AuthMode, JwksCache, JwtConfig};
use assay_workflow::{Engine, SqliteStore, WorkflowStore};
use jsonwebtoken::jwk::{
    CommonParameters, Jwk, JwkSet, KeyAlgorithm, PublicKeyUse, RSAKeyParameters, RSAKeyType,
};
use std::sync::Arc;
use tokio::sync::broadcast;

async fn start_server(auth_mode: AuthMode) -> (String, tokio::task::JoinHandle<()>) {
    let store = SqliteStore::new("sqlite::memory:").await.unwrap();
    start_server_with_store(store, auth_mode).await
}

async fn start_server_with_store(
    store: SqliteStore,
    auth_mode: AuthMode,
) -> (String, tokio::task::JoinHandle<()>) {
    let engine = Engine::start(store);
    let (event_tx, _) = broadcast::channel(64);
    let state = Arc::new(assay_workflow::api::AppState {
        engine: Arc::new(engine),
        event_tx,
        auth_mode,
        binary_version: None,
    });
    let app = assay_workflow::api::router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let base_url = format!("http://127.0.0.1:{port}");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    (base_url, handle)
}

fn client() -> reqwest::Client {
    reqwest::Client::new()
}

// ── RSA key helpers for JWT tests ───────────────────────────

struct TestKeys {
    encoding_key: jsonwebtoken::EncodingKey,
    jwk_set: JwkSet,
}

fn generate_test_rsa_keys() -> TestKeys {
    use rsa::pkcs1::EncodeRsaPrivateKey;
    use rsa::traits::PublicKeyParts;

    // Use rsa's own OsRng to avoid rand version conflicts
    let mut rng = rsa::rand_core::OsRng;
    let private_key = rsa::RsaPrivateKey::new(&mut rng, 2048).unwrap();
    let public_key = private_key.to_public_key();

    // Create encoding key from PEM
    let pem = private_key.to_pkcs1_pem(rsa::pkcs1::LineEnding::LF).unwrap();
    let encoding_key = jsonwebtoken::EncodingKey::from_rsa_pem(pem.as_bytes()).unwrap();

    // Create JWK from public key components
    let n_bytes = public_key.n().to_bytes_be();
    let e_bytes = public_key.e().to_bytes_be();
    let n = data_encoding::BASE64URL_NOPAD.encode(&n_bytes);
    let e = data_encoding::BASE64URL_NOPAD.encode(&e_bytes);

    let jwk = Jwk {
        common: CommonParameters {
            public_key_use: Some(PublicKeyUse::Signature),
            key_id: Some("test-key-1".to_string()),
            key_algorithm: Some(KeyAlgorithm::RS256),
            ..Default::default()
        },
        algorithm: jsonwebtoken::jwk::AlgorithmParameters::RSA(RSAKeyParameters {
            key_type: RSAKeyType::RSA,
            n,
            e,
        }),
    };

    TestKeys {
        encoding_key,
        jwk_set: JwkSet { keys: vec![jwk] },
    }
}

fn sign_jwt(keys: &TestKeys, claims: &serde_json::Value) -> String {
    let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256);
    header.kid = Some("test-key-1".to_string());
    jsonwebtoken::encode(&header, claims, &keys.encoding_key).unwrap()
}

fn jwt_config(keys: &TestKeys, audience: Option<&str>) -> JwtConfig {
    let cache = JwksCache::with_jwks(
        "https://auth.example.com".to_string(),
        keys.jwk_set.clone(),
    );
    JwtConfig {
        issuer: "https://auth.example.com".to_string(),
        audience: audience.map(|s| s.to_string()),
        jwks_cache: Arc::new(cache),
    }
}

fn jwt_auth_mode(keys: &TestKeys) -> AuthMode {
    AuthMode {
        api_key: false,
        jwt: Some(jwt_config(keys, None)),
    }
}

fn jwt_auth_mode_with_audience(keys: &TestKeys, audience: &str) -> AuthMode {
    AuthMode {
        api_key: false,
        jwt: Some(jwt_config(keys, Some(audience))),
    }
}

fn combined_auth_mode(keys: &TestKeys) -> AuthMode {
    AuthMode {
        api_key: true,
        jwt: Some(jwt_config(keys, None)),
    }
}

fn future_exp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 3600
}

// ── No Auth mode ────────────────────────────────────────────

#[tokio::test]
async fn no_auth_allows_all_requests() {
    let (url, _h) = start_server(AuthMode::no_auth()).await;

    let resp = client()
        .get(format!("{url}/api/v1/health"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    let resp = client()
        .get(format!("{url}/api/v1/workflows"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
}

// ── API Key mode ────────────────────────────────────────────

#[tokio::test]
async fn api_key_rejects_no_token() {
    let (url, _h) = start_server(AuthMode::api_key()).await;

    let resp = client()
        .get(format!("{url}/api/v1/workflows"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
}

#[tokio::test]
async fn api_key_rejects_invalid_key() {
    let (url, _h) = start_server(AuthMode::api_key()).await;

    let resp = client()
        .get(format!("{url}/api/v1/workflows"))
        .header("Authorization", "Bearer assay_invalid_key_12345678")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
}

#[tokio::test]
async fn api_key_accepts_valid_key() {
    let store = SqliteStore::new("sqlite::memory:").await.unwrap();

    let key = generate_api_key();
    let hash = hash_api_key(&key);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64();
    store
        .create_api_key(&hash, "assay_test...", None, now)
        .await
        .unwrap();

    let (url, _h) = start_server_with_store(store, AuthMode::api_key()).await;

    let resp = client()
        .get(format!("{url}/api/v1/workflows"))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
}

// ── JWT mode (RSA signature validation) ─────────────────────

#[tokio::test]
async fn jwt_rejects_no_token() {
    let keys = generate_test_rsa_keys();
    let (url, _h) = start_server(jwt_auth_mode(&keys)).await;

    let resp = client()
        .get(format!("{url}/api/v1/workflows"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
}

#[tokio::test]
async fn jwt_accepts_valid_rsa_token() {
    let keys = generate_test_rsa_keys();
    let (url, _h) = start_server(jwt_auth_mode(&keys)).await;

    let token = sign_jwt(
        &keys,
        &serde_json::json!({
            "iss": "https://auth.example.com",
            "exp": future_exp(),
            "sub": "user-1",
        }),
    );

    let resp = client()
        .get(format!("{url}/api/v1/workflows"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
}

#[tokio::test]
async fn jwt_rejects_wrong_issuer() {
    let keys = generate_test_rsa_keys();
    let (url, _h) = start_server(jwt_auth_mode(&keys)).await;

    let token = sign_jwt(
        &keys,
        &serde_json::json!({
            "iss": "https://wrong-issuer.com",
            "exp": future_exp(),
        }),
    );

    let resp = client()
        .get(format!("{url}/api/v1/workflows"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
}

#[tokio::test]
async fn jwt_rejects_expired_token() {
    let keys = generate_test_rsa_keys();
    let (url, _h) = start_server(jwt_auth_mode(&keys)).await;

    let token = sign_jwt(
        &keys,
        &serde_json::json!({
            "iss": "https://auth.example.com",
            "exp": 1_000_000_000u64,  // 2001 — long expired
        }),
    );

    let resp = client()
        .get(format!("{url}/api/v1/workflows"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
}

#[tokio::test]
async fn jwt_rejects_wrong_audience() {
    let keys = generate_test_rsa_keys();
    let (url, _h) = start_server(jwt_auth_mode_with_audience(&keys, "my-app")).await;

    let token = sign_jwt(
        &keys,
        &serde_json::json!({
            "iss": "https://auth.example.com",
            "exp": future_exp(),
            "aud": "wrong-app",
        }),
    );

    let resp = client()
        .get(format!("{url}/api/v1/workflows"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
}

#[tokio::test]
async fn jwt_rejects_tampered_signature() {
    let keys = generate_test_rsa_keys();
    let (url, _h) = start_server(jwt_auth_mode(&keys)).await;

    // Sign with the correct key
    let token = sign_jwt(
        &keys,
        &serde_json::json!({
            "iss": "https://auth.example.com",
            "exp": future_exp(),
            "sub": "user-1",
        }),
    );

    // Tamper with the signature (flip some bytes at the end)
    let mut tampered = token.into_bytes();
    let len = tampered.len();
    tampered[len - 1] ^= 0xFF;
    tampered[len - 2] ^= 0xFF;
    let tampered = String::from_utf8_lossy(&tampered).to_string();

    let resp = client()
        .get(format!("{url}/api/v1/workflows"))
        .header("Authorization", format!("Bearer {tampered}"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
}

#[tokio::test]
async fn jwt_rejects_different_rsa_key() {
    let keys = generate_test_rsa_keys();
    let wrong_keys = generate_test_rsa_keys(); // Different keypair

    // Server expects keys.jwk_set, but we sign with wrong_keys
    let (url, _h) = start_server(jwt_auth_mode(&keys)).await;

    let token = sign_jwt(
        &wrong_keys,
        &serde_json::json!({
            "iss": "https://auth.example.com",
            "exp": future_exp(),
            "sub": "user-1",
        }),
    );

    let resp = client()
        .get(format!("{url}/api/v1/workflows"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
}

// ── Combined mode (JWT + API key on the same server) ─────────

#[tokio::test]
async fn combined_accepts_valid_jwt() {
    let keys = generate_test_rsa_keys();
    let (url, _h) = start_server(combined_auth_mode(&keys)).await;

    let token = sign_jwt(
        &keys,
        &serde_json::json!({
            "iss": "https://auth.example.com",
            "exp": future_exp(),
            "sub": "user-1",
        }),
    );

    let resp = client()
        .get(format!("{url}/api/v1/workflows"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
}

#[tokio::test]
async fn combined_accepts_valid_api_key() {
    let keys = generate_test_rsa_keys();
    let store = SqliteStore::new("sqlite::memory:").await.unwrap();

    let api_key = generate_api_key();
    let hash = hash_api_key(&api_key);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64();
    store
        .create_api_key(&hash, "assay_test...", None, now)
        .await
        .unwrap();

    let (url, _h) = start_server_with_store(store, combined_auth_mode(&keys)).await;

    let resp = client()
        .get(format!("{url}/api/v1/workflows"))
        .header("Authorization", format!("Bearer {api_key}"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
}

#[tokio::test]
async fn combined_rejects_missing_token() {
    let keys = generate_test_rsa_keys();
    let (url, _h) = start_server(combined_auth_mode(&keys)).await;

    let resp = client()
        .get(format!("{url}/api/v1/workflows"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
}

#[tokio::test]
async fn combined_rejects_garbage_token() {
    let keys = generate_test_rsa_keys();
    let (url, _h) = start_server(combined_auth_mode(&keys)).await;

    // Not JWT-shaped, not a stored API key — should fail the API-key lookup.
    let resp = client()
        .get(format!("{url}/api/v1/workflows"))
        .header("Authorization", "Bearer assay_not_a_real_key_abcd")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
}

#[tokio::test]
async fn combined_rejects_expired_jwt_without_api_key_fallback() {
    // An expired JWT is JWT-shaped, so the middleware takes the JWT path
    // and rejects it there — it must NOT silently be retried as an API key.
    let keys = generate_test_rsa_keys();
    let (url, _h) = start_server(combined_auth_mode(&keys)).await;

    let token = sign_jwt(
        &keys,
        &serde_json::json!({
            "iss": "https://auth.example.com",
            "exp": 1_000_000_000u64, // 2001 — long expired
        }),
    );

    let resp = client()
        .get(format!("{url}/api/v1/workflows"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
}

// ── AuthMode helpers ─────────────────────────────────────────

#[test]
fn describe_no_auth() {
    assert_eq!(AuthMode::no_auth().describe(), "no-auth (open access)");
}

#[test]
fn describe_api_key_only() {
    assert_eq!(AuthMode::api_key().describe(), "api-key");
}

#[test]
fn describe_jwt_only() {
    let m = AuthMode::jwt("https://auth.example.com".to_string(), None);
    assert_eq!(m.describe(), "jwt (issuer: https://auth.example.com)");
}

#[test]
fn describe_combined() {
    let m = AuthMode::combined("https://auth.example.com".to_string(), None);
    assert_eq!(
        m.describe(),
        "jwt (issuer: https://auth.example.com) + api-key"
    );
}

#[test]
fn is_enabled_reflects_state() {
    assert!(!AuthMode::no_auth().is_enabled());
    assert!(AuthMode::api_key().is_enabled());
    assert!(AuthMode::jwt("https://auth.example.com".to_string(), None).is_enabled());
    assert!(AuthMode::combined("https://auth.example.com".to_string(), None).is_enabled());
}

// ── POST /api/v1/api-keys — bootstrap window + idempotent mode ────

#[tokio::test]
async fn bootstrap_post_api_keys_allowed_without_auth_when_empty() {
    // Fresh store, api_keys table is empty. In api-key auth mode, the only
    // way to mint the first key is unauthenticated POST /api/v1/api-keys.
    let (url, _h) = start_server(AuthMode::api_key()).await;

    let resp = client()
        .post(format!("{url}/api/v1/api-keys"))
        .json(&serde_json::json!({ "label": "first-ever", "idempotent": true }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201, "bootstrap window should allow unauth");

    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(
        body.get("plaintext").and_then(|v| v.as_str()).is_some(),
        "fresh mint must include plaintext"
    );
    assert_eq!(body["label"], "first-ever");
    assert!(body["prefix"].as_str().unwrap().starts_with("assay_"));
}

#[tokio::test]
async fn bootstrap_closes_after_first_key() {
    // First call closes the window; second unauth call must be rejected.
    let (url, _h) = start_server(AuthMode::api_key()).await;

    let first = client()
        .post(format!("{url}/api/v1/api-keys"))
        .json(&serde_json::json!({ "label": "k1" }))
        .send()
        .await
        .unwrap();
    assert_eq!(first.status(), 201);

    let second = client()
        .post(format!("{url}/api/v1/api-keys"))
        .json(&serde_json::json!({ "label": "k2" }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        second.status(),
        401,
        "bootstrap window closes once any key exists"
    );
}

#[tokio::test]
async fn bootstrap_only_on_post_api_keys_path() {
    // Other endpoints don't get the bootstrap pass — empty api_keys table
    // doesn't imply free access to everything.
    let (url, _h) = start_server(AuthMode::api_key()).await;

    let resp = client()
        .get(format!("{url}/api/v1/workflows"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        401,
        "bootstrap window only covers POST /api/v1/api-keys"
    );
}

#[tokio::test]
async fn api_keys_idempotent_returns_existing_without_plaintext() {
    // Bootstrap the first key; then call again with the same label and
    // idempotent=true. The server should return 200 with metadata but
    // NO plaintext (the plaintext was issued once and can't be re-emitted).
    let (url, _h) = start_server(AuthMode::api_key()).await;

    let first = client()
        .post(format!("{url}/api/v1/api-keys"))
        .json(&serde_json::json!({ "label": "cc_api_key", "idempotent": true }))
        .send()
        .await
        .unwrap();
    assert_eq!(first.status(), 201);
    let first_body: serde_json::Value = first.json().await.unwrap();
    let key = first_body["plaintext"].as_str().unwrap().to_string();

    // Second call with idempotent=true — authenticated via the key we just received.
    let second = client()
        .post(format!("{url}/api/v1/api-keys"))
        .header("Authorization", format!("Bearer {key}"))
        .json(&serde_json::json!({ "label": "cc_api_key", "idempotent": true }))
        .send()
        .await
        .unwrap();
    assert_eq!(second.status(), 200);
    let second_body: serde_json::Value = second.json().await.unwrap();
    assert!(
        second_body.get("plaintext").is_none()
            || second_body["plaintext"].is_null(),
        "idempotent hit must omit plaintext"
    );
    assert_eq!(second_body["label"], "cc_api_key");
    assert_eq!(second_body["prefix"], first_body["prefix"]);
}

#[tokio::test]
async fn api_keys_non_idempotent_mints_another_with_same_label() {
    // Without idempotent=true (default), the server mints a NEW key even when
    // another key with the same label exists. Labels aren't unique in the
    // engine; idempotency is opt-in.
    let (url, _h) = start_server(AuthMode::api_key()).await;

    let first = client()
        .post(format!("{url}/api/v1/api-keys"))
        .json(&serde_json::json!({ "label": "shared-label" }))
        .send()
        .await
        .unwrap();
    assert_eq!(first.status(), 201);
    let first_body: serde_json::Value = first.json().await.unwrap();
    let key = first_body["plaintext"].as_str().unwrap().to_string();

    let second = client()
        .post(format!("{url}/api/v1/api-keys"))
        .header("Authorization", format!("Bearer {key}"))
        .json(&serde_json::json!({ "label": "shared-label" }))
        .send()
        .await
        .unwrap();
    assert_eq!(second.status(), 201);
    let second_body: serde_json::Value = second.json().await.unwrap();
    assert!(second_body["plaintext"].as_str().is_some());
    assert_ne!(
        first_body["prefix"], second_body["prefix"],
        "non-idempotent must mint a fresh key, different prefix"
    );
}

// ── Public endpoints — unauth even when auth is enabled ─────

#[tokio::test]
async fn health_is_unauth_in_api_key_mode() {
    // /api/v1/health must always be reachable without auth so k8s kubelet
    // probes, load balancers, and third-party monitors don't need tokens.
    let (url, _h) = start_server(AuthMode::api_key()).await;

    let resp = client()
        .get(format!("{url}/api/v1/health"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["status"], "ok");
}

#[tokio::test]
async fn health_is_unauth_in_jwt_mode() {
    let keys = generate_test_rsa_keys();
    let (url, _h) = start_server(jwt_auth_mode(&keys)).await;

    let resp = client()
        .get(format!("{url}/api/v1/health"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
}

#[tokio::test]
async fn health_is_unauth_in_combined_mode() {
    let keys = generate_test_rsa_keys();
    let (url, _h) = start_server(combined_auth_mode(&keys)).await;

    let resp = client()
        .get(format!("{url}/api/v1/health"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
}

#[tokio::test]
async fn version_is_unauth_in_api_key_mode() {
    // /api/v1/version — used by the CLI and dashboard to identify the
    // running build. Always unauth for the same reasons as /health.
    let (url, _h) = start_server(AuthMode::api_key()).await;

    let resp = client()
        .get(format!("{url}/api/v1/version"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body.get("version").is_some());
    assert!(body.get("build_profile").is_some());
}

#[tokio::test]
async fn other_api_v1_paths_still_require_auth_when_auth_enabled() {
    // Regression: carving out /health + /version must not accidentally
    // open up the rest of /api/v1/*. /api/v1/workflows is auth-gated.
    let (url, _h) = start_server(AuthMode::api_key()).await;

    let resp = client()
        .get(format!("{url}/api/v1/workflows"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
}

#[tokio::test]
async fn bootstrap_window_closed_in_no_auth_mode_is_a_noop() {
    // no-auth mode: the whole API is open, so the bootstrap gate is irrelevant.
    // This test just confirms we don't trip over ourselves in that configuration.
    let (url, _h) = start_server(AuthMode::no_auth()).await;

    let resp = client()
        .post(format!("{url}/api/v1/api-keys"))
        .json(&serde_json::json!({ "label": "anything" }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);
}
