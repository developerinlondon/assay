//! Norwegian, Danish and UK company registries against recorded response shapes.
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

// ---------------------------------------------------------------------------
// United Kingdom
// ---------------------------------------------------------------------------

/// The search endpoint and the profile endpoint describe the same company
/// under different field names. Both fixtures are the documented shapes.
fn ch_search_item() -> serde_json::Value {
    json!({
        "company_number": "00445790",
        "title": "TESCO PLC",
        "company_status": "active",
        "company_type": "plc",
        "date_of_creation": "1947-11-27",
        "address_snippet": "Welwyn Garden City, AL7 1GA",
        "address": { "locality": "Welwyn Garden City", "country": "United Kingdom",
                     "postal_code": "AL7 1GA", "address_line_1": "Shire Park" }
    })
}

fn ch_profile() -> serde_json::Value {
    json!({
        "company_number": "00445790",
        "company_name": "TESCO PLC",
        "company_status": "active",
        "type": "plc",
        "date_of_creation": "1947-11-27",
        "jurisdiction": "england-wales",
        "sic_codes": ["47110", "47190"],
        "has_been_liquidated": false,
        "registered_office_address": { "locality": "Welwyn Garden City",
                                       "country": "United Kingdom", "postal_code": "AL7 1GA" }
    })
}

async fn ch(uri: &str, body: &str) -> Result<(), mlua::Error> {
    run_lua(&format!(
        "local ch = require(\"assay.companies_house\")\n\
         local c = ch.client({{ base_url = \"{uri}\", api_key = \"testkey\" }})\n{body}"
    ))
    .await
}

/// The key is the Basic username with an empty password; the trailing colon is
/// load-bearing, and getting it wrong reads as a bad key rather than a bad request.
#[tokio::test]
async fn test_companies_house_authenticates_with_the_key_as_basic_username() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/search/companies"))
        // base64("testkey:")
        .and(header("authorization", "Basic dGVzdGtleTo="))
        .respond_with(ResponseTemplate::new(200)
            .set_body_json(json!({ "items": [ch_search_item()], "total_results": 1 })))
        .mount(&server)
        .await;
    ch(
        &server.uri(),
        r#"
        local hits = c:search("tesco")
        assert.eq(#hits, 1)
        assert.eq(hits[1].name, "TESCO PLC")
        "#,
    )
    .await
    .unwrap();
}

