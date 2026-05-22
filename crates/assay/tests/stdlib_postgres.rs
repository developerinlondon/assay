mod common;

use common::{eval_lua, run_lua, run_lua_local};

#[tokio::test]
async fn test_require_postgres() {
    let script = r#"
        local pg = require("assay.postgres")
        assert.not_nil(pg, "postgres module should load")
        assert.not_nil(pg.client, "postgres.client should exist")
        assert.not_nil(pg.client_from_vault, "postgres.client_from_vault should exist")
        assert.not_nil(pg._quote_ident, "postgres._quote_ident should exist")
        assert.not_nil(pg._quote_literal, "postgres._quote_literal should exist")
        assert.not_nil(pg._build_dsn, "postgres._build_dsn should exist")
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_postgres_build_dsn_plain() {
    let result: String = eval_lua(
        r#"
        local pg = require("assay.postgres")
        return pg._build_dsn("localhost", 5432, "alice", "s3cret", "mydb")
    "#,
    )
    .await;
    assert_eq!(result, "postgres://alice:s3cret@localhost:5432/mydb");
}

// Regression: AWS RDS generated passwords routinely contain "?", "/",
// "#", "@" — the URI sub-delim/gen-delim set. Concatenating raw used
// to break sqlx's URL parser ("invalid port number" because "?" was
// read as the query-string boundary).
#[tokio::test]
async fn test_postgres_build_dsn_password_with_url_specials() {
    let result: String = eval_lua(
        r#"
        local pg = require("assay.postgres")
        return pg._build_dsn("host", 5432, "user", "di_)XJp0NTl[P|?)a7b@k9", "postgres")
    "#,
    )
    .await;
    assert!(
        !result.contains("|?"),
        "raw '?' must not appear in the DSN authority — got {result}"
    );
    assert!(
        result.contains("%3F"),
        "'?' must be percent-encoded to %3F — got {result}"
    );
    // Confirms the URL parses as expected: port 5432 visible in the right slot.
    assert!(
        result.contains("@host:5432/postgres"),
        "host/port must be after the encoded password — got {result}"
    );
}

#[tokio::test]
async fn test_postgres_build_dsn_password_with_at_sign() {
    let result: String = eval_lua(
        r#"
        local pg = require("assay.postgres")
        return pg._build_dsn("host", 5432, "user", "a@b", "postgres")
    "#,
    )
    .await;
    assert!(result.contains("a%40b"));
    assert!(result.ends_with("@host:5432/postgres"));
}

#[tokio::test]
async fn test_postgres_build_dsn_username_encoded() {
    let result: String = eval_lua(
        r#"
        local pg = require("assay.postgres")
        return pg._build_dsn("host", 5432, "us er", "pw", "db")
    "#,
    )
    .await;
    assert!(result.contains("us%20er:pw"));
}

#[tokio::test]
async fn test_postgres_quote_ident_simple() {
    let result: String = eval_lua(
        r#"
        local pg = require("assay.postgres")
        return pg._quote_ident("users")
    "#,
    )
    .await;
    assert_eq!(result, "\"users\"");
}

#[tokio::test]
async fn test_postgres_quote_ident_with_quotes() {
    let result: String = eval_lua(
        r#"
        local pg = require("assay.postgres")
        return pg._quote_ident('my"table')
    "#,
    )
    .await;
    assert_eq!(result, "\"my\"\"table\"");
}

#[tokio::test]
async fn test_postgres_quote_literal_simple() {
    let result: String = eval_lua(
        r#"
        local pg = require("assay.postgres")
        return pg._quote_literal("hello")
    "#,
    )
    .await;
    assert_eq!(result, "'hello'");
}

#[tokio::test]
async fn test_postgres_quote_literal_with_quotes() {
    let result: String = eval_lua(
        r#"
        local pg = require("assay.postgres")
        return pg._quote_literal("it's")
    "#,
    )
    .await;
    assert_eq!(result, "'it''s'");
}

#[tokio::test]
async fn test_postgres_client_from_vault_missing_secret() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/v1/secrets/data/db/postgres"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&mock_server)
        .await;

    let script = format!(
        r#"
        local vault = require("assay.hashicorp.vault")
        local pg = require("assay.postgres")
        
        local vault_client = vault.client("{}", "test-token")
        
        local ok, err = pcall(function()
            pg.client_from_vault(vault_client, "db/postgres", "localhost", 5432)
        end)
        
        assert.eq(ok, false, "client_from_vault should fail when secret is missing")
        assert.not_nil(err, "error message should be present")
    "#,
        mock_server.uri()
    );

    run_lua_local(&script).await.unwrap();
}
