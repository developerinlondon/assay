mod common;

use common::run_lua;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn test_http_get() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/health"))
        .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local resp = http.get("{}/health")
        assert.eq(resp.status, 200)
        assert.eq(resp.body, "ok")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_http_get_with_headers() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/authed"))
        .respond_with(ResponseTemplate::new(200).set_body_string("authed"))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local resp = http.get("{}/authed", {{ headers = {{ Authorization = "Bearer tok" }} }})
        assert.eq(resp.status, 200)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_http_post_json_table() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/data"))
        .respond_with(ResponseTemplate::new(201).set_body_string(r#"{"id":1}"#))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local resp = http.post("{}/data", {{ name = "test" }})
        assert.eq(resp.status, 201)
        local body = json.parse(resp.body)
        assert.eq(body.id, 1)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_http_put() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/item/1"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local resp = http.put("{}/item/1", "updated")
        assert.eq(resp.status, 200)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_http_patch() {
    let server = MockServer::start().await;
    Mock::given(method("PATCH"))
        .and(path("/item/1"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local resp = http.patch("{}/item/1", {{ status = "done" }})
        assert.eq(resp.status, 200)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_http_delete() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/item/1"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local resp = http.delete("{}/item/1")
        assert.eq(resp.status, 204)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}
