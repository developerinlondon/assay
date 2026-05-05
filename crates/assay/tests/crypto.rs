mod common;

use common::{create_vm, eval_lua, run_lua};

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
async fn test_jwt_sign_rs256_with_kid() {
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
            }, test_pem, "RS256", { kid = "my-key-id-123" })
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
    assert_eq!(decoded.header.kid, Some("my-key-id-123".to_string()));
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

#[tokio::test]
async fn test_jwt_decode_basic_claims() {
    // JWT with header {"alg":"RS256","typ":"JWT"} and claims
    // {"sub":"user:alice","email":"alice@example.com","role":"admin","groups":["a","b"]}
    // Signature is fake (jwt_decode doesn't verify).
    run_lua(
        r#"
        local token = "eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiJ1c2VyOmFsaWNlIiwiZW1haWwiOiJhbGljZUBleGFtcGxlLmNvbSIsInJvbGUiOiJhZG1pbiIsImdyb3VwcyI6WyJhIiwiYiJdfQ.fake-signature"
        local out = crypto.jwt_decode(token)
        assert.eq(out.header.alg, "RS256")
        assert.eq(out.header.typ, "JWT")
        assert.eq(out.claims.sub, "user:alice")
        assert.eq(out.claims.email, "alice@example.com")
        assert.eq(out.claims.role, "admin")
        assert.eq(#out.claims.groups, 2)
        assert.eq(out.claims.groups[1], "a")
        assert.eq(out.claims.groups[2], "b")
        "#,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn test_jwt_decode_roundtrip_with_jwt_sign() {
    // Sign a JWT with crypto.jwt_sign, then decode it with jwt_decode
    // and verify all the claims round-trip correctly.
    let pem = std::fs::read_to_string("tests/fixtures/test_rsa.pem").unwrap();

    let vm = create_vm();
    vm.globals()
        .set("test_pem", vm.create_string(&pem).unwrap())
        .unwrap();

    vm.load(
        r#"
        local token = crypto.jwt_sign({
          sub = "user:alice",
          email = "alice@example.com",
          role = "admin",
          groups = {"a", "b", "c"},
        }, test_pem, "RS256", { kid = "test-key-1" })

        local decoded = crypto.jwt_decode(token)
        assert.eq(decoded.header.alg, "RS256")
        assert.eq(decoded.header.typ, "JWT")
        assert.eq(decoded.header.kid, "test-key-1")
        assert.eq(decoded.claims.sub, "user:alice")
        assert.eq(decoded.claims.email, "alice@example.com")
        assert.eq(decoded.claims.role, "admin")
        assert.eq(#decoded.claims.groups, 3)
        assert.eq(decoded.claims.groups[1], "a")
        assert.eq(decoded.claims.groups[3], "c")
        "#,
    )
    .exec_async()
    .await
    .unwrap();
}

#[tokio::test]
async fn test_jwt_verify_rs256_roundtrip() {
    // Sign with the private key, verify with the matching public key.
    let priv_pem = std::fs::read_to_string("tests/fixtures/test_rsa.pem").unwrap();
    let pub_pem = std::fs::read_to_string("tests/fixtures/test_rsa_pub.pem").unwrap();

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let exp = now + 3600;

    let vm = create_vm();
    vm.globals()
        .set("priv_pem", vm.create_string(&priv_pem).unwrap())
        .unwrap();
    vm.globals()
        .set("pub_pem", vm.create_string(&pub_pem).unwrap())
        .unwrap();
    vm.globals().set("test_iat", now as i64).unwrap();
    vm.globals().set("test_exp", exp as i64).unwrap();

    vm.load(
        r#"
        local token = crypto.jwt_sign({
            iss = "test-issuer",
            sub = "test-subject",
            aud = "test-audience",
            iat = test_iat,
            exp = test_exp,
        }, priv_pem)
        local out = crypto.jwt_verify(token, pub_pem, {
            audience = "test-audience",
            issuer = "test-issuer",
        })
        assert.eq(out.header.alg, "RS256")
        assert.eq(out.claims.iss, "test-issuer")
        assert.eq(out.claims.sub, "test-subject")
        assert.eq(out.claims.aud, "test-audience")
        "#,
    )
    .exec_async()
    .await
    .unwrap();
}

#[tokio::test]
async fn test_jwt_verify_rejects_wrong_audience() {
    let priv_pem = std::fs::read_to_string("tests/fixtures/test_rsa.pem").unwrap();
    let pub_pem = std::fs::read_to_string("tests/fixtures/test_rsa_pub.pem").unwrap();

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;

    let vm = create_vm();
    vm.globals()
        .set("priv_pem", vm.create_string(&priv_pem).unwrap())
        .unwrap();
    vm.globals()
        .set("pub_pem", vm.create_string(&pub_pem).unwrap())
        .unwrap();
    vm.globals().set("test_iat", now).unwrap();
    vm.globals().set("test_exp", now + 3600).unwrap();

    let result = vm
        .load(
            r#"
            local token = crypto.jwt_sign({
                iss = "test-issuer",
                aud = "intended-audience",
                iat = test_iat,
                exp = test_exp,
            }, priv_pem)
            return crypto.jwt_verify(token, pub_pem, { audience = "different-audience" })
            "#,
        )
        .exec_async()
        .await;
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("audience") || err.contains("Audience") || err.contains("aud"),
        "got: {err}"
    );
}

#[tokio::test]
async fn test_jwt_verify_rejects_wrong_issuer() {
    let priv_pem = std::fs::read_to_string("tests/fixtures/test_rsa.pem").unwrap();
    let pub_pem = std::fs::read_to_string("tests/fixtures/test_rsa_pub.pem").unwrap();

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;

    let vm = create_vm();
    vm.globals()
        .set("priv_pem", vm.create_string(&priv_pem).unwrap())
        .unwrap();
    vm.globals()
        .set("pub_pem", vm.create_string(&pub_pem).unwrap())
        .unwrap();
    vm.globals().set("test_iat", now).unwrap();
    vm.globals().set("test_exp", now + 3600).unwrap();

    let result = vm
        .load(
            r#"
            local token = crypto.jwt_sign({
                iss = "real-issuer",
                aud = "test-audience",
                iat = test_iat,
                exp = test_exp,
            }, priv_pem)
            return crypto.jwt_verify(token, pub_pem, {
                audience = "test-audience",
                issuer = "expected-issuer",
            })
            "#,
        )
        .exec_async()
        .await;
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("issuer") || err.contains("Issuer") || err.contains("iss"),
        "got: {err}"
    );
}

#[tokio::test]
async fn test_jwt_verify_rejects_expired_token() {
    let priv_pem = std::fs::read_to_string("tests/fixtures/test_rsa.pem").unwrap();
    let pub_pem = std::fs::read_to_string("tests/fixtures/test_rsa_pub.pem").unwrap();

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;

    let vm = create_vm();
    vm.globals()
        .set("priv_pem", vm.create_string(&priv_pem).unwrap())
        .unwrap();
    vm.globals()
        .set("pub_pem", vm.create_string(&pub_pem).unwrap())
        .unwrap();
    vm.globals().set("test_iat", now - 7200).unwrap();
    vm.globals().set("test_exp", now - 3600).unwrap();

    let result = vm
        .load(
            r#"
            local token = crypto.jwt_sign({
                iss = "test-issuer",
                aud = "test-audience",
                iat = test_iat,
                exp = test_exp,
            }, priv_pem)
            return crypto.jwt_verify(token, pub_pem, { audience = "test-audience" })
            "#,
        )
        .exec_async()
        .await;
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("expired") || err.contains("ExpiredSignature") || err.contains("exp"),
        "got: {err}"
    );
}

