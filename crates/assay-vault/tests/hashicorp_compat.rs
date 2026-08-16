//! HTTP-level tests for the Vault / OpenBao KV2 read facade.
//!
//! Every assertion is about the wire: an ESO `ExternalSecret` or an ansible
//! `vault_kv2_get` lookup only ever sees status codes and JSON shapes, so a
//! service-level test would not tell us whether those consumers work.

#![cfg(all(feature = "backend-sqlite", feature = "vault-hashicorp-compat"))]

use assay_vault::hashicorp_compat::{Mount, router};
use assay_vault::store::sqlite::SqliteKvStore;
use assay_vault::{KekHandle, VaultCtx};
use axum::Router;
use axum::body::Body;
use axum::extract::Request;
use axum::http::{StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Executor, SqlitePool};
use std::str::FromStr;
use tower::ServiceExt;

const TOKEN: &str = "test-admin-key";

async fn boot_pool() -> SqlitePool {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let suffix = format!(
        "{}_{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    );
    let v = format!("file:assay_vault_hc_{suffix}?mode=memory&cache=shared");
    let e = format!("file:assay_vault_hc_e_{suffix}?mode=memory&cache=shared");

    let opts = SqliteConnectOptions::from_str("sqlite::memory:")
        .unwrap()
        .create_if_missing(true);

    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .after_connect(move |conn, _| {
            let v = v.clone();
            let e = e.clone();
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
        .unwrap();
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS engine.migrations (
            module  TEXT NOT NULL,
            version INTEGER NOT NULL,
            PRIMARY KEY (module, version)
        )",
    )
    .execute(&pool)
    .await
    .unwrap();
    assay_vault::schema::migrate_sqlite(&pool).await.unwrap();
    pool
}

/// Stands in for the engine's admin-bearer gate: same contract (a bearer the
/// deployment already knows), same rejection status.
async fn require_bearer(request: Request, next: Next) -> Response {
    let presented = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok());
    if presented != Some(&format!("Bearer {TOKEN}")) {
        return (
            StatusCode::UNAUTHORIZED,
            axum::Json(json!({"error": "unauthorized"})),
        )
            .into_response();
    }
    next.run(request).await
}

async fn ctx() -> VaultCtx {
    let pool = boot_pool().await;
    let kek = KekHandle::generate_ephemeral();
    VaultCtx::new()
        .with_kek(kek)
        .with_kv(SqliteKvStore::new(pool))
}

fn app_for(ctx: VaultCtx, mount: &str) -> Router {
    router::<VaultCtx, _>(Mount::new(mount), |r| {
        r.layer(axum::middleware::from_fn(require_bearer))
    })
    .with_state(ctx)
}

async fn ctx_with_one_secret() -> VaultCtx {
    let ctx = ctx().await;
    ctx.kv
        .clone()
        .unwrap()
        .put("a/b", br#"{"k":"v"}"#, json!({}))
        .await
        .unwrap();
    ctx
}

/// Seeds the paths every read/list assertion below reads back.
async fn seeded_app() -> Router {
    let ctx = ctx().await;
    let kv = ctx.kv.clone().expect("kv wired");
    kv.put(
        "platform/postgres",
        br#"{"username":"assay","password":"hunter2"}"#,
        json!({"owner": "sre"}),
    )
    .await
    .unwrap();
    kv.put("platform/redis", b"single-value-secret", json!({}))
        .await
        .unwrap();
    kv.put("platform/immich/db", br#"{"password":"x"}"#, json!({}))
        .await
        .unwrap();
    kv.put("apps/neutron", br#"{"token":"t"}"#, json!({}))
        .await
        .unwrap();
    app_for(ctx, "secrets")
}

fn request(method: &str, uri: &str) -> Request {
    Request::builder()
        .method(method)
        .uri(uri)
        .header("X-Vault-Token", TOKEN)
        .body(Body::empty())
        .unwrap()
}

fn anonymous(method: &str, uri: &str) -> Request {
    Request::builder()
        .method(method)
        .uri(uri)
        .body(Body::empty())
        .unwrap()
}

async fn json_body(response: Response) -> Value {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

async fn assert_permission_denied(response: Response) {
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        json_body(response).await,
        json!({"errors": ["permission denied"]})
    );
}

async fn assert_vault_not_found(response: Response) {
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_eq!(json_body(response).await, json!({"errors": []}));
}

async fn assert_lists_the_platform_prefix(response: Response) {
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        json_body(response).await["data"]["keys"],
        json!(["immich/", "postgres", "redis"])
    );
}

#[tokio::test]
async fn a_json_secret_reads_back_in_the_kv2_envelope() {
    let app = seeded_app().await;
    let response = app
        .oneshot(request("GET", "/v1/secrets/data/platform/postgres"))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    assert_eq!(body["data"]["data"]["username"], "assay");
    assert_eq!(body["data"]["data"]["password"], "hunter2");
    assert_eq!(body["data"]["metadata"]["version"], 1);
    assert_eq!(body["data"]["metadata"]["destroyed"], false);
    assert_eq!(body["data"]["metadata"]["deletion_time"], "");
    assert_eq!(body["data"]["metadata"]["custom_metadata"]["owner"], "sre");
    assert!(
        body["data"]["metadata"]["created_time"]
            .as_str()
            .unwrap()
            .ends_with('Z')
    );
    for field in ["request_id", "lease_id", "renewable", "lease_duration"] {
        assert!(body.get(field).is_some(), "envelope is missing {field}");
    }
}

#[tokio::test]
async fn a_non_json_secret_stays_reachable_under_value() {
    let app = seeded_app().await;
    let response = app
        .oneshot(request("GET", "/v1/secrets/data/platform/redis"))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    assert_eq!(body["data"]["data"]["value"], "single-value-secret");
}

#[tokio::test]
async fn an_explicit_version_reads_that_version_not_the_latest() {
    let ctx = ctx().await;
    let kv = ctx.kv.clone().unwrap();
    kv.put("api/key", br#"{"k":"v1"}"#, json!({}))
        .await
        .unwrap();
    kv.put("api/key", br#"{"k":"v2"}"#, json!({}))
        .await
        .unwrap();
    let app = app_for(ctx, "secrets");

    let latest = json_body(
        app.clone()
            .oneshot(request("GET", "/v1/secrets/data/api/key"))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(latest["data"]["data"]["k"], "v2");
    assert_eq!(latest["data"]["metadata"]["version"], 2);

    let pinned = json_body(
        app.oneshot(request("GET", "/v1/secrets/data/api/key?version=1"))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(pinned["data"]["data"]["k"], "v1");
    assert_eq!(pinned["data"]["metadata"]["version"], 1);
}

#[tokio::test]
async fn a_missing_path_is_a_vault_shaped_404() {
    let app = seeded_app().await;
    let response = app
        .oneshot(request("GET", "/v1/secrets/data/nope/nothing"))
        .await
        .unwrap();

    assert_vault_not_found(response).await;
}

#[tokio::test]
async fn a_soft_deleted_version_answers_404_but_still_describes_itself() {
    let ctx = ctx().await;
    let kv = ctx.kv.clone().unwrap();
    kv.put("api/rotated", br#"{"k":"v"}"#, json!({}))
        .await
        .unwrap();
    kv.soft_delete("api/rotated", 1).await.unwrap();
    let app = app_for(ctx, "secrets");

    let response = app
        .oneshot(request("GET", "/v1/secrets/data/api/rotated"))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let body = json_body(response).await;
    assert_eq!(body["data"]["data"], Value::Null);
    assert_ne!(body["data"]["metadata"]["deletion_time"], "");
}

#[tokio::test]
async fn another_mount_name_is_not_served_by_this_engine() {
    let app = seeded_app().await;
    let response = app
        .oneshot(request("GET", "/v1/kv/data/platform/postgres"))
        .await
        .unwrap();

    assert_vault_not_found(response).await;
}

#[tokio::test]
async fn the_configured_mount_name_is_the_one_that_answers() {
    let app = app_for(ctx_with_one_secret().await, "kv");

    let response = app
        .oneshot(request("GET", "/v1/kv/data/a/b"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn a_wrong_token_is_refused_the_way_vault_refuses_one() {
    let app = seeded_app().await;
    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/secrets/data/platform/postgres")
                .header("X-Vault-Token", "not-the-key")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_permission_denied(response).await;
}

#[tokio::test]
async fn a_missing_token_is_refused_too() {
    let app = seeded_app().await;
    let response = app
        .oneshot(anonymous("GET", "/v1/secrets/data/platform/postgres"))
        .await
        .unwrap();

    assert_permission_denied(response).await;
}

#[tokio::test]
async fn an_authorization_bearer_works_without_the_vault_header() {
    let app = seeded_app().await;
    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/secrets/data/platform/postgres")
                .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn list_folds_deeper_keys_into_directory_entries() {
    let app = seeded_app().await;
    let response = app
        .oneshot(request("LIST", "/v1/secrets/metadata/platform"))
        .await
        .unwrap();

    assert_lists_the_platform_prefix(response).await;
}

#[tokio::test]
async fn list_via_the_query_flag_matches_the_list_method() {
    let app = seeded_app().await;
    let response = app
        .oneshot(request("GET", "/v1/secrets/metadata/platform?list=true"))
        .await
        .unwrap();

    assert_lists_the_platform_prefix(response).await;
}

#[tokio::test]
async fn list_at_the_mount_root_reports_top_level_directories() {
    let app = seeded_app().await;
    let response = app
        .oneshot(request("LIST", "/v1/secrets/metadata"))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    assert_eq!(body["data"]["keys"], json!(["apps/", "platform/"]));
}

#[tokio::test]
async fn a_trailing_slash_on_a_list_prefix_lists_the_same_keys() {
    let app = seeded_app().await;
    let response = app
        .oneshot(request("LIST", "/v1/secrets/metadata/platform/"))
        .await
        .unwrap();

    assert_lists_the_platform_prefix(response).await;
}

#[tokio::test]
async fn listing_a_prefix_with_no_keys_is_a_404() {
    let app = seeded_app().await;
    let response = app
        .oneshot(request("LIST", "/v1/secrets/metadata/nothing-here"))
        .await
        .unwrap();

    assert_vault_not_found(response).await;
}

#[tokio::test]
async fn metadata_reports_the_current_version_and_custom_metadata() {
    let app = seeded_app().await;
    let response = app
        .oneshot(request("GET", "/v1/secrets/metadata/platform/postgres"))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    assert_eq!(body["data"]["current_version"], 1);
    assert_eq!(body["data"]["custom_metadata"]["owner"], "sre");
    assert!(body["data"]["versions"]["1"]["created_time"].is_string());
    assert_eq!(body["data"]["versions"]["1"]["destroyed"], false);
}

#[tokio::test]
async fn metadata_for_a_missing_path_is_a_404() {
    let app = seeded_app().await;
    let response = app
        .oneshot(request("GET", "/v1/secrets/metadata/nope"))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn health_answers_without_a_token() {
    let app = seeded_app().await;
    let response = app
        .oneshot(anonymous("GET", "/v1/sys/health"))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    assert_eq!(body["initialized"], true);
    assert_eq!(body["sealed"], false);
    assert!(body["server_time_utc"].is_number());
}

#[tokio::test]
async fn health_reports_a_sealed_engine_as_unavailable() {
    let ctx = ctx().await;
    ctx.seal_state.seal().unwrap();
    let app = app_for(ctx, "secrets");

    let response = app
        .oneshot(anonymous("GET", "/v1/sys/health"))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(json_body(response).await["sealed"], true);
}

#[tokio::test]
async fn a_sealed_engine_refuses_reads_rather_than_serving_stale_plaintext() {
    let ctx = ctx_with_one_secret().await;
    ctx.seal_state.seal().unwrap();
    let app = app_for(ctx, "secrets");

    let response = app
        .oneshot(request("GET", "/v1/secrets/data/a/b"))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(json_body(response).await["errors"][0], "Vault is sealed");
}

#[tokio::test]
async fn every_write_verb_is_refused_on_the_read_facade() {
    let app = seeded_app().await;
    for method in ["PUT", "POST", "DELETE", "PATCH"] {
        let response = app
            .clone()
            .oneshot(request(method, "/v1/secrets/data/platform/postgres"))
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::METHOD_NOT_ALLOWED,
            "{method} on a data path"
        );
        assert_eq!(
            json_body(response).await,
            json!({"errors": ["unsupported operation"]}),
            "{method} on a data path"
        );
    }
}

#[tokio::test]
async fn writes_to_the_metadata_path_are_refused_as_well() {
    let app = seeded_app().await;
    for method in ["PUT", "POST", "DELETE"] {
        let response = app
            .clone()
            .oneshot(request(method, "/v1/secrets/metadata/platform/postgres"))
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::METHOD_NOT_ALLOWED,
            "{method} on a metadata path"
        );
    }
}

#[tokio::test]
async fn the_facade_serves_no_sys_route_beyond_health() {
    let app = seeded_app().await;
    for path in [
        "/v1/sys/seal-status",
        "/v1/sys/mounts",
        "/v1/auth/token/create",
    ] {
        let response = app.clone().oneshot(request("GET", path)).await.unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{path}");
    }
}
