//! Norwegian and Danish company registries against recorded response shapes.
//!
//! Fixtures are trimmed copies of real responses. What is pinned is the
//! normalisation the registries do not do for us: a joinable domain, an ISO
//! date, a single status, and an absent-not-zero headcount.

#[path = "../common/mod.rs"]
mod common;

use common::run_lua;
use serde_json::json;
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn equinor() -> serde_json::Value {
    json!({
        "organisasjonsnummer": "923609016",
        "navn": "EQUINOR ASA",
        "organisasjonsform": { "kode": "ASA", "beskrivelse": "Allmennaksjeselskap" },
        "hjemmeside": "www.equinor.com",
        "forretningsadresse": { "land": "Norge", "landkode": "NO", "poststed": "STAVANGER" },
        "naeringskode1": { "kode": "06.100", "beskrivelse": "Utvinning av råolje" },
        "antallAnsatte": 21239,
        "harRegistrertAntallAnsatte": true,
        "telefon": "51 99 00 00",
        "stiftelsesdato": "1972-09-18",
        "registreringsdatoEnhetsregisteret": "1995-03-12",
        "konkurs": false,
        "underAvvikling": false,
        "underTvangsavviklingEllerTvangsopplosning": false
    })
}

async fn brreg_search(server: &MockServer, body: serde_json::Value) {
    Mock::given(method("GET"))
        .and(path("/enheter"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(server)
        .await;
}

async fn brreg(uri: &str, body: &str) -> Result<(), mlua::Error> {
    run_lua(&format!(
        "local br = require(\"assay.brreg\")\n\
         local c = br.client({{ base_url = \"{uri}\" }})\n{body}"
    ))
    .await
}

#[tokio::test]
async fn test_brreg_normalizes_a_registry_row_into_a_prospect() {
    let server = MockServer::start().await;
    brreg_search(&server, json!({ "_embedded": { "enheter": [equinor()] } })).await;
    brreg(
        &server.uri(),
        r#"
        local hits = c:search("equinor")
        assert.eq(#hits, 1)
        local e = hits[1]
        assert.eq(e.registry_id, "923609016")
        assert.eq(e.name, "EQUINOR ASA")
        assert.eq(e.status, "ACTIVE")
        assert.eq(e.legal_form, "ASA")
        assert.eq(e.jurisdiction, "NO")
        assert.eq(e.city, "STAVANGER")
        assert.eq(e.employees, 21239)
        assert.eq(e.industry_code, "06.100")
        assert.eq(e.founded_at, "1972-09-18")
        assert.eq(e.provenance.provider, "registry:brreg")
        assert.not_nil(e.provenance.retrieved_at)
        "#,
    )
    .await
    .unwrap();
}

/// The registry stores `www.equinor.com`; a prospect list holds the apex.
/// Without this the two never join, which is the whole point of the module.
#[tokio::test]
async fn test_brreg_strips_the_website_down_to_a_joinable_domain() {
    let server = MockServer::start().await;
    for stored in ["www.equinor.com", "https://www.equinor.com/", "EQUINOR.COM"] {
        let mut e = equinor();
        e["hjemmeside"] = json!(stored);
        let s = MockServer::start().await;
        brreg_search(&s, json!({ "_embedded": { "enheter": [e] } })).await;
        brreg(
            &s.uri(),
            r#"assert.eq(c:search("equinor")[1].domain, "equinor.com")"#,
        )
        .await
        .unwrap();
    }
    // An entity with no website must report nil, not an empty string.
    let mut e = equinor();
    e.as_object_mut().unwrap().remove("hjemmeside");
    brreg_search(&server, json!({ "_embedded": { "enheter": [e] } })).await;
    brreg(&server.uri(), r#"assert.eq(c:search("x")[1].domain, nil)"#)
        .await
        .unwrap();
}

/// Norwegian status lives in three separate booleans; a caller acting on
/// "should we approach this company" needs one answer.
#[tokio::test]
async fn test_brreg_collapses_the_three_distress_flags_into_one_status() {
    for (flag, want) in [
        ("konkurs", "BANKRUPT"),
        ("underAvvikling", "LIQUIDATING"),
        ("underTvangsavviklingEllerTvangsopplosning", "COMPULSORY_LIQUIDATION"),
    ] {
        let mut e = equinor();
        e[flag] = json!(true);
        let s = MockServer::start().await;
        brreg_search(&s, json!({ "_embedded": { "enheter": [e] } })).await;
        brreg(
            &s.uri(),
            &format!(r#"assert.eq(c:search("x")[1].status, "{want}")"#),
        )
        .await
        .unwrap();
    }
}

/// Zero employees and "nobody reported" are different claims, and only one of
/// them should reach a record someone later acts on.
#[tokio::test]
async fn test_brreg_reports_an_unreported_headcount_as_absent_not_zero() {
    let mut e = equinor();
    e["harRegistrertAntallAnsatte"] = json!(false);
    e["antallAnsatte"] = json!(0);
    let server = MockServer::start().await;
    brreg_search(&server, json!({ "_embedded": { "enheter": [e] } })).await;
    brreg(&server.uri(), r#"assert.eq(c:search("x")[1].employees, nil)"#)
        .await
        .unwrap();
}

/// A search with no hits omits `_embedded` entirely rather than returning an
/// empty list — normal, and must not read as an error.
#[tokio::test]
async fn test_brreg_treats_no_hits_as_an_empty_list() {
    let server = MockServer::start().await;
    brreg_search(&server, json!({ "page": { "totalElements": 0 } })).await;
    brreg(&server.uri(), r#"assert.eq(#c:search("nothing"), 0)"#)
        .await
        .unwrap();
}

#[tokio::test]
async fn test_brreg_reverse_lookup_retries_the_www_form() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/enheter"))
        .and(query_param("hjemmeside", "equinor.com"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/enheter"))
        .and(query_param("hjemmeside", "www.equinor.com"))
        .respond_with(ResponseTemplate::new(200)
            .set_body_json(json!({ "_embedded": { "enheter": [equinor()] } })))
        .mount(&server)
        .await;
    brreg(
        &server.uri(),
        r#"
        local hits = c:by_website("https://EQUINOR.com/careers")
        assert.eq(#hits, 1)
        assert.eq(hits[1].registry_id, "923609016")
        assert.eq(#c:by_website(""), 0)
        "#,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn test_brreg_reports_an_unknown_org_number_as_nil() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/enheter/999999999"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    brreg(&server.uri(), r#"assert.eq(c:get("999 999 999"), nil)"#)
        .await
        .unwrap();
}

// ---------------------------------------------------------------------------
// Denmark
// ---------------------------------------------------------------------------

fn maersk() -> serde_json::Value {
    json!({
        "vat": 32345794,
        "name": "MAERSK A/S",
        "city": "København K",
        "phone": "33633363",
        "startdate": "04/12 - 2013",
        "employees": serde_json::Value::Null,
        "industrycode": 502000,
        "industrydesc": "Sø- og kysttransport af gods",
        "companydesc": "Aktieselskab",
        "creditbankrupt": false
    })
}

async fn cvr(uri: &str, body: &str) -> Result<(), mlua::Error> {
    run_lua(&format!(
        "local cv = require(\"assay.cvr\")\n\
         local c = cv.client({{ base_url = \"{uri}\", user_agent = \"assay-test\" }})\n{body}"
    ))
    .await
}

#[tokio::test]
async fn test_cvr_normalizes_and_converts_the_danish_date_format() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/"))
        .and(header("user-agent", "assay-test"))
        .respond_with(ResponseTemplate::new(200).set_body_json(maersk()))
        .mount(&server)
        .await;
    cvr(
        &server.uri(),
        r#"
        local e = c:search("maersk")
        assert.eq(e.registry_id, "32345794")
        assert.eq(e.name, "MAERSK A/S")
        assert.eq(e.status, "ACTIVE")
        assert.eq(e.jurisdiction, "DK")
        assert.eq(e.industry_code, "502000")
        assert.eq(e.employees, nil)
        assert.eq(e.founded_at, "2013-12-04")
        assert.eq(e.provenance.provider, "registry:cvr")
        "#,
    )
    .await
    .unwrap();
}

/// The gateway asks callers to identify themselves and throttles those who do
/// not, so the courtesy is structural rather than optional.
#[tokio::test]
async fn test_cvr_refuses_to_run_without_identifying_the_caller() {
    let err = run_lua(
        r#"
        local cv = require("assay.cvr")
        cv.client({ base_url = "http://x" })
    "#,
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(err.contains("user_agent required"), "{err}");
}

/// "No such company" is an answer; a throttle is a failure. They must not look
/// the same to a caller.
#[tokio::test]
async fn test_cvr_separates_absence_from_being_throttled() {
    let missing = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&missing)
        .await;
    cvr(&missing.uri(), r#"assert.eq(c:search("nothing"), nil)"#)
        .await
        .unwrap();

    let limited = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(429))
        .mount(&limited)
        .await;
    let err = cvr(&limited.uri(), r#"c:search("x")"#).await.unwrap_err().to_string();
    assert!(err.contains("rate limited"), "{err}");
}

/// The gateway can answer 200 with an error body instead of a 404.
#[tokio::test]
async fn test_cvr_treats_an_error_body_as_absence() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(200)
            .set_body_json(json!({ "error": "NOT_FOUND", "message": "no result" })))
        .mount(&server)
        .await;
    cvr(&server.uri(), r#"assert.eq(c:get("00000000"), nil)"#)
        .await
        .unwrap();
}

#[tokio::test]
async fn test_cvr_marks_a_closed_or_bankrupt_company() {
    for (field, value, want) in [
        ("enddate", json!("01/01 - 2020"), "CLOSED"),
        ("creditbankrupt", json!(true), "BANKRUPT"),
    ] {
        let mut e = maersk();
        e[field] = value;
        let s = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/"))
            .respond_with(ResponseTemplate::new(200).set_body_json(e))
            .mount(&s)
            .await;
        cvr(
            &s.uri(),
            &format!(r#"assert.eq(c:search("x").status, "{want}")"#),
        )
        .await
        .unwrap();
    }
}