#[tokio::test]
async fn test_jwt_verify_rejects_tampered_signature() {
    let priv_pem = std::fs::read_to_string("tests/fixtures/test_rsa.pem").unwrap();
    let pub_pem = std::fs::read_to_string("tests/fixtures/test_rsa_pub.pem").unwrap();

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;

    let vm = create_vm();
    vm.globals()
        .set("priv_pem", vm.create_string(&priv_pem).unwrap())
        .unwrap();
    vm.globals()
        .set("pub_pem", vm.create_string(&pub_pem).unwrap())
        .unwrap();
    vm.globals().set("test_iat", now).unwrap();
    vm.globals().set("test_exp", now + 3600).unwrap();

    let result = vm
        .load(
            r#"
            -- Two different payloads → different signatures over the same key.
            -- Splice token_a's header.payload onto token_b's signature; result
            -- is well-formed base64url everywhere but signature ≠ message.
            local token_a = crypto.jwt_sign({
                iss = "test-issuer",
                aud = "test-audience",
                iat = test_iat,
                exp = test_exp,
                marker = "alpha",
            }, priv_pem)
            local token_b = crypto.jwt_sign({
                iss = "test-issuer",
                aud = "test-audience",
                iat = test_iat,
                exp = test_exp,
                marker = "beta",
            }, priv_pem)
            local sig_b = token_b:match("[^.]+$")
            local prefix_a = token_a:match("^(.+)%.[^.]+$")
            local tampered = prefix_a .. "." .. sig_b
            return crypto.jwt_verify(tampered, pub_pem, { audience = "test-audience" })
            "#,
        )
        .exec_async()
        .await;
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("InvalidSignature") || err.contains("signature") || err.contains("Signature"),
        "got: {err}"
    );
}

