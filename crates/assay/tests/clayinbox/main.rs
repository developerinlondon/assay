//! assay.clayinbox against the response shapes probed from the live API on
//! 2026-09-04, anonymised. What is pinned: the `x-api-key` header, the address
//! coming off `username` with the domain from its nesting, credentials never
//! reaching `raw`, paging to the end, and auth, rate limiting and a Cloudflare
//! block page reading as themselves rather than as an empty fleet.

#[path = "../common/mod.rs"]
mod common;

use common::run_lua;
use serde_json::json;
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn script(uri: &str, body: &str) -> String {
    format!(
        "local cb = require(\"assay.clayinbox\")\n\
         local c = cb.client({{ api_key = \"k\", base_url = \"{uri}\" }})\n{body}"
    )
}

fn envelope(key: &str, rows: serde_json::Value, total: u64) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({
        "success": true,
        "message": "ok",
        "data": { key: rows, "limit": 100, "page": 1, "total_count": total },
    }))
}

async fn mount(server: &MockServer, route: &str, key: &str, rows: serde_json::Value, total: u64) {
    Mock::given(method("GET"))
        .and(path(route))
        .and(header("x-api-key", "k"))
        .respond_with(envelope(key, rows, total))
        .mount(server)
        .await;
}

fn domain_row() -> serde_json::Value {
    json!({
        "domain_id": "dom_xa1", "domain": "Example.TEST", "status": "ACTIVE",
        "mailbox_count": 3, "dmarc": true, "spf": true, "dkim": true,
        "mx_records": true, "workspace_type": "GOOGLE", "blacklisted": false,
    })
}

fn mailbox_row() -> serde_json::Value {
    json!({
        "id": "mbx_xa1", "first_name": "Ada", "last_name": "Person",
        "username": "Ada@Example.TEST", "type": "GOOGLE", "status": "ACTIVE",
        "master_inbox": false, "password": "hunter2",
        "domains": { "domain_id": "dom_xa1", "domain": "example.test" },
    })
}

