use std::time::Duration;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn create_vm() -> mlua::Lua {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();
    assay::lua::create_vm(client).unwrap()
}

async fn run_lua(script: &str) -> Result<(), mlua::Error> {
    let vm = create_vm();
    vm.load(script).exec_async().await
}

async fn eval_lua<T: mlua::FromLua>(script: &str) -> T {
    let vm = create_vm();
    vm.load(script).eval_async::<T>().await.unwrap()
}

mod http_builtins {
    use super::*;

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
}

mod fs_builtins {
    use super::*;

    #[tokio::test]
    async fn test_fs_read() {
        let script = r#"
            local content = fs.read("Cargo.toml")
            assert.contains(content, "assay")
        "#;
        run_lua(script).await.unwrap();
    }

    #[tokio::test]
    async fn test_fs_read_nonexistent() {
        let result = run_lua(r#"fs.read("/nonexistent/file.txt")"#).await;
        assert!(result.is_err());
    }
}

mod base64_builtins {
    use super::*;

    #[tokio::test]
    async fn test_base64_encode() {
        let result: String = eval_lua(r#"return base64.encode("hello world")"#).await;
        assert_eq!(result, "aGVsbG8gd29ybGQ=");
    }

    #[tokio::test]
    async fn test_base64_decode() {
        let result: String = eval_lua(r#"return base64.decode("aGVsbG8gd29ybGQ=")"#).await;
        assert_eq!(result, "hello world");
    }

    #[tokio::test]
    async fn test_base64_roundtrip() {
        let script = r#"
            local original = "special chars: !@#$%^&*()_+-={}[]|;':\",./<>?"
            local encoded = base64.encode(original)
            local decoded = base64.decode(encoded)
            assert.eq(decoded, original)
        "#;
        run_lua(script).await.unwrap();
    }

    #[tokio::test]
    async fn test_base64_empty() {
        let result: String = eval_lua(r#"return base64.encode("")"#).await;
        assert_eq!(result, "");
    }
}

mod crypto_builtins {
    use super::*;

    #[tokio::test]
    async fn test_jwt_sign_rs256() {
        let pem = std::fs::read_to_string("tests/fixtures/test_rsa.pem").unwrap();
        let pub_pem = std::fs::read_to_string("tests/fixtures/test_rsa_pub.pem").unwrap();

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let exp = now + 3600;

        let vm = create_vm();
        vm.globals()
            .set("test_pem", vm.create_string(&pem).unwrap())
            .unwrap();
        vm.globals().set("test_iat", now as i64).unwrap();
        vm.globals().set("test_exp", exp as i64).unwrap();

        let token: String = vm
            .load(
                r#"
                return crypto.jwt_sign({
                    iss = "test-issuer",
                    sub = "test-subject",
                    aud = "test-audience",
                    iat = test_iat,
                    exp = test_exp,
                }, test_pem)
                "#,
            )
            .eval_async()
            .await
            .unwrap();

        assert!(token.contains('.'));
        let parts: Vec<&str> = token.split('.').collect();
        assert_eq!(parts.len(), 3);

        let decoding_key = jsonwebtoken::DecodingKey::from_rsa_pem(pub_pem.as_bytes()).unwrap();
        let mut validation = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::RS256);
        validation.set_audience(&["test-audience"]);
        validation.set_required_spec_claims(&["iss", "sub", "aud", "iat", "exp"]);
        let decoded: jsonwebtoken::TokenData<serde_json::Value> =
            jsonwebtoken::decode(&token, &decoding_key, &validation).unwrap();
        assert_eq!(decoded.claims["iss"], "test-issuer");
        assert_eq!(decoded.claims["sub"], "test-subject");
    }

    #[tokio::test]
    async fn test_jwt_sign_invalid_key() {
        let result = run_lua(
            r#"
            crypto.jwt_sign({ iss = "test" }, "not-a-valid-pem-key")
            "#,
        )
        .await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("invalid PEM key"), "got: {err}");
    }
}

mod json_builtins {
    use super::*;