#[tokio::test]
async fn test_jwt_verify_jwks_dispatch_by_kid() {
    let priv_pem = std::fs::read_to_string("tests/fixtures/test_rsa.pem").unwrap();
    let jwks_json = std::fs::read_to_string("tests/fixtures/test_rsa_pub.jwks.json").unwrap();

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;

    let vm = create_vm();
    vm.globals()
        .set("priv_pem", vm.create_string(&priv_pem).unwrap())
        .unwrap();
    vm.globals()
        .set("jwks_json", vm.create_string(&jwks_json).unwrap())
        .unwrap();
    vm.globals().set("test_iat", now).unwrap();
    vm.globals().set("test_exp", now + 3600).unwrap();

    vm.load(
        r#"
        local jwks = json.parse(jwks_json)
        local token = crypto.jwt_sign({
            iss = "test-issuer",
            aud = "test-audience",
            iat = test_iat,
            exp = test_exp,
        }, priv_pem, "RS256", { kid = "test-key-1" })
        local out = crypto.jwt_verify(token, jwks, { audience = "test-audience" })
        assert.eq(out.header.kid, "test-key-1")
        assert.eq(out.claims.iss, "test-issuer")
        "#,
    )
    .exec_async()
    .await
    .unwrap();
}

#[tokio::test]
async fn test_jwt_verify_jwks_rejects_unknown_kid() {
    let priv_pem = std::fs::read_to_string("tests/fixtures/test_rsa.pem").unwrap();
    let jwks_json = std::fs::read_to_string("tests/fixtures/test_rsa_pub.jwks.json").unwrap();

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;

    let vm = create_vm();
    vm.globals()
        .set("priv_pem", vm.create_string(&priv_pem).unwrap())
        .unwrap();
    vm.globals()
        .set("jwks_json", vm.create_string(&jwks_json).unwrap())
        .unwrap();
    vm.globals().set("test_iat", now).unwrap();
    vm.globals().set("test_exp", now + 3600).unwrap();

    let result = vm
        .load(
            r#"
            local jwks = json.parse(jwks_json)
            local token = crypto.jwt_sign({
                iss = "test-issuer",
                aud = "test-audience",
                iat = test_iat,
                exp = test_exp,
            }, priv_pem, "RS256", { kid = "missing-key" })
            return crypto.jwt_verify(token, jwks, { audience = "test-audience" })
            "#,
        )
        .exec_async()
        .await;
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("no key in JWKS matches kid") || err.contains("missing-key"),
        "got: {err}"
    );
}

