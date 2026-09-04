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

/// A walk that stopped because the vendor ran out of rows has the whole set.
#[tokio::test]
async fn test_a_walk_the_vendor_ended_is_not_truncated() {
    let server = MockServer::start().await;
    mount(&server, "/mailbox", "mailboxes", json!([mailbox_row()]), 1).await;
    run_lua(&script(
        &server.uri(),
        r#"
        local rows, meta = c:mailboxes()
        assert.eq(#rows, 1)
        assert.eq(meta.truncated, false)
        assert.eq(meta.cap, 5000)
        assert.eq(meta.seen, 1)
        "#,
    ))
    .await
    .unwrap();
}

/// A vendor that keeps answering with a full page and a `total_count` it never
/// reaches walks into the page cap. The list is then short, and saying so is
/// the difference between a partial fleet and a fleet that lost rows.
#[tokio::test]
async fn test_a_walk_stopped_by_the_page_cap_says_it_is_truncated() {
    let server = MockServer::start().await;
    let full: Vec<serde_json::Value> = (0..100)
        .map(|n| json!({ "id": format!("mbx_x{n}"), "username": format!("p{n}@example.test") }))
        .collect();
    // Every page is full and the total is never reached, so only the cap stops it.
    Mock::given(method("GET"))
        .and(path("/mailbox"))
        .respond_with(envelope("mailboxes", json!(full), 999_999))
        .mount(&server)
        .await;
    run_lua(&script(
        &server.uri(),
        r#"
        local rows, meta = c:mailboxes()
        assert.eq(meta.truncated, true)
        assert.eq(meta.cap, 5000)
        assert.eq(meta.seen, 5000)
        assert.eq(#rows, 5000)
        "#,
    ))
    .await
    .unwrap();
}

/// The price rides on the mailbox row: `cost` as a decimal string, alongside the
/// cycle it repeats on and the status that says whether the vendor is still
/// charging for it. Every invoice, order and price path the vendor might have
/// put this behind answers 404, so this row is the whole billing surface.
fn priced_row(id: &str, address: &str, cost: &str, cycle: &str) -> serde_json::Value {
    json!({
        "id": id, "username": address, "type": "GOOGLE", "status": "ACTIVE",
        "password": "hunter2", "cost": cost, "billing_cycle": cycle,
        "next_billing_date": "2026-09-30T00:00:00.000Z",
        "domains": { "domain_id": "dom_xa1", "domain": "example.test" },
    })
}

async fn mount_wallet(server: &MockServer, available: serde_json::Value) {
    Mock::given(method("GET"))
        .and(path("/wallet"))
        .and(header("x-api-key", "k"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "success": true,
            "message": "ok",
            "data": { "available": available, "auto_topup": false, "threshold": 0, "topup_amount": 0 },
        })))
        .mount(server)
        .await;
}

