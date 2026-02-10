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
