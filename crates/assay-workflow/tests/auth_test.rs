use assay_workflow::api::auth::{generate_api_key, hash_api_key, AuthMode, JwksCache};
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

fn jwt_auth_mode(keys: &TestKeys) -> AuthMode {
    let cache = JwksCache::with_jwks(
        "https://auth.example.com".to_string(),
        keys.jwk_set.clone(),
    );
    AuthMode::Jwt {
        issuer: "https://auth.example.com".to_string(),
        audience: None,
        jwks_cache: Arc::new(cache),
    }
}

fn jwt_auth_mode_with_audience(keys: &TestKeys, audience: &str) -> AuthMode {
    let cache = JwksCache::with_jwks(
        "https://auth.example.com".to_string(),
        keys.jwk_set.clone(),
    );
    AuthMode::Jwt {
        issuer: "https://auth.example.com".to_string(),
        audience: Some(audience.to_string()),
        jwks_cache: Arc::new(cache),
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
    let (url, _h) = start_server(AuthMode::NoAuth).await;

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
    let (url, _h) = start_server(AuthMode::ApiKey).await;

    let resp = client()
        .get(format!("{url}/api/v1/workflows"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
}

#[tokio::test]
async fn api_key_rejects_invalid_key() {
    let (url, _h) = start_server(AuthMode::ApiKey).await;

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

    let (url, _h) = start_server_with_store(store, AuthMode::ApiKey).await;

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