#[tokio::test]
async fn test_jwt_verify_jwks_requires_kid_in_token() {
    let priv_pem = std::fs::read_to_string("tests/fixtures/test_rsa.pem").unwrap();
    let jwks_json = std::fs::read_to_string("tests/fixtures/test_rsa_pub.jwks.json").unwrap();

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;

    let vm = create_vm();
    vm.globals()
        .set("priv_pem", vm.create_string(&priv_pem).unwrap())
        .unwrap();
    vm.globals()
        .set("jwks_json", vm.create_string(&jwks_json).unwrap())
        .unwrap();
    vm.globals().set("test_iat", now).unwrap();
    vm.globals().set("test_exp", now + 3600).unwrap();

    let result = vm
        .load(
            r#"
            local jwks = json.parse(jwks_json)
            -- Sign without a kid header.
            local token = crypto.jwt_sign({
                iss = "test-issuer",
                aud = "test-audience",
                iat = test_iat,
                exp = test_exp,
            }, priv_pem)
            return crypto.jwt_verify(token, jwks, { audience = "test-audience" })
            "#,
        )
        .exec_async()
        .await;
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("missing 'kid'"), "got: {err}");
}

#[tokio::test]
async fn test_jwt_verify_audience_array() {
    // `audience` accepts both a string and an array of strings; either acts
    // as an allowlist (token's aud must match at least one entry).
    let priv_pem = std::fs::read_to_string("tests/fixtures/test_rsa.pem").unwrap();
    let pub_pem = std::fs::read_to_string("tests/fixtures/test_rsa_pub.pem").unwrap();

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;

    let vm = create_vm();
    vm.globals()
        .set("priv_pem", vm.create_string(&priv_pem).unwrap())
        .unwrap();
    vm.globals()
        .set("pub_pem", vm.create_string(&pub_pem).unwrap())
        .unwrap();
    vm.globals().set("test_iat", now).unwrap();
    vm.globals().set("test_exp", now + 3600).unwrap();

    vm.load(
        r#"
        local token = crypto.jwt_sign({
            iss = "test-issuer",
            aud = "second-audience",
            iat = test_iat,
            exp = test_exp,
        }, priv_pem)
        local out = crypto.jwt_verify(token, pub_pem, {
            audience = { "first-audience", "second-audience", "third-audience" },
        })
        assert.eq(out.claims.aud, "second-audience")
        "#,
    )
    .exec_async()
    .await
    .unwrap();
}

#[tokio::test]
async fn test_jwt_verify_skip_exp_validation() {
    let priv_pem = std::fs::read_to_string("tests/fixtures/test_rsa.pem").unwrap();
    let pub_pem = std::fs::read_to_string("tests/fixtures/test_rsa_pub.pem").unwrap();

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;

    let vm = create_vm();
    vm.globals()
        .set("priv_pem", vm.create_string(&priv_pem).unwrap())
        .unwrap();
    vm.globals()
        .set("pub_pem", vm.create_string(&pub_pem).unwrap())
        .unwrap();
    vm.globals().set("test_iat", now - 7200).unwrap();
    vm.globals().set("test_exp", now - 3600).unwrap();

    vm.load(
        r#"
        local token = crypto.jwt_sign({
            iss = "test-issuer",
            aud = "test-audience",
            iat = test_iat,
            exp = test_exp,
        }, priv_pem)
        -- An expired token verifies fine when validate_exp is off and the
        -- exp claim is no longer required.
        local out = crypto.jwt_verify(token, pub_pem, {
            audience = "test-audience",
            validate_exp = false,
            required_claims = {},
        })
        assert.eq(out.claims.iss, "test-issuer")
        "#,
    )
    .exec_async()
    .await
    .unwrap();
}

