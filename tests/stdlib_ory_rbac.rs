mod common;

use common::run_lua;
use wiremock::matchers::{body_string_contains, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

// Helper that produces a Lua snippet which builds a standard test policy
// against a provided keto read URL and (optionally) a write URL. Keeps
// the per-test setup compact.
fn build_policy_snippet(read_url: &str, write_url: Option<&str>) -> String {
    let write_part = match write_url {
        Some(w) => format!(", write_url = \"{w}\""),
        None => String::new(),
    };
    format!(
        r#"
        local rbac = require("assay.ory.rbac")
        local keto = require("assay.ory.keto")
        local k = keto.client("{read_url}", {{ {write_part} }})
        local p = rbac.policy({{
          namespace = "demo-app",
          keto = k,
          default_role = "viewer",
          roles = {{
            owner    = {{ rank = 5, capabilities = {{"read","trigger","approve","configure","manage_roles"}} }},
            admin    = {{ rank = 4, capabilities = {{"read","trigger","approve","configure"}} }},
            approver = {{ rank = 3, capabilities = {{"read","approve"}} }},
            operator = {{ rank = 2, capabilities = {{"read","trigger"}} }},
            viewer   = {{ rank = 1, capabilities = {{"read"}} }},
          }},
        }})
        "#,
        read_url = read_url,
        write_part = write_part.trim_start_matches(", "),
    )
}

// Mock the standard Keto list endpoint with the given relation tuples.
async fn mock_user_role_list(server: &MockServer, subject: &str, tuples: serde_json::Value) {
    Mock::given(method("GET"))
        .and(path("/relation-tuples"))
        .and(query_param("namespace", "Role"))
        .and(query_param("relation", "members"))
        .and(query_param("subject_id", subject))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "relation_tuples": tuples,
            "next_page_token": "",
        })))
        .mount(server)
        .await;
}

