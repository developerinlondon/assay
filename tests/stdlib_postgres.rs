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
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_postgres_quote_ident_simple() {
    let result: String = eval_lua(r#"
        local pg = require("assay.postgres")
        return pg._quote_ident("users")
    "#).await;
    assert_eq!(result, "\"users\"");
}

#[tokio::test]
async fn test_postgres_quote_ident_with_quotes() {
    let result: String = eval_lua(r#"
        local pg = require("assay.postgres")
        return pg._quote_ident('my"table')
    "#).await;
    assert_eq!(result, "\"my\"\"table\"");
}

#[tokio::test]
async fn test_postgres_quote_literal_simple() {
    let result: String = eval_lua(r#"
        local pg = require("assay.postgres")
        return pg._quote_literal("hello")
    "#).await;
    assert_eq!(result, "'hello'");
}

#[tokio::test]
async fn test_postgres_quote_literal_with_quotes() {
    let result: String = eval_lua(r#"
        local pg = require("assay.postgres")
        return pg._quote_literal("it's")
    "#).await;
    assert_eq!(result, "'it''s'");
}

#[tokio::test]
async fn test_postgres_client_from_vault_missing_secret() {
    use wiremock::{MockServer, Mock, ResponseTemplate};
    use wiremock::matchers::{method, path};

    let mock_server = MockServer::start().await;
    
    Mock::given(method("GET"))
        .and(path("/v1/secrets/data/db/postgres"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&mock_server)
        .await;

    let script = format!(r#"
        local vault = require("assay.vault")
        local pg = require("assay.postgres")
        
        local vault_client = vault.client("{}", "test-token")
        
        local ok, err = pcall(function()
            pg.client_from_vault(vault_client, "db/postgres", "localhost", 5432)
        end)
        
        assert.eq(ok, false, "client_from_vault should fail when secret is missing")
        assert.not_nil(err, "error message should be present")
    "#, mock_server.uri());
    
    run_lua_local(&script).await.unwrap();
}
