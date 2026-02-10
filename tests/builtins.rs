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
