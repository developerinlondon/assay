mod common;

use common::run_lua;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// One test, not several: `env.set` writes the process environment, so two
/// tests mutating `NEUTRON_EXTRA_HEADERS` in the same binary would race each
/// other. Sequential steps inside one test cannot.
#[tokio::test]
async fn test_extra_headers_from_env() {
    let server = MockServer::start().await;
    // Answers only when both internal-principal headers ride along with the bearer.
    Mock::given(method("GET"))
        .and(path("/api/admin/crm/overview"))
        .and(header("x-neutron-internal", "1"))
        .and(header("x-neutron-token", "int_tok"))
        .and(header("authorization", "Bearer nck_test"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"ok": true})))
        .mount(&server)
        .await;
    // Answers on the bearer alone, so a request whose Authorization was
    // replaced would not reach it.
    Mock::given(method("GET"))
        .and(path("/api/admin/crm/agent"))
        .and(header("authorization", "Bearer nck_test"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"ok": true})))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local neutron = require("assay.neutron")
        local url = "{}"

        -- the JSON object is read when opts.headers is absent
        env.set("NEUTRON_EXTRA_HEADERS",
            '{{"x-neutron-internal":"1","x-neutron-token":"int_tok"}}')
        local c = neutron.client(url, {{ token = "nck_test" }})
        assert.eq(c.crm:overview().ok, true)

        -- opts.headers wins over the env when both are set
        env.set("NEUTRON_EXTRA_HEADERS", '{{"x-neutron-internal":"wrong"}}')
        local explicit = neutron.client(url, {{
            token = "nck_test",
            headers = {{ ["x-neutron-internal"] = "1", ["x-neutron-token"] = "int_tok" }},
        }})
        assert.eq(explicit.crm:overview().ok, true)

        -- the env cannot replace the bearer either
        env.set("NEUTRON_EXTRA_HEADERS", '{{"Authorization":"Bearer stolen"}}')
        local guarded = neutron.client(url, {{ token = "nck_test" }})
        assert.eq(guarded.crm:agent().ok, true)

        -- the guard is case-insensitive: HTTP header names are, so a lowercase
        -- key must not slip a second Authorization past an exact-match check
        env.set("NEUTRON_EXTRA_HEADERS", '{{"authorization":"Bearer stolen"}}')
        local lower = neutron.client(url, {{ token = "nck_test" }})
        assert.eq(lower.crm:agent().ok, true)

        -- malformed JSON is an error, never a silently unauthenticated call
        env.set("NEUTRON_EXTRA_HEADERS", "not json at all")
        local ok, err = pcall(function() return neutron.client(url, {{ token = "nck_test" }}) end)
        assert.eq(ok, false)
        assert.contains(tostring(err), "NEUTRON_EXTRA_HEADERS")

        -- a JSON array is not a header object
        env.set("NEUTRON_EXTRA_HEADERS", '["x-neutron-internal"]')
        local ok2, err2 = pcall(function() return neutron.client(url, {{ token = "nck_test" }}) end)
        assert.eq(ok2, false)
        assert.contains(tostring(err2), "NEUTRON_EXTRA_HEADERS")

        -- unset is simply no extra headers
        env.set("NEUTRON_EXTRA_HEADERS", nil)
        local plain = neutron.client(url, {{ token = "nck_test" }})
        assert.eq(plain.crm:agent().ok, true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}
