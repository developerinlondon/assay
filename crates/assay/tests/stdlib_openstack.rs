mod common;

use assay::lua::{ApprovalConfig, ExecMode, VmOptions};
use common::run_lua;
use wiremock::matchers::{body_json, header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn http_client() -> reqwest::Client {
    reqwest::Client::builder().build().unwrap()
}

fn vm(mode: ExecMode) -> mlua::Lua {
    assay::lua::create_vm_with_options(
        http_client(),
        VmOptions {
            global_modules_path: None,
            mode,
            approval: ApprovalConfig::default(),
        },
    )
    .unwrap()
}

fn catalog(base_url: &str) -> serde_json::Value {
    serde_json::json!({
        "token": {
            "project": {"id": "project-1", "name": "demo-project"},
            "catalog": [
                {
                    "type": "identity",
                    "endpoints": [{
                        "interface": "public",
                        "region": "RegionOne",
                        "region_id": "RegionOne",
                        "url": format!("{base_url}/v3")
                    }]
                },
                {
                    "type": "compute",
                    "endpoints": [
                        {
                            "interface": "internal",
                            "region": "RegionOne",
                            "region_id": "RegionOne",
                            "url": format!("{base_url}/internal-compute")
                        },
                        {
                            "interface": "public",
                            "region": "RegionTwo",
                            "region_id": "RegionTwo",
                            "url": format!("{base_url}/region-two-compute")
                        },
                        {
                            "interface": "public",
                            "region": "RegionOne",
                            "region_id": "RegionOne",
                            "url": format!("{base_url}/compute/v2.1/project-1")
                        }
                    ]
                },
                {
                    "type": "image",
                    "endpoints": [{
                        "interface": "public",
                        "region": "RegionOne",
                        "region_id": "RegionOne",
                        "url": format!("{base_url}/image/v2")
                    }]
                },
                {
                    "type": "network",
                    "endpoints": [{
                        "interface": "public",
                        "region": "RegionOne",
                        "region_id": "RegionOne",
                        "url": format!("{base_url}/network")
                    }]
                }
            ]
        }
    })
}

async fn mount_password_auth(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/v3/auth/tokens"))
        .and(body_json(serde_json::json!({
            "auth": {
                "identity": {
                    "methods": ["password"],
                    "password": {
                        "user": {
                            "name": "reader",
                            "password": "secret",
                            "domain": {"name": "Users"}
                        }
                    }
                },
                "scope": {
                    "project": {
                        "name": "demo-project",
                        "domain": {"name": "Projects"}
                    }
                }
            }
        })))
        .respond_with(
            ResponseTemplate::new(201)
                .insert_header("x-subject-token", "token-from-keystone")
                .set_body_json(catalog(&server.uri())),
        )
        .mount(server)
        .await;
}

#[tokio::test]
async fn require_openstack_exposes_read_only_service_groups() {
    let script = r#"
        local openstack = require("assay.openstack")
        assert.not_nil(openstack.client)

        local c = openstack.client("https://identity.example.com/v3", {
          token = "token",
          endpoints = {
            compute = "https://compute.example.com",
            image = "https://image.example.com",
            network = "https://network.example.com",
          },
        })
        assert.not_nil(c.identity)
        assert.not_nil(c.compute)
        assert.not_nil(c.image)
        assert.not_nil(c.network)
        assert.eq(c.compute.create_server, nil)
        assert.eq(c.network.create_network, nil)
        assert.eq(c.image.delete_image, nil)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn password_auth_discovers_the_selected_compute_endpoint() {
    let server = MockServer::start().await;
    mount_password_auth(&server).await;
    Mock::given(method("GET"))
        .and(path("/compute/v2.1/project-1/servers/detail"))
        .and(query_param("all_tenants", "true"))
        .and(query_param("name", "app one"))
        .and(header("x-auth-token", "token-from-keystone"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "servers": [{"id": "server-1", "name": "app one", "status": "ACTIVE"}]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local openstack = require("assay.openstack")
        local c = openstack.client("{}/v3", {{
          username = "reader",
          password = "secret",
          project_name = "demo-project",
          user_domain_name = "Users",
          project_domain_name = "Projects",
          region = "RegionOne",
          interface = "public",
        }})
        local servers = c.compute:list_servers({{ all_tenants = true, name = "app one" }})
        assert.eq(#servers, 1)
        assert.eq(servers[1].id, "server-1")
        assert.eq(servers[1].status, "ACTIVE")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn existing_token_and_endpoint_override_skip_authentication() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/custom-compute/servers/detail"))
        .and(header("x-auth-token", "existing-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "servers": [{"id": "server-2"}]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local openstack = require("assay.openstack")
        local c = openstack.client("{}/v3", {{
          token = "existing-token",
          endpoints = {{ compute = "{}/custom-compute/" }},
        }})
        local servers = c.compute:list_servers()
        assert.eq(servers[1].id, "server-2")
        "#,
        server.uri(),
        server.uri()
    );
    run_lua(&script).await.unwrap();
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn identity_methods_list_and_get_projects_users_and_regions() {
    let server = MockServer::start().await;
    for (resource, body) in [
        (
            "projects",
            serde_json::json!({"projects": [{"id": "project-1", "name": "demo-project"}]}),
        ),
        (
            "users",
            serde_json::json!({"users": [{"id": "user-1", "name": "reader"}]}),
        ),
        (
            "regions",
            serde_json::json!({"regions": [{"id": "RegionOne"}]}),
        ),
    ] {
        Mock::given(method("GET"))
            .and(path(format!("/v3/{resource}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&server)
            .await;
    }
    Mock::given(method("GET"))
        .and(path("/v3/projects/project-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "project": {"id": "project-1", "name": "demo-project"}
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v3/users/user-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "user": {"id": "user-1", "name": "reader"}
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local openstack = require("assay.openstack")
        local c = openstack.client("{}/v3", {{ token = "token" }})
        assert.eq(c.identity:list_projects()[1].id, "project-1")
        assert.eq(c.identity:get_project("project-1").name, "demo-project")
        assert.eq(c.identity:list_users()[1].id, "user-1")
        assert.eq(c.identity:get_user("user-1").name, "reader")
        assert.eq(c.identity:list_regions()[1].id, "RegionOne")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn compute_methods_return_server_limits_and_quota_details() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/compute/servers/server-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "server": {"id": "server-1", "status": "ACTIVE"}
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/compute/limits"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "limits": {"absolute": {"maxTotalInstances": 20}}
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/compute/os-quota-sets/project-1/detail"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "quota_set": {"id": "project-1", "instances": {"limit": 20, "in_use": 3}}
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local openstack = require("assay.openstack")
        local c = openstack.client("{}/v3", {{
          token = "token",
          endpoints = {{ compute = "{}/compute" }},
        }})
        assert.eq(c.compute:get_server("server-1").status, "ACTIVE")
        assert.eq(c.compute:get_limits().absolute.maxTotalInstances, 20)
        assert.eq(c.compute:get_quota("project-1").instances.in_use, 3)
        "#,
        server.uri(),
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn image_methods_list_get_and_return_nil_on_not_found() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/image/v2/images"))
        .and(query_param("status", "active"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "images": [{"id": "image-1", "name": "base-image", "status": "active"}]
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/image/v2/images/image-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "image-1", "name": "base-image", "status": "active"
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/image/v2/images/missing"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local openstack = require("assay.openstack")
        local c = openstack.client("{}/v3", {{
          token = "token",
          endpoints = {{ image = "{}/image/v2" }},
        }})
        assert.eq(c.image:list_images({{ status = "active" }})[1].id, "image-1")
        assert.eq(c.image:get_image("image-1").name, "base-image")
        assert.eq(c.image:get_image("missing"), nil)
        "#,
        server.uri(),
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn network_methods_cover_inventory_and_project_quotas() {
    let server = MockServer::start().await;
    for (resource, response_key) in [
        ("networks", "networks"),
        ("subnets", "subnets"),
        ("ports", "ports"),
        ("routers", "routers"),
        ("security-groups", "security_groups"),
    ] {
        Mock::given(method("GET"))
            .and(path(format!("/network/v2.0/{resource}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                response_key: [{"id": format!("{resource}-1")}]
            })))
            .mount(&server)
            .await;
        let singular = resource.trim_end_matches('s');
        Mock::given(method("GET"))
            .and(path(format!("/network/v2.0/{resource}/{singular}-1")))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                singular.replace('-', "_"): {"id": format!("{singular}-1")}
            })))
            .mount(&server)
            .await;
    }
    Mock::given(method("GET"))
        .and(path("/network/v2.0/quotas/project-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "quota": {"network": 10, "port": 50}
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local openstack = require("assay.openstack")
        local c = openstack.client("{}/v3", {{
          token = "token",
          endpoints = {{ network = "{}/network" }},
        }})
        assert.eq(c.network:list_networks()[1].id, "networks-1")
        assert.eq(c.network:get_network("network-1").id, "network-1")
        assert.eq(c.network:list_subnets()[1].id, "subnets-1")
        assert.eq(c.network:get_subnet("subnet-1").id, "subnet-1")
        assert.eq(c.network:list_ports()[1].id, "ports-1")
        assert.eq(c.network:get_port("port-1").id, "port-1")
        assert.eq(c.network:list_routers()[1].id, "routers-1")
        assert.eq(c.network:get_router("router-1").id, "router-1")
        assert.eq(c.network:list_security_groups()[1].id, "security-groups-1")
        assert.eq(c.network:get_security_group("security-group-1").id, "security-group-1")
        assert.eq(c.network:get_quota("project-1").port, 50)
        "#,
        server.uri(),
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn service_errors_include_method_path_status_and_body() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/compute/servers/detail"))
        .respond_with(ResponseTemplate::new(503).set_body_string("compute unavailable"))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local openstack = require("assay.openstack")
        local c = openstack.client("{}/v3", {{
          token = "token",
          endpoints = {{ compute = "{}/compute" }},
        }})
        local ok, err = pcall(function() c.compute:list_servers() end)
        assert.eq(ok, false)
        assert.contains(tostring(err), "openstack: GET /servers/detail HTTP 503: compute unavailable")
        "#,
        server.uri(),
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn missing_credentials_and_service_endpoints_fail_loud() {
    let script = r#"
        local openstack = require("assay.openstack")
        local missing_auth = openstack.client("https://identity.example.com/v3")
        local ok_auth, err_auth = pcall(function() missing_auth:authenticate() end)
        assert.eq(ok_auth, false)
        assert.contains(tostring(err_auth), "openstack: username and password are required")

        local missing_endpoint = openstack.client("https://identity.example.com/v3", {
          token = "token",
        })
        local ok_endpoint, err_endpoint = pcall(function()
          missing_endpoint.compute:list_servers()
        end)
        assert.eq(ok_endpoint, false)
        assert.contains(tostring(err_endpoint), "openstack: no endpoint for compute")
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn token_inventory_runs_in_readonly_mode() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/compute/servers/detail"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "servers": [{"id": "server-readonly"}]
        })))
        .mount(&server)
        .await;

    let lua = vm(ExecMode::ReadOnly);
    let script = format!(
        r#"
        local openstack = require("assay.openstack")
        local c = openstack.client("{}/v3", {{
          token = "token",
          endpoints = {{ compute = "{}/compute" }},
        }})
        assert.eq(c.compute:list_servers()[1].id, "server-readonly")
        "#,
        server.uri(),
        server.uri()
    );
    lua.load(&script).exec_async().await.unwrap();
}

#[tokio::test]
async fn password_authentication_suspends_in_approval_mode() {
    let lua = vm(ExecMode::Approval);
    let err = lua
        .load(
            r#"
            local openstack = require("assay.openstack")
            local c = openstack.client("https://identity.example.com/v3", {
              username = "reader",
              password = "secret",
              project_name = "demo-project",
            })
            c:authenticate()
            "#,
        )
        .exec_async()
        .await
        .unwrap_err();
    let message = err.to_string();
    assert!(message.contains("__assay_approval_request__"));
    assert!(message.contains("http.post"));
}