/// Rows sharing a price and a cycle collapse into one item, which is what makes
/// `quantity` mean anything. Two prices are two items, never one averaged line.
/// A grouped line names no instance, so it carries no `ref`.
#[tokio::test]
async fn test_the_fleet_bill_groups_boxes_by_the_price_the_vendor_charges() {
    let server = MockServer::start().await;
    mount(
        &server,
        "/mailbox",
        "mailboxes",
        json!([
            priced_row("mbx_xa1", "a@example.test", "2.5", "MONTHLY"),
            priced_row("mbx_xa2", "b@example.test", "2.5", "MONTHLY"),
            priced_row("mbx_xa3", "c@example.test", "4", "MONTHLY"),
        ]),
        3,
    )
    .await;
    mount_wallet(&server, json!(2.5)).await;
    run_lua(&script(
        &server.uri(),
        r#"
        local out = c:costs()
        assert.eq(#out.items, 2)
        assert.eq(out.items[1].kind, "box")
        assert.eq(out.items[1].unit, "mailbox")
        assert.eq(out.items[1].ref, nil)
        assert.eq(out.items[1].quantity, 2)
        assert.eq(out.items[1].unit_price_cents, 250)
        assert.eq(out.items[1].period, "month")
        assert.eq(out.items[1].source, "vendor")
        assert.eq(out.items[2].quantity, 1)
        assert.eq(out.items[2].unit_price_cents, 400)
        assert.eq(out.meta.priced, true)
        assert.eq(out.meta.unpriced, 0)
        assert.eq(out.meta.inactive, 0)
        assert.eq(out.meta.status_unknown, 0)
        assert.eq(out.meta.seen, 3)
        "#,
    ))
    .await
    .unwrap();
}

/// The cycle is part of the grouping key. Two boxes at the same number on
/// different cycles are two different bills, and one line of quantity two would
/// charge the yearly box twelve times over.
#[tokio::test]
async fn test_the_same_price_on_two_cycles_stays_two_items() {
    let server = MockServer::start().await;
    mount(
        &server,
        "/mailbox",
        "mailboxes",
        json!([
            priced_row("mbx_xa1", "a@example.test", "2.5", "MONTHLY"),
            priced_row("mbx_xa2", "b@example.test", "2.5", "YEARLY"),
        ]),
        2,
    )
    .await;
    mount_wallet(&server, json!(0)).await;
    run_lua(&script(
        &server.uri(),
        r#"
        local out = c:costs()
        assert.eq(#out.items, 2)
        assert.eq(out.items[1].period, "month")
        assert.eq(out.items[1].quantity, 1)
        assert.eq(out.items[2].period, "year")
        assert.eq(out.items[2].quantity, 1)
        assert.eq(out.items[1].unit_price_cents, out.items[2].unit_price_cents)
        "#,
    ))
    .await
    .unwrap();
}

/// A decimal string is the only form the price arrives in, and a fleet priced in
/// floats accumulates a rounding error across every row. `19.99` reaching 1998
/// rather than 1999 is the failure this pins.
#[tokio::test]
async fn test_a_decimal_price_string_becomes_whole_cents() {
    let cases = [
        ("2.5", 250),
        ("19.99", 1999),
        ("0.1", 10),
        ("4", 400),
        ("0", 0),
    ];
    for (cost, cents) in cases {
        let server = MockServer::start().await;
        mount(
            &server,
            "/mailbox",
            "mailboxes",
            json!([priced_row("mbx_xa1", "a@example.test", cost, "MONTHLY")]),
            1,
        )
        .await;
        mount_wallet(&server, json!(0)).await;
        let body = format!(
            r#"
            local out = c:costs()
            assert.eq(out.items[1].unit_price_cents, {cents})
            "#
        );
        run_lua(&script(&server.uri(), &body)).await.unwrap();
    }
}

/// `tonumber` reads "0x10" as 16 and "1e2" as 100. Neither is a price the vendor
/// writes, and either one billed would be sixteen or a hundred times the truth.
/// A negative is not a price either. All three are counted, never charged.
#[tokio::test]
async fn test_a_price_that_is_not_a_decimal_string_is_not_a_price() {
    let server = MockServer::start().await;
    mount(
        &server,
        "/mailbox",
        "mailboxes",
        json!([
            priced_row("mbx_xa1", "a@example.test", "0x10", "MONTHLY"),
            priced_row("mbx_xa2", "b@example.test", "1e2", "MONTHLY"),
            priced_row("mbx_xa3", "c@example.test", "-2.5", "MONTHLY"),
            priced_row("mbx_xa4", "d@example.test", "not a price", "MONTHLY"),
        ]),
        4,
    )
    .await;
    mount_wallet(&server, json!(0)).await;
    run_lua(&script(
        &server.uri(),
        r#"
        local out = c:costs()
        assert.eq(#out.items, 0)
        assert.eq(out.meta.unpriced, 4)
        "#,
    ))
    .await
    .unwrap();
}

/// A row the vendor prices with nothing is a row this module cannot price. Read
/// as free it would quietly shrink the bill, so it is counted and left out.
#[tokio::test]
async fn test_a_row_the_vendor_does_not_price_is_counted_rather_than_read_as_free() {
    let server = MockServer::start().await;
    mount(
        &server,
        "/mailbox",
        "mailboxes",
        json!([
            priced_row("mbx_xa1", "a@example.test", "2.5", "MONTHLY"),
            json!({ "id": "mbx_xa2", "username": "b@example.test", "status": "ACTIVE" }),
            priced_row("mbx_xa3", "c@example.test", "2.5", "FORTNIGHTLY"),
        ]),
        3,
    )
    .await;
    mount_wallet(&server, json!(0)).await;
    run_lua(&script(
        &server.uri(),
        r#"
        local out = c:costs()
        assert.eq(#out.items, 1)
        assert.eq(out.items[1].quantity, 1)
        -- One row carries no price at all, one carries a cycle this module
        -- cannot map. Neither is a mailbox that costs nothing.
        assert.eq(out.meta.unpriced, 2)
        "#,
    ))
    .await
    .unwrap();
}

/// A cancelled mailbox is one the vendor has stopped charging for, whatever
/// price its row still carries. Billed anyway, it inflates the fleet's cost
/// with a box nobody is paying for, so it is set aside and counted.
#[tokio::test]
async fn test_a_mailbox_the_vendor_no_longer_charges_for_is_not_billed() {
    let server = MockServer::start().await;
    let mut cancelled = priced_row("mbx_xa2", "b@example.test", "2.5", "MONTHLY");
    cancelled["status"] = json!("CANCELLED");
    let mut suspended = priced_row("mbx_xa3", "c@example.test", "2.5", "MONTHLY");
    suspended["status"] = json!("SUSPENDED");
    mount(
        &server,
        "/mailbox",
        "mailboxes",
        json!([
            priced_row("mbx_xa1", "a@example.test", "2.5", "MONTHLY"),
            cancelled,
            suspended,
        ]),
        3,
    )
    .await;
    mount_wallet(&server, json!(0)).await;
    run_lua(&script(
        &server.uri(),
        r#"
        local out = c:costs()
        assert.eq(#out.items, 1)
        assert.eq(out.items[1].quantity, 1)
        assert.eq(out.meta.inactive, 2)
        assert.eq(out.meta.unpriced, 0)
        assert.eq(out.meta.status_unknown, 0)
        "#,
    ))
    .await
    .unwrap();
}

/// A row is set aside for one reason. A cancelled box whose price is also
/// unreadable is a cancelled box, counted once, so the two counters can be read
/// against the row count without double-counting.
#[tokio::test]
async fn test_an_inactive_row_counts_as_inactive_and_not_also_as_unpriced() {
    let server = MockServer::start().await;
    let mut both = priced_row("mbx_xa1", "a@example.test", "0x10", "MONTHLY");
    both["status"] = json!("CANCELLED");
    mount(&server, "/mailbox", "mailboxes", json!([both]), 1).await;
    mount_wallet(&server, json!(0)).await;
    run_lua(&script(
        &server.uri(),
        r#"
        local out = c:costs()
        assert.eq(out.meta.inactive, 1)
        assert.eq(out.meta.unpriced, 0)
        assert.eq(out.meta.status_unknown, 0)
        "#,
    ))
    .await
    .unwrap();
}

/// The vendor states no currency on any row, and none on the wallet either. A
/// currency here would be one this module invented, so `currency_known` says
/// outright that the numbers carry no unit.
#[tokio::test]
async fn test_no_item_carries_a_currency_and_the_meta_says_none_is_known() {
    let server = MockServer::start().await;
    mount(
        &server,
        "/mailbox",
        "mailboxes",
        json!([priced_row("mbx_xa1", "a@example.test", "2.5", "MONTHLY")]),
        1,
    )
    .await;
    mount_wallet(&server, json!(2.5)).await;
    run_lua(&script(
        &server.uri(),
        r#"
        local out = c:costs()
        assert.eq(out.items[1].currency, nil)
        assert.eq(out.meta.currency_known, false)
        assert.eq(out.meta.currency, nil)
        "#,
    ))
    .await
    .unwrap();
}

/// The next charge is the earliest one still coming, taken off the rows that
/// are actually being billed.
#[tokio::test]
async fn test_the_next_billing_date_is_the_earliest_one_still_coming() {
    let server = MockServer::start().await;
    let mut later = priced_row("mbx_xa1", "a@example.test", "2.5", "MONTHLY");
    later["next_billing_date"] = json!("2026-10-30T00:00:00.000Z");
    let mut sooner = priced_row("mbx_xa2", "b@example.test", "2.5", "MONTHLY");
    sooner["next_billing_date"] = json!("2026-09-30T00:00:00.000Z");
    mount(&server, "/mailbox", "mailboxes", json!([later, sooner]), 2).await;
    mount_wallet(&server, json!(0)).await;
    run_lua(&script(
        &server.uri(),
        r#"
        local out = c:costs()
        assert.eq(out.meta.next_billing_date, "2026-09-30T00:00:00.000Z")
        "#,
    ))
    .await
    .unwrap();
}

/// A costing that answered an empty list when the key was refused would report
/// a fleet that costs nothing, which is the most expensive wrong answer this
/// can give. A refusal reads as a refusal.
#[tokio::test]
async fn test_a_refused_key_reads_as_an_error_not_as_a_fleet_that_costs_nothing() {
    for (status, code) in [
        (401u16, "auth"),
        (403, "auth"),
        (429, "rate_limit"),
        (500, "server"),
    ] {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/mailbox"))
            .respond_with(ResponseTemplate::new(status))
            .mount(&server)
            .await;
        let body = format!(
            r#"
            local out, err = c:costs()
            assert.eq(out, nil)
            assert.eq(err.code, "{code}")
            assert.eq(err.status, {status})
            "#
        );
        run_lua(&script(&server.uri(), &body)).await.unwrap();
    }
}

/// The wallet is the balance these charges draw down, not a charge. A vendor
/// that refuses it leaves the bill standing, and the whole typed error is kept:
/// a 401 and a 500 need different answers from the caller, and a bare code
/// tells them apart no better than a nil balance does.
#[tokio::test]
async fn test_a_refused_wallet_leaves_the_bill_standing_and_keeps_the_whole_error() {
    for (status, code) in [(401u16, "auth"), (429, "rate_limit"), (500, "server")] {
        let server = MockServer::start().await;
        mount(
            &server,
            "/mailbox",
            "mailboxes",
            json!([priced_row("mbx_xa1", "a@example.test", "2.5", "MONTHLY")]),
            1,
        )
        .await;
        Mock::given(method("GET"))
            .and(path("/wallet"))
            .respond_with(ResponseTemplate::new(status))
            .mount(&server)
            .await;
        let body = format!(
            r#"
            local out = c:costs()
            assert.eq(#out.items, 1)
            assert.eq(out.meta.wallet_available_cents, nil)
            assert.eq(out.meta.wallet_error.code, "{code}")
            assert.eq(out.meta.wallet_error.status, {status})
            assert.contains(tostring(out.meta.wallet_error), "clayinbox: ")
            "#
        );
        run_lua(&script(&server.uri(), &body)).await.unwrap();
    }
}

/// The balance lands in cents like every other amount, off a bare number rather
/// than the decimal string the mailbox rows use.
#[tokio::test]
async fn test_the_wallet_balance_lands_in_cents() {
    let server = MockServer::start().await;
    mount(&server, "/mailbox", "mailboxes", json!([]), 0).await;
    mount_wallet(&server, json!(2.5)).await;
    run_lua(&script(
        &server.uri(),
        r#"
        local out = c:costs()
        assert.eq(#out.items, 0)
        assert.eq(out.meta.wallet_available_cents, 250)
        assert.eq(out.meta.wallet_error, nil)
        assert.eq(out.meta.next_billing_date, nil)
        "#,
    ))
    .await
    .unwrap();
}

/// A costing walk hits the same page cap the listing does, and a bill computed
/// from a short walk is a bill missing rows. `truncated` is the difference
/// between a cheap fleet and a fleet only partly counted.
#[tokio::test]
async fn test_a_costing_walk_stopped_by_the_page_cap_says_it_is_truncated() {
    let server = MockServer::start().await;
    let full: Vec<serde_json::Value> = (0..100)
        .map(|n| {
            priced_row(
                &format!("mbx_x{n}"),
                &format!("p{n}@example.test"),
                "2.5",
                "MONTHLY",
            )
        })
        .collect();
    Mock::given(method("GET"))
        .and(path("/mailbox"))
        .respond_with(envelope("mailboxes", json!(full), 999_999))
        .mount(&server)
        .await;
    mount_wallet(&server, json!(0)).await;
    run_lua(&script(
        &server.uri(),
        r#"
        local out = c:costs()
        assert.eq(out.meta.truncated, true)
        assert.eq(out.meta.cap, 5000)
        assert.eq(out.meta.seen, 5000)
        assert.eq(out.items[1].quantity, 5000)
        "#,
    ))
    .await
    .unwrap();
}

/// A row the vendor sent no status for says nothing about whether it is being
/// charged. Counted as inactive it would report a cancellation nobody made;
/// billed, it would charge for a box that may already be gone. It is counted
/// under its own name, so the bill under-reports by an amount a caller can see
/// rather than by a reason that was invented for it.
#[tokio::test]
async fn test_a_row_whose_status_cannot_be_read_is_neither_billed_nor_called_inactive() {
    let server = MockServer::start().await;
    let mut no_status = priced_row("mbx_xa2", "b@example.test", "2.5", "MONTHLY");
    no_status.as_object_mut().unwrap().remove("status");
    let mut unknown_word = priced_row("mbx_xa3", "c@example.test", "2.5", "MONTHLY");
    unknown_word["status"] = json!("PROVISIONING");
    let mut empty = priced_row("mbx_xa4", "d@example.test", "2.5", "MONTHLY");
    empty["status"] = json!("   ");
    let mut cancelled = priced_row("mbx_xa5", "e@example.test", "2.5", "MONTHLY");
    cancelled["status"] = json!("CANCELLED");
    mount(
        &server,
        "/mailbox",
        "mailboxes",
        json!([
            priced_row("mbx_xa1", "a@example.test", "2.5", "MONTHLY"),
            no_status,
            unknown_word,
            empty,
            cancelled,
        ]),
        5,
    )
    .await;
    mount_wallet(&server, json!(0)).await;
    run_lua(&script(
        &server.uri(),
        r#"
        local out = c:costs()
        assert.eq(#out.items, 1)
        assert.eq(out.items[1].quantity, 1)
        -- The vendor said "cancelled" about exactly one of these five.
        assert.eq(out.meta.inactive, 1)
        -- It said nothing readable about three of them.
        assert.eq(out.meta.status_unknown, 3)
        assert.eq(out.meta.unpriced, 0)
        assert.eq(out.meta.seen, 5)
        "#,
    ))
    .await
    .unwrap();
}

/// The words the vendor uses for a box it has stopped charging for are read as
/// what they are, whatever case they arrive in. Read as unknown instead, a
/// cancelled fleet would look like a fleet nobody could account for.
#[tokio::test]
async fn test_the_vendors_own_words_for_a_stopped_box_read_as_inactive() {
    for word in [
        "CANCELLED",
        "canceled",
        "Suspended",
        "DELETED",
        "expired",
        "terminated",
        "INACTIVE",
    ] {
        let server = MockServer::start().await;
        let mut stopped = priced_row("mbx_xa1", "a@example.test", "2.5", "MONTHLY");
        stopped["status"] = json!(word);
        mount(&server, "/mailbox", "mailboxes", json!([stopped]), 1).await;
        mount_wallet(&server, json!(0)).await;
        run_lua(&script(
            &server.uri(),
            r#"
            local out = c:costs()
            assert.eq(out.meta.inactive, 1)
            assert.eq(out.meta.status_unknown, 0)
            "#,
        ))
        .await
        .unwrap();
    }
}
