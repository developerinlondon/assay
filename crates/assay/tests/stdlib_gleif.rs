mod common;

use common::run_lua;
use serde_json::json;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn lei_record(lei: &str, name: &str, country: &str) -> serde_json::Value {
    json!({
        "id": lei,
        "attributes": {
            "lei": lei,
            "entity": {
                "legalName": { "name": name },
                "status": "ACTIVE",
                "jurisdiction": country,
                "legalForm": { "id": "H0PO" },
                "legalAddress": { "city": "Northampton", "country": country }
            },
            "registration": { "initialRegistrationDate": "2013-06-10T00:00:00Z" }
        }
    })
}

#[tokio::test]
async fn test_require_gleif() {
    run_lua(
        r#"
        local gleif = require("assay.gleif")
        assert.not_nil(gleif.client)
    "#,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn test_gleif_search_normalizes_jsonapi() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/lei-records"))
        .and(query_param("filter[entity.legalName]", "Joseph Cheaney"))
        .and(query_param("page[size]", "5"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": [lei_record("529900T8BM49AURSDO55", "Joseph Cheaney & Sons Ltd", "GB")]
        })))
        .mount(&server)
        .await;
    run_lua(&format!(
        r#"
        local gleif = require("assay.gleif")
        local c = gleif.client({{ base_url = "{}" }})
        local out = c:search("Joseph Cheaney", {{ limit = 5 }})
        assert.eq(#out, 1)
        assert.eq(out[1].lei, "529900T8BM49AURSDO55")
        assert.eq(out[1].name, "Joseph Cheaney & Sons Ltd")
        assert.eq(out[1].status, "ACTIVE")
        assert.eq(out[1].country, "GB")
        assert.eq(out[1].provenance.provider, "registry:gleif")
        assert.not_nil(out[1].provenance.retrieved_from)
    "#,
        server.uri()
    ))
    .await
    .unwrap();
}

#[tokio::test]
async fn test_gleif_get_unknown_lei_is_nil() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/lei-records/NOPE"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    run_lua(&format!(
        r#"
        local gleif = require("assay.gleif")
        local c = gleif.client({{ base_url = "{}" }})
        assert.eq(type(c:get("NOPE")), "nil")
    "#,
        server.uri()
    ))
    .await
    .unwrap();
}

#[tokio::test]
async fn test_gleif_empty_lei_is_nil_not_collection() {
    run_lua(
        r#"
        local gleif = require("assay.gleif")
        local c = gleif.client({ base_url = "http://example.invalid" })
        assert.eq(type(c:get("")), "nil")
        assert.eq(type(c:get("   ")), "nil")
    "#,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn test_gleif_fuzzy_returns_values() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/fuzzycompletions"))
        .and(query_param("field", "entity.legalName"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": [
                { "attributes": { "value": "Cheaney Shoes Ltd" } },
                { "attributes": { "value": "Joseph Cheaney & Sons Ltd" } }
            ]
        })))
        .mount(&server)
        .await;
    run_lua(&format!(
        r#"
        local gleif = require("assay.gleif")
        local c = gleif.client({{ base_url = "{}" }})
        local names = c:fuzzy("Cheaney")
        assert.eq(#names, 2)
        assert.eq(names[2], "Joseph Cheaney & Sons Ltd")
    "#,
        server.uri()
    ))
    .await
    .unwrap();
}