    #[tokio::test]
    async fn test_json_parse_and_encode() {
        let script = r#"
            local data = json.parse('{"name":"assay","version":1}')
            assert.eq(data.name, "assay")
            assert.eq(data.version, 1)
            local encoded = json.encode(data)
            assert.contains(encoded, '"name"')
        "#;
        run_lua(script).await.unwrap();
    }

    #[tokio::test]
    async fn test_json_array() {
        let script = r#"
            local arr = json.parse('[1,2,3]')
            assert.eq(#arr, 3)
            assert.eq(arr[1], 1)
            assert.eq(arr[3], 3)
        "#;
        run_lua(script).await.unwrap();
    }
}

mod stdlib {
    use super::*;

    #[tokio::test]
    async fn test_require_assay_prometheus() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/query"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "status": "success",
                "data": {
                    "resultType": "vector",
                    "result": [{
                        "metric": {"__name__": "up"},
                        "value": [1700000000.0, "42"]
                    }]
                }
            })))
            .mount(&server)
            .await;

        let script = format!(
            r#"
            local prom = require("assay.prometheus")
            local val = prom.query("{}", "up")
            assert.eq(val, 42)
            "#,
            server.uri()
        );
        run_lua(&script).await.unwrap();
    }

    #[tokio::test]
    async fn test_require_nonexistent_module() {
        let result = run_lua(r#"require("assay.nonexistent")"#).await;
        assert!(result.is_err());
    }
}

mod env_builtins {
    use super::*;

    #[tokio::test]
    async fn test_env_get_existing() {
        unsafe { std::env::set_var("ASSAY_TEST_VAR", "hello") };
        let result: String = eval_lua(r#"return env.get("ASSAY_TEST_VAR")"#).await;
        assert_eq!(result, "hello");
        unsafe { std::env::remove_var("ASSAY_TEST_VAR") };
    }

    #[tokio::test]
    async fn test_env_get_missing() {
        let script = r#"
            local val = env.get("ASSAY_NONEXISTENT_VAR_12345")
            assert.eq(val, nil)
        "#;
        run_lua(script).await.unwrap();
    }
}

mod assert_builtins {
    use super::*;

