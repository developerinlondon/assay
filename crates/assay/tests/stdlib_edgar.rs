mod common;

use common::run_lua;
use serde_json::json;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn test_edgar_requires_user_agent() {
    let err = run_lua(
        r#"
        local edgar = require("assay.edgar")
        edgar.client({})
    "#,
    )
    .await
    .unwrap_err();
    assert!(format!("{err:#}").contains("user_agent"));
}

#[tokio::test]
async fn test_edgar_tickers_and_find_send_user_agent() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/files/company_tickers.json"))
        .and(header("User-Agent", "my-app contact@example.com"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "0": { "cik_str": 320193, "ticker": "AAPL", "title": "Apple Inc." },
            "1": { "cik_str": 789019, "ticker": "MSFT", "title": "Microsoft Corp" }
        })))
        .mount(&server)
        .await;
    run_lua(&format!(
        r#"
        local edgar = require("assay.edgar")
        local c = edgar.client({{
            user_agent = "my-app contact@example.com",
            www_url = "{u}", data_url = "{u}", efts_url = "{u}",
        }})
        local all = c:tickers()
        assert.eq(#all, 2)
        local hits = c:find("apple")
        assert.eq(#hits, 1)
        assert.eq(hits[1].ticker, "AAPL")
        assert.eq(hits[1].name, "Apple Inc.")
        assert.eq(hits[1].registry_id, "320193")
        assert.eq(hits[1].jurisdiction, "US")
        assert.eq(hits[1].provenance.provider, "registry:edgar")
        assert.not_nil(hits[1].provenance.retrieved_at)
    "#,
        u = server.uri()
    ))
    .await
    .unwrap();
}

#[tokio::test]
async fn test_edgar_find_refuses_empty_needle() {
    let err = run_lua(
        r#"
        local edgar = require("assay.edgar")
        local c = edgar.client({ user_agent = "t t@t.t", www_url = "http://example.invalid",
            data_url = "http://example.invalid", efts_url = "http://example.invalid" })
        c:find("   ")
    "#,
    )
    .await
    .unwrap_err();
    assert!(format!("{err:#}").contains("non-empty"));
}

#[tokio::test]
async fn test_edgar_submissions_pads_cik() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/submissions/CIK0000320193.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            // Submissions zero-pads the CIK; the ticker index does not.
            "cik": "0000320193",
            "name": "Apple Inc.",
            "sic": "3571",
            "sicDescription": "Electronic Computers",
            "addresses": { "business": { "city": "CUPERTINO", "stateOrCountry": "CA" } },
            "tickers": ["AAPL"],
            "exchanges": ["Nasdaq"],
            "website": "",
            "filings": { "recent": { "form": ["10-K", "8-K"] } }
        })))
        .mount(&server)
        .await;
    run_lua(&format!(
        r#"
        local edgar = require("assay.edgar")
        local c = edgar.client({{
            user_agent = "t t@t.t", www_url = "{u}", data_url = "{u}", efts_url = "{u}",
        }})
        local sub = c:submissions(320193)
        assert.eq(sub.name, "Apple Inc.")
        assert.eq(sub.registry_id, "320193")
        assert.eq(sub.industry, "Electronic Computers")
        assert.eq(sub.industry_code, "3571")
        assert.eq(sub.recent_filing_count, 2)
        assert.eq(sub.provenance.provider, "registry:edgar")
        assert.not_nil(sub.provenance.retrieved_at)
        -- EDGAR sends "" for a registrant with no website; absent is not blank.
        assert.eq(sub.domain, nil)
        assert.eq(sub.city, "CUPERTINO")
        -- "CA" here is California, not Canada: a domestic filer's country is
        -- implied by the state code, never stated.
        assert.eq(sub.country, "US")
    "#,
        u = server.uri()
    ))
    .await
    .unwrap();
}

#[tokio::test]
async fn test_edgar_fulltext_normalizes_hits() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/LATEST/search-index"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "hits": { "hits": [ {
                "_id": "abc:doc.htm",
                "_source": {
                    "form": "10-K",
                    "file_date": "2026-02-01",
                    "display_names": ["Apple Inc.  (AAPL)"],
                    "ciks": ["0000320193"]
                }
            } ] }
        })))
        .mount(&server)
        .await;
    run_lua(&format!(
        r#"
        local edgar = require("assay.edgar")
        local c = edgar.client({{
            user_agent = "t t@t.t", www_url = "{u}", data_url = "{u}", efts_url = "{u}",
        }})
        local hits = c:fulltext("supply chain", {{ forms = "10-K" }})
        assert.eq(#hits, 1)
        assert.eq(hits[1].form, "10-K")
        assert.eq(hits[1].company, "Apple Inc.  (AAPL)")
    "#,
        u = server.uri()
    ))
    .await
    .unwrap();
}

/// The ticker index reports CIK `320193` and submissions reports `"0000320193"`
/// for the same company. Two identities for one entity breaks every join a
/// caller makes between the search surface and the profile surface.
#[tokio::test]
async fn test_edgar_reports_one_identity_for_one_company_across_endpoints() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/files/company_tickers.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "0": { "cik_str": 320193, "ticker": "AAPL", "title": "Apple Inc." }
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/submissions/CIK0000320193.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "cik": "0000320193", "name": "Apple Inc."
        })))
        .mount(&server)
        .await;
    run_lua(&format!(
        r#"
        local edgar = require("assay.edgar")
        local c = edgar.client({{
            user_agent = "t t@t.t", www_url = "{u}", data_url = "{u}", efts_url = "{u}",
        }})
        local found = c:find("apple")[1]
        local sub = c:submissions(found.registry_id)
        assert.eq(found.registry_id, "320193")
        assert.eq(sub.registry_id, found.registry_id)
    "#,
        u = server.uri()
    ))
    .await
    .unwrap();
}

/// Foreign filers invert the address: `stateOrCountry` is null and `country`
/// carries free text, which is passed through for want of a code.
#[tokio::test]
async fn test_edgar_keeps_a_foreign_filers_country() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/submissions/CIK0001594805.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "cik": "0001594805",
            "name": "SHOPIFY INC.",
            "addresses": { "business": { "city": "OTTAWA", "stateOrCountry": serde_json::Value::Null,
                                         "country": "Ontario, Canada" } }
        })))
        .mount(&server)
        .await;
    run_lua(&format!(
        r#"
        local edgar = require("assay.edgar")
        local c = edgar.client({{
            user_agent = "t t@t.t", www_url = "{u}", data_url = "{u}", efts_url = "{u}",
        }})
        local sub = c:submissions("1594805")
        assert.eq(sub.country, "Ontario, Canada")
        assert.eq(sub.city, "OTTAWA")
    "#,
        u = server.uri()
    ))
    .await
    .unwrap();
}
