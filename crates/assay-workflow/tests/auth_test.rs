use assay_workflow::api::auth::{generate_api_key, hash_api_key, AuthMode};
use assay_workflow::{Engine, SqliteStore, WorkflowStore};
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

    // No token needed
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

    // Generate and store a key
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

    // Use the key
    let resp = client()
        .get(format!("{url}/api/v1/workflows"))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
}

// ── JWT mode ────────────────────────────────────────────────

fn make_jwt(claims: &serde_json::Value) -> String {
    // Sign with HS256 using a dummy key. Our current JWT validation only checks
    // claims (issuer, expiry, audience), not signature (TODO: JWKS).
    let key = jsonwebtoken::EncodingKey::from_secret(b"test-secret");
    let header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::HS256);
    jsonwebtoken::encode(&header, claims, &key).unwrap()
}

#[tokio::test]
async fn jwt_rejects_no_token() {
    let (url, _h) = start_server(AuthMode::Jwt {
        issuer: "https://auth.example.com".to_string(),
        audience: None,
    })
    .await;

    let resp = client()
        .get(format!("{url}/api/v1/workflows"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
}

#[tokio::test]
async fn jwt_accepts_valid_token() {
    let (url, _h) = start_server(AuthMode::Jwt {
        issuer: "https://auth.example.com".to_string(),
        audience: None,
    })
    .await;

    let future_exp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 3600;

    let token = make_jwt(&serde_json::json!({
        "iss": "https://auth.example.com",
        "exp": future_exp,
        "sub": "user-1",
    }));

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
    let (url, _h) = start_server(AuthMode::Jwt {
        issuer: "https://auth.example.com".to_string(),
        audience: None,
    })
    .await;

    let future_exp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 3600;

    let token = make_jwt(&serde_json::json!({
        "iss": "https://wrong-issuer.com",
        "exp": future_exp,
    }));

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
    let (url, _h) = start_server(AuthMode::Jwt {
        issuer: "https://auth.example.com".to_string(),
        audience: None,
    })
    .await;

    let token = make_jwt(&serde_json::json!({
        "iss": "https://auth.example.com",
        "exp": 1000000000,  // 2001 — long expired
    }));

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
    let (url, _h) = start_server(AuthMode::Jwt {
        issuer: "https://auth.example.com".to_string(),
        audience: Some("my-app".to_string()),
    })
    .await;

    let future_exp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 3600;

    let token = make_jwt(&serde_json::json!({
        "iss": "https://auth.example.com",
        "exp": future_exp,
        "aud": "wrong-app",
    }));

    let resp = client()
        .get(format!("{url}/api/v1/workflows"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
}