    #[tokio::test]
    async fn test_assert_eq_pass() {
        run_lua(r#"assert.eq(42, 42)"#).await.unwrap();
    }

    #[tokio::test]
    async fn test_assert_eq_fail() {
        let result = run_lua(r#"assert.eq(1, 2, "numbers differ")"#).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("assert.eq failed"), "got: {err}");
        assert!(err.contains("numbers differ"), "got: {err}");
    }

    #[tokio::test]
    async fn test_assert_gt() {
        run_lua(r#"assert.gt(10, 5)"#).await.unwrap();
        assert!(run_lua(r#"assert.gt(5, 10)"#).await.is_err());
    }

    #[tokio::test]
    async fn test_assert_lt() {
        run_lua(r#"assert.lt(5, 10)"#).await.unwrap();
        assert!(run_lua(r#"assert.lt(10, 5)"#).await.is_err());
    }

    #[tokio::test]
    async fn test_assert_contains() {
        run_lua(r#"assert.contains("hello world", "world")"#)
            .await
            .unwrap();
        assert!(
            run_lua(r#"assert.contains("hello", "xyz")"#)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn test_assert_not_nil() {
        run_lua(r#"assert.not_nil("something")"#).await.unwrap();
        assert!(run_lua(r#"assert.not_nil(nil)"#).await.is_err());
    }
}

mod crypto_hash {
    use super::*;

    #[tokio::test]
    async fn test_sha256_default() {
        let result: String = eval_lua(r#"return crypto.hash("hello")"#).await;
        assert_eq!(
            result,
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }

    #[tokio::test]
    async fn test_sha256_explicit() {
        let result: String = eval_lua(r#"return crypto.hash("hello", "sha256")"#).await;
        assert_eq!(
            result,
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }

    #[tokio::test]
    async fn test_sha512() {
        let result: String = eval_lua(r#"return crypto.hash("hello", "sha512")"#).await;
        assert_eq!(result.len(), 128);
        assert!(result.starts_with("9b71d224"));
    }

    #[tokio::test]
    async fn test_sha3_256() {
        let result: String = eval_lua(r#"return crypto.hash("hello", "sha3-256")"#).await;
        assert_eq!(result.len(), 64);
        assert_eq!(
            result,
            "3338be694f50c5f338814986cdf0686453a888b84f424d792af4b9202398f392"
        );
    }

    #[tokio::test]
    async fn test_sha224() {
        let result: String = eval_lua(r#"return crypto.hash("hello", "sha224")"#).await;
        assert_eq!(result.len(), 56);
    }

    #[tokio::test]
    async fn test_unsupported_algorithm() {
        let result = run_lua(r#"crypto.hash("hello", "md5")"#).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("unsupported algorithm"), "got: {err}");
    }

    #[tokio::test]
    async fn test_empty_input() {
        let result: String = eval_lua(r#"return crypto.hash("", "sha256")"#).await;
        assert_eq!(
            result,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }
}

mod crypto_random {
    use super::*;

    #[tokio::test]
    async fn test_default_length() {
        let result: String = eval_lua(r#"return crypto.random()"#).await;
        assert_eq!(result.len(), 32);
        assert!(result.chars().all(|c| c.is_ascii_alphanumeric()));
    }

    #[tokio::test]
    async fn test_custom_length() {
        let result: String = eval_lua(r#"return crypto.random(64)"#).await;
        assert_eq!(result.len(), 64);
        assert!(result.chars().all(|c| c.is_ascii_alphanumeric()));
    }

    #[tokio::test]
    async fn test_short_length() {
        let result: String = eval_lua(r#"return crypto.random(1)"#).await;
        assert_eq!(result.len(), 1);
    }

    #[tokio::test]
    async fn test_uniqueness() {
        let script = r#"
            local a = crypto.random(32)
            local b = crypto.random(32)
            if a == b then
                error("crypto.random produced identical values")
            end
            return a
        "#;
        let result: String = eval_lua(script).await;
        assert_eq!(result.len(), 32);
    }

    #[tokio::test]
    async fn test_invalid_length() {
        assert!(run_lua(r#"crypto.random(0)"#).await.is_err());
        assert!(run_lua(r#"crypto.random(-1)"#).await.is_err());
    }
}

mod regex_builtins {
    use super::*;

    #[tokio::test]
    async fn test_match_true() {
        let result: bool = eval_lua(r#"return regex.match("hello world", "^hello")"#).await;
        assert!(result);
    }

    #[tokio::test]
    async fn test_match_false() {
        let result: bool = eval_lua(r#"return regex.match("hello world", "^world")"#).await;
        assert!(!result);
    }

    #[tokio::test]
    async fn test_find_with_groups() {
        let script = r#"
            local result = regex.find("2026-02-10", "^(\\d{4})-(\\d{2})-(\\d{2})$")
            assert.eq(result.match, "2026-02-10")
            assert.eq(result.groups[1], "2026")
            assert.eq(result.groups[2], "02")
            assert.eq(result.groups[3], "10")
        "#;
        run_lua(script).await.unwrap();
    }

    #[tokio::test]
    async fn test_find_no_match() {
        let script = r#"
            local result = regex.find("hello", "^\\d+$")
            assert.eq(result, nil)
        "#;
        run_lua(script).await.unwrap();
    }

    #[tokio::test]
    async fn test_find_all() {
        let script = r#"
            local results = regex.find_all("foo123bar456baz", "\\d+")
            assert.eq(#results, 2)
            assert.eq(results[1], "123")
            assert.eq(results[2], "456")
        "#;
        run_lua(script).await.unwrap();
    }

    #[tokio::test]
    async fn test_replace() {
        let result: String =
            eval_lua(r#"return regex.replace("hello world", "world", "lua")"#).await;
        assert_eq!(result, "hello lua");
    }

    #[tokio::test]
    async fn test_replace_all() {
        let result: String =
            eval_lua(r#"return regex.replace("aaa bbb aaa", "aaa", "ccc")"#).await;
        assert_eq!(result, "ccc bbb ccc");
    }

    #[tokio::test]
    async fn test_replace_with_capture_groups() {
        let result: String =
            eval_lua(r#"return regex.replace("John Smith", "(\\w+) (\\w+)", "$2, $1")"#).await;
        assert_eq!(result, "Smith, John");
    }

    #[tokio::test]
    async fn test_invalid_pattern() {
        assert!(run_lua(r#"regex.match("test", "[")"#).await.is_err());
    }
}

mod stdlib_openbao {
    use super::*;

    #[tokio::test]
    async fn test_require_openbao() {
        let script = r#"
            local bao = require("assay.openbao")
            assert.not_nil(bao)
            assert.not_nil(bao.client)
        "#;
        run_lua(script).await.unwrap();
    }

    #[tokio::test]
    async fn test_openbao_read() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/secret/data/mykey"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "data": {"data": {"username": "admin", "password": "secret123"}}
                })),
            )
            .mount(&server)
            .await;

        let script = format!(
            r#"
            local bao = require("assay.openbao")
            local c = bao.client("{}", "test-token")
            local data = c:read("secret/data/mykey")
            assert.eq(data.data.username, "admin")
            assert.eq(data.data.password, "secret123")
            "#,
            server.uri()
        );
        run_lua(&script).await.unwrap();
    }

    #[tokio::test]
    async fn test_openbao_kv_get() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/secret/data/mykey"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "data": {"data": {"foo": "bar"}}
                })),
            )
            .mount(&server)
            .await;

        let script = format!(
            r#"
            local bao = require("assay.openbao")
            local c = bao.client("{}", "test-token")
            local data = c:kv_get("secret", "mykey")
            assert.eq(data.data.foo, "bar")
            "#,
            server.uri()
        );
        run_lua(&script).await.unwrap();
    }

    #[tokio::test]
    async fn test_openbao_write() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/secret/data/newkey"))
            .respond_with(ResponseTemplate::new(204))
            .mount(&server)
            .await;

        let script = format!(
            r#"
            local bao = require("assay.openbao")
            local c = bao.client("{}", "test-token")
            c:write("secret/data/newkey", {{ data = {{ key = "value" }} }})
            "#,
            server.uri()
        );
        run_lua(&script).await.unwrap();
    }

    #[tokio::test]
    async fn test_openbao_read_404() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/secret/data/missing"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let script = format!(
            r#"
            local bao = require("assay.openbao")
            local c = bao.client("{}", "test-token")
            local data = c:read("secret/data/missing")
            assert.eq(data, nil)
            "#,
            server.uri()
        );
        run_lua(&script).await.unwrap();
    }
}

mod stdlib_k8s {
    use super::*;

    #[tokio::test]
    async fn test_require_k8s() {
        let script = r#"
            local k8s = require("assay.k8s")
            assert.not_nil(k8s)
            assert.not_nil(k8s.get)
            assert.not_nil(k8s.get_secret)
        "#;
        run_lua(script).await.unwrap();
    }

    #[tokio::test]
    async fn test_k8s_get_with_base_url() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/namespaces/default/pods"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "kind": "PodList",
                    "items": [{"metadata": {"name": "test-pod"}}]
                })),
            )
            .mount(&server)
            .await;

        let script = format!(
            r#"
            local k8s = require("assay.k8s")
            local pods = k8s.get("/api/v1/namespaces/default/pods", {{
                base_url = "{}",
                token = "fake-token",
            }})
            assert.eq(pods.kind, "PodList")
            assert.eq(pods.items[1].metadata.name, "test-pod")
            "#,
            server.uri()
        );
        run_lua(&script).await.unwrap();
    }

    #[tokio::test]
    async fn test_k8s_get_secret() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/namespaces/infra/secrets/db-creds"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "kind": "Secret",
                    "data": {
                        "username": "YWRtaW4=",
                        "password": "c2VjcmV0"
                    }
                })),
            )
            .mount(&server)
            .await;

        let script = format!(
            r#"
            local k8s = require("assay.k8s")
            local secret = k8s.get_secret("infra", "db-creds", {{
                base_url = "{}",
                token = "fake-token",
            }})
            assert.eq(secret.username, "admin")
            assert.eq(secret.password, "secret")
            "#,
            server.uri()
        );
        run_lua(&script).await.unwrap();
    }

    #[tokio::test]
    async fn test_k8s_exists_true() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/namespaces/infra/secrets/db-creds"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
            .mount(&server)
            .await;

        let script = format!(
            r#"
            local k8s = require("assay.k8s")
            local found = k8s.exists("infra", "secret", "db-creds", {{
                base_url = "{}",
                token = "fake-token",
            }})
            assert.eq(found, true)
            "#,
            server.uri()
        );
        run_lua(&script).await.unwrap();
    }

    #[tokio::test]
    async fn test_k8s_exists_false() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/namespaces/infra/secrets/missing"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let script = format!(
            r#"
            local k8s = require("assay.k8s")
            local found = k8s.exists("infra", "secret", "missing", {{
                base_url = "{}",
                token = "fake-token",
            }})
            assert.eq(found, false)
            "#,
            server.uri()
        );
        run_lua(&script).await.unwrap();
    }

    #[tokio::test]
    async fn test_k8s_is_ready_deployment() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/apis/apps/v1/namespaces/infra/deployments/api"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "status": {"replicas": 3, "readyReplicas": 3}
                })),
            )
            .mount(&server)
            .await;

        let script = format!(
            r#"
            local k8s = require("assay.k8s")
            local ready = k8s.is_ready("infra", "deployment", "api", {{
                base_url = "{}",
                token = "fake-token",
            }})
            assert.eq(ready, true)
            "#,
            server.uri()
        );
        run_lua(&script).await.unwrap();
    }

    #[tokio::test]
    async fn test_k8s_is_ready_deployment_not_ready() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/apis/apps/v1/namespaces/infra/deployments/api"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "status": {"replicas": 3, "readyReplicas": 1}
                })),
            )
            .mount(&server)
            .await;

        let script = format!(
            r#"
            local k8s = require("assay.k8s")
            local ready = k8s.is_ready("infra", "deployment", "api", {{
                base_url = "{}",
                token = "fake-token",
            }})
            assert.eq(ready, false)
            "#,
            server.uri()
        );
        run_lua(&script).await.unwrap();
    }

    #[tokio::test]
    async fn test_k8s_is_ready_pod() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/namespaces/infra/pods/worker-0"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "status": {
                        "conditions": [
                            {"type": "Ready", "status": "True"}
                        ]
                    }
                })),
            )
            .mount(&server)
            .await;

        let script = format!(
            r#"
            local k8s = require("assay.k8s")
            local ready = k8s.is_ready("infra", "pod", "worker-0", {{
                base_url = "{}",
                token = "fake-token",
            }})
            assert.eq(ready, true)
            "#,
            server.uri()
        );
        run_lua(&script).await.unwrap();
    }

    #[tokio::test]
    async fn test_k8s_pod_status() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/namespaces/infra/pods"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "items": [
                        {"status": {"phase": "Running"}},
                        {"status": {"phase": "Running"}},
                        {"status": {"phase": "Pending"}},
                    ]
                })),
            )
            .mount(&server)
            .await;

        let script = format!(
            r#"
            local k8s = require("assay.k8s")
            local status = k8s.pod_status("infra", {{
                base_url = "{}",
                token = "fake-token",
            }})
            assert.eq(status.total, 3)
            assert.eq(status.running, 2)
            assert.eq(status.pending, 1)
            assert.eq(status.failed, 0)
            "#,
            server.uri()
        );
        run_lua(&script).await.unwrap();
    }

    #[tokio::test]
    async fn test_k8s_service_endpoints() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/namespaces/infra/endpoints/postgres"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "subsets": [{
                        "addresses": [
                            {"ip": "10.42.0.5"},
                            {"ip": "10.42.0.6"}
                        ]
                    }]
                })),
            )
            .mount(&server)
            .await;

        let script = format!(
            r#"
            local k8s = require("assay.k8s")
            local ips = k8s.service_endpoints("infra", "postgres", {{
                base_url = "{}",
                token = "fake-token",
            }})
            assert.eq(#ips, 2)
            assert.eq(ips[1], "10.42.0.5")
            assert.eq(ips[2], "10.42.0.6")
            "#,
            server.uri()
        );
        run_lua(&script).await.unwrap();
    }

    #[tokio::test]
    async fn test_k8s_service_endpoints_empty() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/namespaces/infra/endpoints/broken"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"subsets": []})),
            )
            .mount(&server)
            .await;

        let script = format!(
            r#"
            local k8s = require("assay.k8s")
            local ips = k8s.service_endpoints("infra", "broken", {{
                base_url = "{}",
                token = "fake-token",
            }})
            assert.eq(#ips, 0)
            "#,
            server.uri()
        );
        run_lua(&script).await.unwrap();
    }

    #[tokio::test]
    async fn test_k8s_logs() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/namespaces/infra/pods/api-7b9d4/log"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string("2026-02-10 INFO started\n2026-02-10 INFO ready\n"),
            )
            .mount(&server)
            .await;

        let script = format!(
            r#"
            local k8s = require("assay.k8s")
            local output = k8s.logs("infra", "api-7b9d4", {{
                base_url = "{}",
                token = "fake-token",
                tail = 50,
            }})
            assert.contains(output, "started")
            assert.contains(output, "ready")
            "#,
            server.uri()
        );
        run_lua(&script).await.unwrap();
    }

    #[tokio::test]
    async fn test_k8s_rollout_status() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/apis/apps/v1/namespaces/infra/deployments/api"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "spec": {"replicas": 3},
                    "status": {
                        "updatedReplicas": 3,
                        "readyReplicas": 3,
                        "availableReplicas": 3,
                        "unavailableReplicas": 0,
                    }
                })),
            )
            .mount(&server)
            .await;

        let script = format!(
            r#"
            local k8s = require("assay.k8s")
            local rs = k8s.rollout_status("infra", "api", {{
                base_url = "{}",
                token = "fake-token",
            }})
            assert.eq(rs.desired, 3)
            assert.eq(rs.ready, 3)
            assert.eq(rs.complete, true)
            "#,
            server.uri()
        );
        run_lua(&script).await.unwrap();
    }

    #[tokio::test]
    async fn test_k8s_register_crd() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(
                "/apis/argoproj.io/v1alpha1/namespaces/argocd/applications/traefik",
            ))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "kind": "Application",
                    "metadata": {"name": "traefik"},
                    "status": {"health": {"status": "Healthy"}, "sync": {"status": "Synced"}}
                })),
            )
            .mount(&server)
            .await;

        let script = format!(
            r#"
            local k8s = require("assay.k8s")
            k8s.register_crd("application", "argoproj.io", "v1alpha1", "applications")
            local app = k8s.get_resource("argocd", "application", "traefik", {{
                base_url = "{}",
                token = "fake-token",
            }})
            assert.eq(app.metadata.name, "traefik")
            assert.eq(app.status.health.status, "Healthy")
            assert.eq(app.status.sync.status, "Synced")
            "#,
            server.uri()
        );
        run_lua(&script).await.unwrap();
    }

    #[tokio::test]
    async fn test_k8s_list_generic() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/apis/apps/v1/namespaces/infra/deployments"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "kind": "DeploymentList",
                    "items": [
                        {"metadata": {"name": "api"}},
                        {"metadata": {"name": "worker"}},
                    ]
                })),
            )
            .mount(&server)
            .await;

        let script = format!(
            r#"
            local k8s = require("assay.k8s")
            local deploys = k8s.list("infra", "deployment", {{
                base_url = "{}",
                token = "fake-token",
            }})
            assert.eq(#deploys.items, 2)
            assert.eq(deploys.items[1].metadata.name, "api")
            "#,
            server.uri()
        );
        run_lua(&script).await.unwrap();
    }

    #[tokio::test]
    async fn test_k8s_is_ready_generic_conditions() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/apis/networking.k8s.io/v1/namespaces/infra/ingresses/web"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "status": {
                        "conditions": [{"type": "Ready", "status": "True"}]
                    }
                })),
            )
            .mount(&server)
            .await;

        let script = format!(
            r#"
            local k8s = require("assay.k8s")
            local ready = k8s.is_ready("infra", "ingress", "web", {{
                base_url = "{}",
                token = "fake-token",
            }})
            assert.eq(ready, true)
            "#,
            server.uri()
        );
        run_lua(&script).await.unwrap();
    }

    #[tokio::test]
    async fn test_k8s_is_ready_phase_fallback() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/namespaces/infra/persistentvolumeclaims/data-vol"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"status": {"phase": "Bound"}})),
            )
            .mount(&server)
            .await;

        let script = format!(
            r#"
            local k8s = require("assay.k8s")
            local ready = k8s.is_ready("infra", "pvc", "data-vol", {{
                base_url = "{}",
                token = "fake-token",
            }})
            assert.eq(ready, true)
            "#,
            server.uri()
        );
        run_lua(&script).await.unwrap();
    }

    #[tokio::test]
    async fn test_k8s_is_ready_job() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/apis/batch/v1/namespaces/infra/jobs/migrate"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"status": {"succeeded": 1}})),
            )
            .mount(&server)
            .await;

        let script = format!(
            r#"
            local k8s = require("assay.k8s")
            local ready = k8s.is_ready("infra", "job", "migrate", {{
                base_url = "{}",
                token = "fake-token",
            }})
            assert.eq(ready, true)
            "#,
            server.uri()
        );
        run_lua(&script).await.unwrap();
    }

    #[tokio::test]
    async fn test_k8s_node_status() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/nodes"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "items": [{
                        "metadata": {
                            "name": "node-1",
                            "labels": {"node-role.kubernetes.io/control-plane": ""}
                        },
                        "status": {
                            "conditions": [{"type": "Ready", "status": "True"}],
                            "capacity": {"cpu": "4", "memory": "8Gi"},
                            "allocatable": {"cpu": "3800m", "memory": "7Gi"}
                        }
                    }]
                })),
            )
            .mount(&server)
            .await;

        let script = format!(
            r#"
            local k8s = require("assay.k8s")
            local nodes = k8s.node_status({{
                base_url = "{}",
                token = "fake-token",
            }})
            assert.eq(#nodes, 1)
            assert.eq(nodes[1].name, "node-1")
            assert.eq(nodes[1].ready, true)
            assert.eq(nodes[1].roles[1], "control-plane")
            assert.eq(nodes[1].capacity.cpu, "4")
            "#,
            server.uri()
        );
        run_lua(&script).await.unwrap();
    }

    #[tokio::test]
    async fn test_k8s_events_for() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/namespaces/infra/events"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "items": [
                        {"reason": "Scheduled", "message": "Successfully assigned pod"},
                        {"reason": "Pulled", "message": "Container image pulled"},
                    ]
                })),
            )
            .mount(&server)
            .await;

        let script = format!(
            r#"
            local k8s = require("assay.k8s")
            local events = k8s.events_for("infra", "Pod", "api-7b9d4", {{
                base_url = "{}",
                token = "fake-token",
            }})
            assert.eq(#events.items, 2)
            assert.eq(events.items[1].reason, "Scheduled")
            "#,
            server.uri()
        );
        run_lua(&script).await.unwrap();
    }

    #[tokio::test]
    async fn test_k8s_get_configmap() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/namespaces/infra/configmaps/gitops-config"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "kind": "ConfigMap",
                    "data": {
                        "clusterDomain": "jeebon.xyz",
                        "environment": "test"
                    }
                })),
            )
            .mount(&server)
            .await;

        let script = format!(
            r#"
            local k8s = require("assay.k8s")
            local cm = k8s.get_configmap("infra", "gitops-config", {{
                base_url = "{}",
                token = "fake-token",
            }})
            assert.eq(cm.clusterDomain, "jeebon.xyz")
            assert.eq(cm.environment, "test")
            "#,
            server.uri()
        );
        run_lua(&script).await.unwrap();
    }
}

mod time_and_sleep {
    use super::*;

    #[tokio::test]
    async fn test_time_returns_epoch() {
        let result: f64 = eval_lua("return time()").await;
        assert!(result > 1_700_000_000.0);
    }

    #[tokio::test]
    async fn test_sleep_brief() {
        let start = std::time::Instant::now();
        run_lua("sleep(0.05)").await.unwrap();
        let elapsed = start.elapsed();
        assert!(elapsed >= Duration::from_millis(40));
        assert!(elapsed < Duration::from_millis(200));
    }
}