#[tokio::test]
async fn test_jwt_verify_rejects_malformed_token() {
    let pub_pem = std::fs::read_to_string("tests/fixtures/test_rsa_pub.pem").unwrap();
    let vm = create_vm();
    vm.globals()
        .set("pub_pem", vm.create_string(&pub_pem).unwrap())
        .unwrap();

    let result = vm
        .load(r#"return crypto.jwt_verify("only.two", pub_pem)"#)
        .exec_async()
        .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_jwt_verify_rejects_invalid_pem() {
    let result = run_lua(
        r#"
        crypto.jwt_verify("a.b.c", "not-a-real-pem-key")
        "#,
    )
    .await;
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("invalid PEM key"), "got: {err}");
}

#[tokio::test]
async fn test_jwt_decode_rejects_malformed_token() {
    // Not three segments
    let result = run_lua(r#"crypto.jwt_decode("only.two")"#).await;
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("three '.'-separated segments"), "got: {err}");

    // Invalid base64url in payload
    let result = run_lua(r#"crypto.jwt_decode("eyJhbGciOiJSUzI1NiJ9.!!!notbase64.sig")"#).await;
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("invalid base64url") || err.contains("payload"),
        "got: {err}"
    );

    // Valid base64url but invalid JSON in payload (base64url of "not json")
    let result = run_lua(r#"crypto.jwt_decode("eyJhbGciOiJSUzI1NiJ9.bm90LWpzb24.sig")"#).await;
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("invalid JSON") || err.contains("payload"),
        "got: {err}"
    );
}

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

#[tokio::test]
async fn test_hmac_sha256_basic() {
    let result: String =
        eval_lua(r#"return crypto.hmac("Jefe", "what do ya want for nothing?", "sha256")"#).await;
    assert_eq!(
        result,
        "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
    );
}

#[tokio::test]
async fn test_hmac_sha256_raw_output() {
    let vm = create_vm();
    let raw_bytes: mlua::String = vm
        .load(r#"return crypto.hmac("Jefe", "what do ya want for nothing?", "sha256", true)"#)
        .eval_async()
        .await
        .unwrap();
    assert_eq!(raw_bytes.as_bytes().len(), 32);
}

#[tokio::test]
async fn test_hmac_sha256_key_chaining() {
    let script = r#"
        local k1 = crypto.hmac("AWS4" .. "secret", "20130524", "sha256", true)
        local k2 = crypto.hmac(k1, "us-east-1", "sha256", true)
        local k3 = crypto.hmac(k2, "s3", "sha256", true)
        local k4 = crypto.hmac(k3, "aws4_request", "sha256", true)
        return #k4
    "#;
    let len: i64 = eval_lua(script).await;
    assert_eq!(len, 32);
}

#[tokio::test]
async fn test_hmac_default_algorithm() {
    let result: String = eval_lua(r#"return crypto.hmac("key", "data")"#).await;
    let explicit: String = eval_lua(r#"return crypto.hmac("key", "data", "sha256")"#).await;
    assert_eq!(result, explicit);
}

#[tokio::test]
async fn test_hmac_sha512() {
    let result: String =
        eval_lua(r#"return crypto.hmac("Jefe", "what do ya want for nothing?", "sha512")"#).await;
    assert_eq!(result.len(), 128);
    assert_eq!(
        result,
        "164b7a7bfcf819e2e395fbe73b56e0a387bd64222e831fd610270cd7ea2505549758bf75c05a994a6d034f65f8f0e6fdcaeab1a34d4a6b4b636e070a38bce737"
    );
}

#[tokio::test]
async fn test_hmac_unsupported_algorithm() {
    let result = run_lua(r#"crypto.hmac("key", "data", "md5")"#).await;
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("unsupported algorithm"), "got: {err}");
}

#[tokio::test]
async fn test_hmac_empty_data() {
    let result: String = eval_lua(r#"return crypto.hmac("key", "")"#).await;
    assert_eq!(
        result,
        "5d5d139563c95b5967b9bd9a8c9b233a9dedb45072794cd232dc1b74832607d0"
    );
}