/// Search returns `title`; the profile returns `company_name`. Reading only one
/// yields a record with a nil name from the other endpoint.
#[tokio::test]
async fn test_companies_house_reads_the_name_from_both_endpoint_shapes() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/search/companies"))
        .respond_with(ResponseTemplate::new(200)
            .set_body_json(json!({ "items": [ch_search_item()] })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/company/00445790"))
        .respond_with(ResponseTemplate::new(200).set_body_json(ch_profile()))
        .mount(&server)
        .await;
    ch(
        &server.uri(),
        r#"
        local from_search = c:search("tesco")[1]
        local from_profile = c:get("00445790")
        assert.eq(from_search.name, "TESCO PLC")
        assert.eq(from_profile.name, "TESCO PLC")
        assert.eq(from_search.legal_form, "plc")
        assert.eq(from_profile.legal_form, "plc")
        assert.eq(from_search.city, "Welwyn Garden City")
        assert.eq(from_profile.city, "Welwyn Garden City")
        assert.eq(from_profile.registry_id, "00445790")
        assert.eq(from_profile.status, "ACTIVE")
        assert.eq(from_profile.jurisdiction, "GB")
        assert.eq(from_profile.industry_code, "47110")
        assert.eq(from_profile.founded_at, "1947-11-27")
        assert.eq(from_profile.provenance.provider, "registry:companies_house")
        assert.not_nil(from_profile.provenance.retrieved_at)
        assert.not_nil(from_profile.provenance.retrieved_from)
        "#,
    )
    .await
    .unwrap();
}

/// Twelve registry statuses, one question: can this company be approached.
#[tokio::test]
async fn test_companies_house_buckets_the_registry_statuses() {
    for (raw, want) in [
        ("active", "ACTIVE"),
        ("dissolved", "CLOSED"),
        ("converted-closed", "CLOSED"),
        ("liquidation", "LIQUIDATING"),
        ("administration", "LIQUIDATING"),
        ("voluntary-arrangement", "LIQUIDATING"),
        // A status the registry adds later must not silently become ACTIVE.
        ("some-new-status", "SOME_NEW_STATUS"),
    ] {
        let mut p = ch_profile();
        p["company_status"] = json!(raw);
        let s = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/company/00445790"))
            .respond_with(ResponseTemplate::new(200).set_body_json(p))
            .mount(&s)
            .await;
        ch(
            &s.uri(),
            &format!(r#"assert.eq(c:get("00445790").status, "{want}")"#),
        )
        .await
        .unwrap();
    }
}

/// No hits is an answer, not a failure, and the envelope omits `items`.
#[tokio::test]
async fn test_companies_house_treats_no_hits_as_an_empty_list() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/search/companies"))
        .respond_with(ResponseTemplate::new(200)
            .set_body_json(json!({ "total_results": 0, "items_per_page": 20 })))
        .mount(&server)
        .await;
    ch(&server.uri(), r#"assert.eq(#c:search("nothing at all"), 0)"#)
        .await
        .unwrap();
}

/// An unknown company number is absence; a rejected key or a throttle is a
/// failure. A caller must be able to tell them apart.
#[tokio::test]
async fn test_companies_house_separates_absence_from_rejection_and_throttling() {
    let missing = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/company/99999999"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&missing)
        .await;
    ch(&missing.uri(), r#"assert.eq(c:get("9999 9999"), nil)"#)
        .await
        .unwrap();

    for (status, needle) in [(401u16, "rejected the api_key"), (429, "rate limited")] {
        let s = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/company/00445790"))
            .respond_with(ResponseTemplate::new(status))
            .mount(&s)
            .await;
        let err = ch(&s.uri(), r#"c:get("00445790")"#).await.unwrap_err().to_string();
        assert!(err.contains(needle), "status {status}: {err}");
    }
}

/// Companies House is the one registry module that needs a key. Failing at
/// construction beats a 401 later that reads like an outage.
#[tokio::test]
async fn test_companies_house_refuses_to_construct_without_a_key() {
    let err = run_lua(
        r#"
        local ch = require("assay.companies_house")
        ch.client({ base_url = "http://x" })
    "#,
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(err.contains("api_key required"), "{err}");
}

/// The profile names the company; this names the person to write to. A
/// resignation date is the only thing separating a contact from a dead lead,
/// and the withheld day of birth must not become a fabricated one.
#[tokio::test]
async fn test_companies_house_officers_carry_appointment_and_partial_birth_date() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/company/00445790/officers"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "active_count": 1,
            "resigned_count": 1,
            "items": [
                { "name": "SMITH, John", "officer_role": "director",
                  "appointed_on": "2015-06-01", "nationality": "British",
                  "occupation": "Company Director", "country_of_residence": "United Kingdom",
                  "date_of_birth": { "month": 5, "year": 1980 } },
                { "name": "JONES, Ada", "officer_role": "secretary",
                  "appointed_on": "2010-01-04", "resigned_on": "2019-03-02" }
            ]
        })))
        .mount(&server)
        .await;
    ch(
        &server.uri(),
        r#"
        local all = c:officers("00445790")
        assert.eq(#all, 2)
        local d = all[1]
        assert.eq(d.full_name, "SMITH, John")
        assert.eq(d.officer_role, "director")
        assert.eq(d.title, "Company Director")
        assert.eq(d.appointed_on, "2015-06-01")
        assert.eq(d.born_at, "1980-05")
        assert.eq(d.active, true)
        assert.eq(d.provenance.provider, "registry:companies_house")
        assert.not_nil(d.provenance.retrieved_at)

        local gone = all[2]
        assert.eq(gone.active, false)
        assert.eq(gone.resigned_on, "2019-03-02")
        -- No date of birth reported means none is claimed.
        assert.eq(gone.born_at, nil)

        local serving = c:officers("00445790", { active_only = true })
        assert.eq(#serving, 1)
        assert.eq(serving[1].full_name, "SMITH, John")
        "#,
    )
    .await
    .unwrap();
}

/// An empty company number would hit the collection path and answer 200 with
/// something that is not the company asked for.
#[tokio::test]
async fn test_companies_house_refuses_an_empty_company_number() {
    let err = ch("http://example.invalid", r#"c:get("   ")"#)
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("company number is required"), "{err}");
}