/// The key rides in `x-api-key`; the mock refuses anything else, so a client
/// that sent an Authorization header would 404 rather than pass.
#[tokio::test]
async fn test_a_domain_row_carries_its_dns_flags_and_lands_lowercase() {
    let server = MockServer::start().await;
    mount(&server, "/domain", "domains", json!([domain_row()]), 1).await;
    run_lua(&script(
        &server.uri(),
        r#"
        local rows = c:domains()
        assert.eq(#rows, 1)
        assert.eq(rows[1].domain, "example.test")
        assert.eq(rows[1].provider, "clayinbox")
        assert.eq(rows[1].provider_ref, "dom_xa1")
        assert.eq(rows[1].status, "active")
        assert.eq(rows[1].dns.spf, true)
        assert.eq(rows[1].dns.dkim, true)
        assert.eq(rows[1].dns.dmarc, true)
        assert.eq(rows[1].dns.mx, true)
        "#,
    ))
    .await
    .unwrap();
}

/// A flag the vendor omits is a record it has not seen published. Reading that
/// as unknown would let a domain with no DMARC pass for configured.
#[tokio::test]
async fn test_an_absent_dns_flag_is_a_record_that_is_not_published() {
    let server = MockServer::start().await;
    mount(
        &server,
        "/domain",
        "domains",
        json!([{ "domain": "bare.test", "dkim": true }]),
        1,
    )
    .await;
    run_lua(&script(
        &server.uri(),
        r#"
        local d = c:domains()[1]
        assert.eq(d.dns.dkim, true)
        assert.eq(d.dns.spf, false)
        assert.eq(d.dns.dmarc, false)
        assert.eq(d.dns.mx, false)
        assert.eq(d.status, "unknown")
        "#,
    ))
    .await
    .unwrap();
}

#[tokio::test]
async fn test_a_mailbox_address_comes_off_username_with_the_domain_from_its_nesting() {
    let server = MockServer::start().await;
    mount(&server, "/mailbox", "mailboxes", json!([mailbox_row()]), 1).await;
    run_lua(&script(
        &server.uri(),
        r#"
        local b = c:mailboxes()[1]
        assert.eq(b.address, "ada@example.test")
        assert.eq(b.domain, "example.test")
        assert.eq(b.status, "active")
        assert.eq(b.provider, "clayinbox")
        assert.eq(b.provider_ref, "mbx_xa1")
        "#,
    ))
    .await
    .unwrap();
}

/// `raw` exists so a caller can read the fields this module does not map. The
/// live row carries the mailbox's own password, and a credential on `raw` would
/// reach every log that prints a row.
#[tokio::test]
async fn test_the_mailbox_password_never_reaches_the_raw_row() {
    let server = MockServer::start().await;
    mount(&server, "/mailbox", "mailboxes", json!([mailbox_row()]), 1).await;
    run_lua(&script(
        &server.uri(),
        r#"
        local b = c:mailboxes()[1]
        assert.eq(b.raw.password, "[redacted]")
        assert.eq(b.raw.first_name, "Ada")
        "#,
    ))
    .await
    .unwrap();
}

/// A row whose address cannot be read is not a mailbox a caller can act on, and
/// a guessed one would be worse than none.
#[tokio::test]
async fn test_a_row_whose_address_cannot_be_read_is_dropped_not_guessed() {
    let server = MockServer::start().await;
    mount(
        &server,
        "/mailbox",
        "mailboxes",
        json!([
            { "id": "mbx_xb1", "status": "ACTIVE" },
            { "id": "mbx_xb2", "username": "not-an-address" },
            { "id": "mbx_xb3", "username": "ada@one.test", "domains": { "domain": "other.test" } },
            mailbox_row(),
        ]),
        4,
    )
    .await;
    run_lua(&script(
        &server.uri(),
        r#"
        local rows = c:mailboxes()
        assert.eq(#rows, 1)
        assert.eq(rows[1].address, "ada@example.test")
        "#,
    ))
    .await
    .unwrap();
}

/// The vendor pages at 100. A caller that read only the first page would report
/// a fleet smaller than the one it holds.
#[tokio::test]
async fn test_paging_walks_past_the_first_page_and_stops_on_the_total() {
    let server = MockServer::start().await;
    let full: Vec<serde_json::Value> = (0..100)
        .map(|n| json!({ "id": format!("mbx_x{n}"), "username": format!("p{n}@example.test") }))
        .collect();
    Mock::given(method("GET"))
        .and(path("/mailbox"))
        .and(query_param("page", "1"))
        .respond_with(envelope("mailboxes", json!(full), 101))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/mailbox"))
        .and(query_param("page", "2"))
        .respond_with(envelope(
            "mailboxes",
            json!([{ "id": "mbx_xlast", "username": "last@example.test" }]),
            101,
        ))
        .mount(&server)
        .await;
    run_lua(&script(
        &server.uri(),
        r#"
        local rows = c:mailboxes()
        assert.eq(#rows, 101)
        assert.eq(rows[101].address, "last@example.test")
        "#,
    ))
    .await
    .unwrap();
}

/// A refused key is not an empty fleet. Reported as one it would tell an
/// operator every domain they hold had vanished.
#[tokio::test]
async fn test_auth_and_rate_limits_read_as_themselves_not_as_absence() {
    for (status, code) in [
        (401u16, "auth"),
        (403, "auth"),
        (429, "rate_limit"),
        (503, "server"),
    ] {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/domain"))
            .respond_with(ResponseTemplate::new(status))
            .mount(&server)
            .await;
        run_lua(&script(
            &server.uri(),
            &format!(
                r#"
                local rows, err = c:domains()
                assert.eq(rows, nil)
                assert.eq(err.code, "{code}")
                assert.eq(err.status, {status})
                assert.contains(tostring(err), "clayinbox: ")
                "#
            ),
        ))
        .await
        .unwrap();
    }
}

/// Cloudflare's block page is HTML under an HTTP 200. Parsed as an empty list it
/// would read as a workspace that had lost everything in it.
#[tokio::test]
async fn test_a_block_page_under_a_200_reads_as_unreadable_not_as_empty() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/domain"))
        .respond_with(ResponseTemplate::new(200).set_body_string("<html>error 1010</html>"))
        .mount(&server)
        .await;
    run_lua(&script(
        &server.uri(),
        r#"
        local rows, err = c:domains()
        assert.eq(rows, nil)
        assert.eq(err.code, "unreadable")
        "#,
    ))
    .await
    .unwrap();
}

/// A client with no key at all is a programming error, not a vendor answer.
#[tokio::test]
async fn test_a_client_refuses_to_build_without_a_key() {
    let err =
        run_lua("local cb = require(\"assay.clayinbox\")\ncb.client({ base_url = \"http://x\" })")
            .await
            .unwrap_err()
            .to_string();
    assert!(err.contains("api key required"), "gave {err}");
}