#[tokio::test]
async fn test_rbac_require() {
    let script = r#"
        local rbac = require("assay.ory.rbac")
        assert.not_nil(rbac)
        assert.not_nil(rbac.policy)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_rbac_user_with_no_roles_gets_default() {
    let server = MockServer::start().await;
    mock_user_role_list(&server, "user:alice", serde_json::json!([])).await;

    let script = format!(
        r#"
        {policy}
        local roles = p.users:roles("alice")
        assert.eq(#roles, 0)
        assert.eq(p.users:primary_role("alice"), "viewer")
        local caps = p.users:capabilities("alice")
        assert.eq(caps.read, true)
        assert.eq(caps.trigger, nil)
        assert.eq(caps.approve, nil)
        "#,
        policy = build_policy_snippet(&server.uri(), None)
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_rbac_user_with_single_role() {
    let server = MockServer::start().await;
    mock_user_role_list(
        &server,
        "user:bob",
        serde_json::json!([
            {
                "namespace": "Role",
                "object": "demo-app:operator",
                "relation": "members",
                "subject_id": "user:bob"
            }
        ]),
    )
    .await;

    let script = format!(
        r#"
        {policy}
        local roles = p.users:roles("bob")
        assert.eq(#roles, 1)
        assert.eq(roles[1], "operator")
        assert.eq(p.users:primary_role("bob"), "operator")
        local caps = p.users:capabilities("bob")
        assert.eq(caps.read, true)
        assert.eq(caps.trigger, true)
        assert.eq(caps.approve, nil)
        assert.eq(p.users:has_capability("bob", "trigger"), true)
        assert.eq(p.users:has_capability("bob", "approve"), false)
        "#,
        policy = build_policy_snippet(&server.uri(), None)
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_rbac_user_with_multiple_roles_unions_capabilities() {
    let server = MockServer::start().await;
    // Carol is both an approver AND an operator — so the union of her
    // capabilities includes BOTH trigger (from operator) and approve
    // (from approver), even though neither role grants both alone.
    mock_user_role_list(
        &server,
        "user:carol",
        serde_json::json!([
            { "namespace": "Role", "object": "demo-app:approver", "relation": "members", "subject_id": "user:carol" },
            { "namespace": "Role", "object": "demo-app:operator", "relation": "members", "subject_id": "user:carol" }
        ]),
    )
    .await;

    let script = format!(
        r#"
        {policy}
        local roles = p.users:roles("carol")
        assert.eq(#roles, 2)
        -- highest rank first
        assert.eq(roles[1], "approver")
        assert.eq(roles[2], "operator")
        assert.eq(p.users:primary_role("carol"), "approver")
        local caps = p.users:capabilities("carol")
        assert.eq(caps.read, true)
        assert.eq(caps.trigger, true)
        assert.eq(caps.approve, true)
        assert.eq(caps.configure, nil)
        "#,
        policy = build_policy_snippet(&server.uri(), None)
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_rbac_ignores_unknown_roles() {
    // If Keto returns a tuple for a role the policy doesn't define
    // (e.g. left over from an old deployment) we silently ignore it.
    let server = MockServer::start().await;
    mock_user_role_list(
        &server,
        "user:dave",
        serde_json::json!([
            { "namespace": "Role", "object": "demo-app:legacy-role", "relation": "members", "subject_id": "user:dave" },
            { "namespace": "Role", "object": "demo-app:viewer",      "relation": "members", "subject_id": "user:dave" }
        ]),
    )
    .await;

    let script = format!(
        r#"
        {policy}
        local roles = p.users:roles("dave")
        assert.eq(#roles, 1)
        assert.eq(roles[1], "viewer")
        "#,
        policy = build_policy_snippet(&server.uri(), None)
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_rbac_ignores_other_namespaces() {
    // Tuples from a different app (e.g. platform:admin) should not
    // count as demo-app roles.
    let server = MockServer::start().await;
    mock_user_role_list(
        &server,
        "user:eve",
        serde_json::json!([
            { "namespace": "Role", "object": "platform:admin",   "relation": "members", "subject_id": "user:eve" },
            { "namespace": "Role", "object": "demo-app:operator", "relation": "members", "subject_id": "user:eve" }
        ]),
    )
    .await;

    let script = format!(
        r#"
        {policy}
        local roles = p.users:roles("eve")
        assert.eq(#roles, 1)
        assert.eq(roles[1], "operator")
        "#,
        policy = build_policy_snippet(&server.uri(), None)
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_rbac_members_list() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/relation-tuples"))
        .and(query_param("namespace", "Role"))
        .and(query_param("object", "demo-app:admin"))
        .and(query_param("relation", "members"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "relation_tuples": [
                { "namespace": "Role", "object": "demo-app:admin", "relation": "members", "subject_id": "user:alice" },
                { "namespace": "Role", "object": "demo-app:admin", "relation": "members", "subject_id": "user:bob" }
            ],
            "next_page_token": ""
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        {policy}
        local members = p.members:list("admin")
        assert.eq(#members, 2)
        assert.eq(members[1], "alice")
        assert.eq(members[2], "bob")
        "#,
        policy = build_policy_snippet(&server.uri(), None)
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_rbac_members_add() {
    let read = MockServer::start().await;
    let write = MockServer::start().await;

    // Empty initial list so the idempotency check sees no existing member.
    Mock::given(method("GET"))
        .and(path("/relation-tuples"))
        .and(query_param("object", "demo-app:approver"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "relation_tuples": [], "next_page_token": ""
        })))
        .mount(&read)
        .await;

    Mock::given(method("PUT"))
        .and(path("/admin/relation-tuples"))
        .and(body_string_contains("demo-app:approver"))
        .and(body_string_contains("user:seth"))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({})))
        .mount(&write)
        .await;

    let script = format!(
        r#"
        {policy}
        p.members:add("seth", "approver")
        "#,
        policy = build_policy_snippet(&read.uri(), Some(&write.uri()))
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_rbac_members_add_is_idempotent() {
    // The list call returns Seth already, so add() should NOT issue a
    // PUT — wiremock will fail the test if it does because we don't
    // mount a PUT mock.
    let read = MockServer::start().await;
    let write = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/relation-tuples"))
        .and(query_param("object", "demo-app:approver"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "relation_tuples": [
                { "namespace": "Role", "object": "demo-app:approver", "relation": "members", "subject_id": "user:seth" }
            ],
            "next_page_token": ""
        })))
        .mount(&read)
        .await;

    let script = format!(
        r#"
        {policy}
        p.members:add("seth", "approver")
        "#,
        policy = build_policy_snippet(&read.uri(), Some(&write.uri()))
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_rbac_members_remove() {
    let read = MockServer::start().await;
    let write = MockServer::start().await;

    Mock::given(method("DELETE"))
        .and(path("/admin/relation-tuples"))
        .and(query_param("object", "demo-app:operator"))
        .and(query_param("subject_id", "user:bob"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&write)
        .await;

    let script = format!(
        r#"
        {policy}
        p.members:remove("bob", "operator")
        "#,
        policy = build_policy_snippet(&read.uri(), Some(&write.uri()))
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_rbac_members_reset() {
    let read = MockServer::start().await;
    let write = MockServer::start().await;

    Mock::given(method("DELETE"))
        .and(path("/admin/relation-tuples"))
        .and(query_param("namespace", "Role"))
        .and(query_param("object", "demo-app:owner"))
        .and(query_param("relation", "members"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&write)
        .await;

    let script = format!(
        r#"
        {policy}
        p.members:reset("owner")
        "#,
        policy = build_policy_snippet(&read.uri(), Some(&write.uri()))
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_rbac_unknown_role_errors() {
    let server = MockServer::start().await;
    let script = format!(
        r#"
        {policy}
        local ok, err = pcall(function() p.members:list("not-a-real-role") end)
        assert.eq(ok, false)
        assert.contains(tostring(err), "unknown role")
        "#,
        policy = build_policy_snippet(&server.uri(), None)
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_rbac_policy_roles_returned_in_rank_order() {
    let server = MockServer::start().await;
    let script = format!(
        r#"
        {policy}
        local rs = p.policy:roles()
        assert.eq(#rs, 5)
        assert.eq(rs[1], "owner")
        assert.eq(rs[2], "admin")
        assert.eq(rs[3], "approver")
        assert.eq(rs[4], "operator")
        assert.eq(rs[5], "viewer")
        "#,
        policy = build_policy_snippet(&server.uri(), None)
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_rbac_policy_role_metadata_lookup() {
    let server = MockServer::start().await;
    let script = format!(
        r#"
        {policy}
        local r = p.policy:get("approver")
        assert.eq(r.rank, 3)
        assert.eq(#r.capabilities, 2)
        -- capabilities are returned sorted alphabetically
        assert.eq(r.capabilities[1], "approve")
        assert.eq(r.capabilities[2], "read")
        "#,
        policy = build_policy_snippet(&server.uri(), None)
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_rbac_policy_validation() {
    // Missing namespace
    let script = r#"
        local rbac = require("assay.ory.rbac")
        local ok, err = pcall(function()
          rbac.policy({ keto = {}, roles = { viewer = { rank = 1, capabilities = {"read"} } } })
        end)
        assert.eq(ok, false)
        assert.contains(tostring(err), "namespace is required")
    "#;
    run_lua(script).await.unwrap();

    // Missing keto
    let script = r#"
        local rbac = require("assay.ory.rbac")
        local ok, err = pcall(function()
          rbac.policy({ namespace = "x", roles = { viewer = { rank = 1, capabilities = {"read"} } } })
        end)
        assert.eq(ok, false)
        assert.contains(tostring(err), "keto client is required")
    "#;
    run_lua(script).await.unwrap();

    // Empty roles
    let script = r#"
        local rbac = require("assay.ory.rbac")
        local ok, err = pcall(function()
          rbac.policy({ namespace = "x", keto = {}, roles = {} })
        end)
        assert.eq(ok, false)
        assert.contains(tostring(err), "roles map is required")
    "#;
    run_lua(script).await.unwrap();
}
