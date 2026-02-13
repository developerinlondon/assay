mod common;

use common::run_lua;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn test_require_zitadel() {
    let script = r#"
        local z = require("assay.zitadel")
        assert.not_nil(z)
        assert.not_nil(z.client)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_zitadel_client_token_auth() {
    let server = MockServer::start().await;

    let script = format!(
        r#"
        local z = require("assay.zitadel")
        local c = z.client({{ url = "{}", domain = "example.com", token = "test-token" }})
        assert.not_nil(c)
        assert.eq(c.url, "{}")
        assert.eq(c.domain, "example.com")
        assert.eq(c.access_token, "test-token")
        "#,
        server.uri(),
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_zitadel_ensure_primary_domain_already_primary() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/admin/v1/orgs/me/domains"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "result": [
                { "domainName": "example.com", "isPrimary": true }
            ]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local z = require("assay.zitadel")
        local c = z.client({{ url = "{}", domain = "example.com", token = "test-token" }})
        local ok = c:ensure_primary_domain("example.com")
        assert.eq(ok, true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_zitadel_ensure_primary_domain_needs_setting() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/admin/v1/orgs/me/domains"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "result": [
                { "domainName": "old.example.com", "isPrimary": true }
            ]
        })))
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/admin/v1/orgs/me/domains"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/admin/v1/orgs/me/domains/example.com/_set_primary"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local z = require("assay.zitadel")
        local c = z.client({{ url = "{}", domain = "example.com", token = "test-token" }})
        local ok = c:ensure_primary_domain("example.com")
        assert.eq(ok, true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_zitadel_ensure_primary_domain_add_conflict() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/admin/v1/orgs/me/domains"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "result": []
        })))
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/admin/v1/orgs/me/domains"))
        .respond_with(ResponseTemplate::new(409).set_body_json(serde_json::json!({
            "message": "domain already exists"
        })))
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/admin/v1/orgs/me/domains/example.com/_set_primary"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local z = require("assay.zitadel")
        local c = z.client({{ url = "{}", domain = "example.com", token = "test-token" }})
        local ok = c:ensure_primary_domain("example.com")
        assert.eq(ok, true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_zitadel_ensure_primary_domain_list_failure() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/admin/v1/orgs/me/domains"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local z = require("assay.zitadel")
        local c = z.client({{ url = "{}", domain = "example.com", token = "test-token" }})
        local ok = c:ensure_primary_domain("example.com")
        assert.eq(ok, false)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_zitadel_find_project_found() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/management/v1/projects/_search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "result": [
                { "id": "proj-123", "name": "my-project" }
            ]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local z = require("assay.zitadel")
        local c = z.client({{ url = "{}", domain = "example.com", token = "test-token" }})
        local proj = c:find_project("my-project")
        assert.not_nil(proj)
        assert.eq(proj.id, "proj-123")
        assert.eq(proj.name, "my-project")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_zitadel_find_project_not_found() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/management/v1/projects/_search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "result": []
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local z = require("assay.zitadel")
        local c = z.client({{ url = "{}", domain = "example.com", token = "test-token" }})
        local proj = c:find_project("nonexistent")
        assert.eq(proj, nil)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_zitadel_find_project_api_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/management/v1/projects/_search"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local z = require("assay.zitadel")
        local c = z.client({{ url = "{}", domain = "example.com", token = "test-token" }})
        local proj = c:find_project("my-project")
        assert.eq(proj, nil)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_zitadel_create_project() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/management/v1/projects"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "proj-456",
            "name": "new-project"
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local z = require("assay.zitadel")
        local c = z.client({{ url = "{}", domain = "example.com", token = "test-token" }})
        local proj = c:create_project("new-project")
        assert.not_nil(proj)
        assert.eq(proj.id, "proj-456")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_zitadel_ensure_project_creates_when_not_found() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/management/v1/projects/_search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "result": []
        })))
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/management/v1/projects"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "proj-789",
            "name": "my-project"
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local z = require("assay.zitadel")
        local c = z.client({{ url = "{}", domain = "example.com", token = "test-token" }})
        local proj = c:ensure_project("my-project")
        assert.not_nil(proj)
        assert.eq(proj.id, "proj-789")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_zitadel_ensure_project_returns_existing() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/management/v1/projects/_search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "result": [
                { "id": "proj-existing", "name": "my-project" }
            ]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local z = require("assay.zitadel")
        local c = z.client({{ url = "{}", domain = "example.com", token = "test-token" }})
        local proj = c:ensure_project("my-project")
        assert.not_nil(proj)
        assert.eq(proj.id, "proj-existing")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_zitadel_find_app_found() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/management/v1/projects/proj-123/apps/_search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "result": [
                { "id": "app-1", "name": "my-app" },
                { "id": "app-2", "name": "other-app" }
            ]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local z = require("assay.zitadel")
        local c = z.client({{ url = "{}", domain = "example.com", token = "test-token" }})
        local app = c:find_app("proj-123", "my-app")
        assert.not_nil(app)
        assert.eq(app.id, "app-1")
        assert.eq(app.name, "my-app")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_zitadel_find_app_not_found() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/management/v1/projects/proj-123/apps/_search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "result": [
                { "id": "app-1", "name": "other-app" }
            ]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local z = require("assay.zitadel")
        local c = z.client({{ url = "{}", domain = "example.com", token = "test-token" }})
        local app = c:find_app("proj-123", "nonexistent")
        assert.eq(app, nil)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_zitadel_create_oidc_app() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/management/v1/projects/proj-123/apps/oidc"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "appId": "app-new",
            "clientId": "client-id-123",
            "clientSecret": "secret-456"
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local z = require("assay.zitadel")
        local c = z.client({{ url = "{}", domain = "example.com", token = "test-token" }})
        local app = c:create_oidc_app("proj-123", {{
            name = "my-oidc-app",
            subdomain = "app",
            callbackPath = "/oauth/callback",
        }})
        assert.not_nil(app)
        assert.eq(app.clientId, "client-id-123")
        assert.eq(app.clientSecret, "secret-456")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_zitadel_ensure_oidc_app_creates_when_not_found() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/management/v1/projects/proj-123/apps/_search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "result": []
        })))
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/management/v1/projects/proj-123/apps/oidc"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "appId": "app-new",
            "clientId": "client-new-123"
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local z = require("assay.zitadel")
        local c = z.client({{ url = "{}", domain = "example.com", token = "test-token" }})
        local app = c:ensure_oidc_app("proj-123", {{
            name = "my-oidc-app",
            subdomain = "app",
            callbackPath = "/oauth/callback",
        }})
        assert.not_nil(app)
        assert.eq(app.clientId, "client-new-123")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_zitadel_ensure_oidc_app_returns_existing() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/management/v1/projects/proj-123/apps/_search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "result": [
                { "id": "app-existing", "name": "my-oidc-app" }
            ]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local z = require("assay.zitadel")
        local c = z.client({{ url = "{}", domain = "example.com", token = "test-token" }})
        local app = c:ensure_oidc_app("proj-123", {{
            name = "my-oidc-app",
            subdomain = "app",
            callbackPath = "/oauth/callback",
        }})
        assert.not_nil(app)
        assert.eq(app.id, "app-existing")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_zitadel_find_idp_found() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/admin/v1/idps/templates/_search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "result": [
                { "id": "idp-google-1", "name": "Google" }
            ]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local z = require("assay.zitadel")
        local c = z.client({{ url = "{}", domain = "example.com", token = "test-token" }})
        local idp = c:find_idp("Google")
        assert.not_nil(idp)
        assert.eq(idp.id, "idp-google-1")
        assert.eq(idp.name, "Google")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_zitadel_find_idp_not_found() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/admin/v1/idps/templates/_search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "result": []
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local z = require("assay.zitadel")
        local c = z.client({{ url = "{}", domain = "example.com", token = "test-token" }})
        local idp = c:find_idp("Nonexistent")
        assert.eq(idp, nil)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_zitadel_ensure_google_idp_creates_new() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/admin/v1/idps/templates/_search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "result": []
        })))
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/admin/v1/idps/google"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "idp-google-new"
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local z = require("assay.zitadel")
        local c = z.client({{ url = "{}", domain = "example.com", token = "test-token" }})
        local idp_id = c:ensure_google_idp({{
            clientId = "google-client-id",
            clientSecret = "google-secret",
        }})
        assert.eq(idp_id, "idp-google-new")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_zitadel_ensure_google_idp_returns_existing() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/admin/v1/idps/templates/_search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "result": [
                { "id": "idp-google-existing", "name": "Google" }
            ]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local z = require("assay.zitadel")
        local c = z.client({{ url = "{}", domain = "example.com", token = "test-token" }})
        local idp_id = c:ensure_google_idp({{
            clientId = "google-client-id",
            clientSecret = "google-secret",
        }})
        assert.eq(idp_id, "idp-google-existing")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_zitadel_ensure_oidc_idp_creates_new() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/admin/v1/idps/templates/_search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "result": []
        })))
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/admin/v1/idps/generic_oidc"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "idp-oidc-new"
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local z = require("assay.zitadel")
        local c = z.client({{ url = "{}", domain = "example.com", token = "test-token" }})
        local idp_id = c:ensure_oidc_idp({{
            name = "MyOIDC",
            clientId = "oidc-client-id",
            clientSecret = "oidc-secret",
            issuer = "https://idp.example.com",
        }})
        assert.eq(idp_id, "idp-oidc-new")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_zitadel_ensure_oidc_idp_updates_existing() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/admin/v1/idps/templates/_search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "result": [
                { "id": "idp-oidc-existing", "name": "MyOIDC" }
            ]
        })))
        .mount(&server)
        .await;

    Mock::given(method("PUT"))
        .and(path("/admin/v1/idps/generic_oidc/idp-oidc-existing"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local z = require("assay.zitadel")
        local c = z.client({{ url = "{}", domain = "example.com", token = "test-token" }})
        local idp_id = c:ensure_oidc_idp({{
            name = "MyOIDC",
            clientId = "oidc-client-id",
            clientSecret = "oidc-secret",
            issuer = "https://idp.example.com",
        }})
        assert.eq(idp_id, "idp-oidc-existing")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_zitadel_add_idp_to_login_policy_success() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/admin/v1/policies/login/idps"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local z = require("assay.zitadel")
        local c = z.client({{ url = "{}", domain = "example.com", token = "test-token" }})
        local ok = c:add_idp_to_login_policy("idp-123")
        assert.eq(ok, true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_zitadel_add_idp_to_login_policy_already_exists() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/admin/v1/policies/login/idps"))
        .respond_with(ResponseTemplate::new(409).set_body_json(serde_json::json!({
            "message": "already exists"
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local z = require("assay.zitadel")
        local c = z.client({{ url = "{}", domain = "example.com", token = "test-token" }})
        local ok = c:add_idp_to_login_policy("idp-123")
        assert.eq(ok, true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_zitadel_add_idp_to_login_policy_failure() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/admin/v1/policies/login/idps"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local z = require("assay.zitadel")
        local c = z.client({{ url = "{}", domain = "example.com", token = "test-token" }})
        local ok = c:add_idp_to_login_policy("idp-123")
        assert.eq(ok, false)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_zitadel_search_users() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/management/v1/users/_search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "result": [
                { "userId": "user-1", "userName": "alice@example.com" },
                { "userId": "user-2", "userName": "bob@example.com" }
            ]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local z = require("assay.zitadel")
        local c = z.client({{ url = "{}", domain = "example.com", token = "test-token" }})
        local users = c:search_users({{ queries = {{}} }})
        assert.eq(#users, 2)
        assert.eq(users[1].userId, "user-1")
        assert.eq(users[2].userName, "bob@example.com")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_zitadel_search_users_empty() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/management/v1/users/_search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "result": []
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local z = require("assay.zitadel")
        local c = z.client({{ url = "{}", domain = "example.com", token = "test-token" }})
        local users = c:search_users({{ queries = {{}} }})
        assert.eq(#users, 0)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_zitadel_search_users_api_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/management/v1/users/_search"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local z = require("assay.zitadel")
        local c = z.client({{ url = "{}", domain = "example.com", token = "test-token" }})
        local users = c:search_users({{ queries = {{}} }})
        assert.eq(#users, 0)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_zitadel_update_user_email_success() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/management/v1/users/user-1/email"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local z = require("assay.zitadel")
        local c = z.client({{ url = "{}", domain = "example.com", token = "test-token" }})
        local ok = c:update_user_email("user-1", "newemail@example.com")
        assert.eq(ok, true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_zitadel_update_user_email_failure() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/management/v1/users/user-1/email"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local z = require("assay.zitadel")
        local c = z.client({{ url = "{}", domain = "example.com", token = "test-token" }})
        local ok = c:update_user_email("user-1", "newemail@example.com")
        assert.eq(ok, false)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_zitadel_get_login_policy() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/admin/v1/policies/login"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "policy": {
                "allowUsernamePassword": true,
                "allowExternalIdp": true,
                "allowRegister": false,
                "forceMfa": false,
                "passwordlessType": "PASSWORDLESS_TYPE_NOT_ALLOWED",
                "hidePasswordReset": false
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local z = require("assay.zitadel")
        local c = z.client({{ url = "{}", domain = "example.com", token = "test-token" }})
        local policy = c:get_login_policy()
        assert.not_nil(policy)
        assert.eq(policy.allowUsernamePassword, true)
        assert.eq(policy.allowExternalIdp, true)
        assert.eq(policy.allowRegister, false)
        assert.eq(policy.hidePasswordReset, false)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_zitadel_get_login_policy_failure() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/admin/v1/policies/login"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local z = require("assay.zitadel")
        local c = z.client({{ url = "{}", domain = "example.com", token = "test-token" }})
        local policy = c:get_login_policy()
        assert.eq(policy, nil)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_zitadel_update_login_policy_success() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/admin/v1/policies/login"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local z = require("assay.zitadel")
        local c = z.client({{ url = "{}", domain = "example.com", token = "test-token" }})
        local ok = c:update_login_policy({{
            allowUsernamePassword = false,
            allowExternalIdp = true,
        }})
        assert.eq(ok, true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_zitadel_update_login_policy_failure() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/admin/v1/policies/login"))
        .respond_with(ResponseTemplate::new(500).set_body_string("internal error"))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local z = require("assay.zitadel")
        local c = z.client({{ url = "{}", domain = "example.com", token = "test-token" }})
        local ok = c:update_login_policy({{
            allowUsernamePassword = false,
        }})
        assert.eq(ok, false)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_zitadel_disable_password_login() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/admin/v1/policies/login"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "policy": {
                "allowUsernamePassword": true,
                "allowExternalIdp": true,
                "allowRegister": false,
                "forceMfa": false,
                "passwordlessType": "PASSWORDLESS_TYPE_NOT_ALLOWED",
                "hidePasswordReset": false,
                "passwordCheckLifetime": "240h",
                "externalLoginCheckLifetime": "240h",
                "mfaInitSkipLifetime": "720h",
                "secondFactorCheckLifetime": "18h",
                "multiFactorCheckLifetime": "12h"
            }
        })))
        .mount(&server)
        .await;

    Mock::given(method("PUT"))
        .and(path("/admin/v1/policies/login"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local z = require("assay.zitadel")
        local c = z.client({{ url = "{}", domain = "example.com", token = "test-token" }})
        local ok = c:disable_password_login()
        assert.eq(ok, true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_zitadel_disable_password_login_already_disabled() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/admin/v1/policies/login"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "policy": {
                "allowUsernamePassword": false,
                "allowExternalIdp": true,
                "allowRegister": false,
                "forceMfa": false,
                "passwordlessType": "PASSWORDLESS_TYPE_NOT_ALLOWED",
                "hidePasswordReset": true
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local z = require("assay.zitadel")
        local c = z.client({{ url = "{}", domain = "example.com", token = "test-token" }})
        local ok = c:disable_password_login()
        assert.eq(ok, true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_zitadel_disable_password_login_policy_read_failure() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/admin/v1/policies/login"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local z = require("assay.zitadel")
        local c = z.client({{ url = "{}", domain = "example.com", token = "test-token" }})
        local ok = c:disable_password_login()
        assert.eq(ok, false)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}
