mod common;

use common::run_lua;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn test_traefik_overview() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/overview"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "http": {
                "routers": {"total": 10, "warnings": 1, "errors": 0},
                "services": {"total": 8, "warnings": 0, "errors": 0},
                "middlewares": {"total": 5, "warnings": 0, "errors": 0}
            },
            "tcp": {
                "routers": {"total": 2, "warnings": 0, "errors": 0},
                "services": {"total": 2, "warnings": 0, "errors": 0}
            },
            "features": {
                "tracing": "",
                "metrics": "",
                "accessLog": false
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local traefik = require("assay.traefik")
        local ov = traefik.overview("{}")
        assert.eq(ov.http.routers.total, 10)
        assert.eq(ov.http.routers.warnings, 1)
        assert.eq(ov.http.services.total, 8)
        assert.eq(ov.tcp.routers.total, 2)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_traefik_version() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/version"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "Version": "3.2.0",
            "Codename": "picodon",
            "startDate": "2026-02-10T00:00:00Z"
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local traefik = require("assay.traefik")
        local ver = traefik.version("{}")
        assert.eq(ver.Version, "3.2.0")
        assert.eq(ver.Codename, "picodon")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_traefik_entrypoints() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/entrypoints"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "name": "web",
                "address": ":80",
                "transport": {"lifeCycle": {"graceTimeOut": "10s"}}
            },
            {
                "name": "websecure",
                "address": ":443",
                "transport": {"lifeCycle": {"graceTimeOut": "10s"}}
            }
        ])))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local traefik = require("assay.traefik")
        local eps = traefik.entrypoints("{}")
        assert.eq(#eps, 2)
        assert.eq(eps[1].name, "web")
        assert.eq(eps[1].address, ":80")
        assert.eq(eps[2].name, "websecure")
        assert.eq(eps[2].address, ":443")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_traefik_http_routers() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/http/routers"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "name": "dashboard@internal",
                "rule": "PathPrefix(`/api`) || PathPrefix(`/dashboard`)",
                "service": "api@internal",
                "status": "enabled",
                "entryPoints": ["traefik"]
            },
            {
                "name": "my-app@docker",
                "rule": "Host(`app.example.com`)",
                "service": "my-app@docker",
                "status": "enabled",
                "entryPoints": ["websecure"],
                "tls": {}
            }
        ])))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local traefik = require("assay.traefik")
        local routers = traefik.http_routers("{}")
        assert.eq(#routers, 2)
        assert.eq(routers[1].name, "dashboard@internal")
        assert.eq(routers[1].status, "enabled")
        assert.eq(routers[2].name, "my-app@docker")
        assert.eq(routers[2].rule, "Host(`app.example.com`)")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_traefik_http_router_single() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/http/routers/my-app@docker"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "name": "my-app@docker",
            "rule": "Host(`app.example.com`)",
            "service": "my-app@docker",
            "status": "enabled",
            "entryPoints": ["websecure"],
            "middlewares": ["auth@docker", "ratelimit@docker"],
            "tls": {"certResolver": "letsencrypt"}
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local traefik = require("assay.traefik")
        local router = traefik.http_router("{}", "my-app@docker")
        assert.eq(router.name, "my-app@docker")
        assert.eq(router.status, "enabled")
        assert.eq(router.service, "my-app@docker")
        assert.eq(#router.middlewares, 2)
        assert.eq(router.middlewares[1], "auth@docker")
        assert.eq(router.tls.certResolver, "letsencrypt")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_traefik_http_services() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/http/services"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "name": "my-app@docker",
                "type": "loadbalancer",
                "status": "enabled",
                "loadBalancer": {
                    "servers": [
                        {"url": "http://10.0.0.1:8080"},
                        {"url": "http://10.0.0.2:8080"}
                    ],
                    "passHostHeader": true
                }
            },
            {
                "name": "api@internal",
                "type": "loadbalancer",
                "status": "enabled"
            }
        ])))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local traefik = require("assay.traefik")
        local services = traefik.http_services("{}")
        assert.eq(#services, 2)
        assert.eq(services[1].name, "my-app@docker")
        assert.eq(services[1].type, "loadbalancer")
        assert.eq(#services[1].loadBalancer.servers, 2)
        assert.eq(services[1].loadBalancer.servers[1].url, "http://10.0.0.1:8080")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_traefik_http_service_single() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/http/services/my-app@docker"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "name": "my-app@docker",
            "type": "loadbalancer",
            "status": "enabled",
            "loadBalancer": {
                "servers": [
                    {"url": "http://10.0.0.1:8080"},
                    {"url": "http://10.0.0.2:8080"},
                    {"url": "http://10.0.0.3:8080"}
                ],
                "passHostHeader": true,
                "healthCheck": {
                    "path": "/health",
                    "interval": "10s",
                    "timeout": "3s"
                }
            },
            "serverStatus": {
                "http://10.0.0.1:8080": "UP",
                "http://10.0.0.2:8080": "UP",
                "http://10.0.0.3:8080": "UP"
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local traefik = require("assay.traefik")
        local svc = traefik.http_service("{}", "my-app@docker")
        assert.eq(svc.name, "my-app@docker")
        assert.eq(svc.status, "enabled")
        assert.eq(#svc.loadBalancer.servers, 3)
        assert.eq(svc.loadBalancer.healthCheck.path, "/health")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_traefik_http_middlewares() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/http/middlewares"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "name": "auth@docker",
                "type": "forwardauth",
                "status": "enabled",
                "forwardAuth": {
                    "address": "http://auth-service:4181",
                    "trustForwardHeader": true
                }
            },
            {
                "name": "ratelimit@docker",
                "type": "ratelimit",
                "status": "enabled",
                "rateLimit": {
                    "average": 100,
                    "burst": 50
                }
            }
        ])))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local traefik = require("assay.traefik")
        local mws = traefik.http_middlewares("{}")
        assert.eq(#mws, 2)
        assert.eq(mws[1].name, "auth@docker")
        assert.eq(mws[1].type, "forwardauth")
        assert.eq(mws[2].name, "ratelimit@docker")
        assert.eq(mws[2].rateLimit.average, 100)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_traefik_is_router_enabled_true() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/http/routers/my-app@docker"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "name": "my-app@docker",
            "rule": "Host(`app.example.com`)",
            "service": "my-app@docker",
            "status": "enabled"
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local traefik = require("assay.traefik")
        local enabled = traefik.is_router_enabled("{}", "my-app@docker")
        assert.eq(enabled, true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_traefik_is_router_enabled_false() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/http/routers/broken@docker"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "name": "broken@docker",
            "rule": "Host(`broken.example.com`)",
            "service": "broken@docker",
            "status": "disabled"
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local traefik = require("assay.traefik")
        local enabled = traefik.is_router_enabled("{}", "broken@docker")
        assert.eq(enabled, false)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_traefik_router_has_tls_true() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/http/routers/secure@docker"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "name": "secure@docker",
            "rule": "Host(`secure.example.com`)",
            "service": "secure@docker",
            "status": "enabled",
            "tls": {"certResolver": "letsencrypt"}
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local traefik = require("assay.traefik")
        local has_tls = traefik.router_has_tls("{}", "secure@docker")
        assert.eq(has_tls, true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_traefik_router_has_tls_false() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/http/routers/plain@docker"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "name": "plain@docker",
            "rule": "Host(`plain.example.com`)",
            "service": "plain@docker",
            "status": "enabled"
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local traefik = require("assay.traefik")
        local has_tls = traefik.router_has_tls("{}", "plain@docker")
        assert.eq(has_tls, false)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_traefik_service_server_count() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/http/services/my-app@docker"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "name": "my-app@docker",
            "type": "loadbalancer",
            "status": "enabled",
            "loadBalancer": {
                "servers": [
                    {"url": "http://10.0.0.1:8080"},
                    {"url": "http://10.0.0.2:8080"},
                    {"url": "http://10.0.0.3:8080"}
                ]
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local traefik = require("assay.traefik")
        local count = traefik.service_server_count("{}", "my-app@docker")
        assert.eq(count, 3)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_traefik_healthy_routers() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/http/routers"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {"name": "app1@docker", "rule": "Host(`a`)", "status": "enabled"},
            {"name": "app2@docker", "rule": "Host(`b`)", "status": "enabled"},
            {"name": "app3@docker", "rule": "Host(`c`)", "status": "enabled"},
            {"name": "broken@docker", "rule": "Host(`d`)", "status": "disabled"},
            {"name": "error@docker", "rule": "Host(`e`)", "status": "warning"}
        ])))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local traefik = require("assay.traefik")
        local enabled, errored = traefik.healthy_routers("{}")
        assert.eq(enabled, 3)
        assert.eq(errored, 2)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}
