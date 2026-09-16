//! Apify against recorded response shapes: no run leaves without a spend cap,
//! the actor id reaches the API in the form it wants, a poll keeps going until
//! the run says it is over, a failed run still reports what it cost, the
//! dataset is read a page at a time, and each typed reader answers in one
//! shape whatever the vendor's field types did that day.

#[path = "../common/mod.rs"]
mod common;
mod smoke;

use common::run_lua;
use serde_json::{Value, json};
use wiremock::matchers::{body_partial_json, header, method, path, query_param};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

fn client(uri: &str, body: &str) -> String {
    format!(
        "local apify = require(\"assay.apify\")\n\
         local c = apify.client({{ token = \"tok\", base_url = \"{uri}\" }})\n{body}"
    )
}

async fn ok(server: &MockServer, body: &str) {
    run_lua(&client(&server.uri(), body)).await.unwrap();
}

fn assert_says(err: mlua::Error, want: &str, ctx: &str) {
    let text = err.to_string();
    assert!(text.contains(want), "{ctx} gave {text}, wanted {want}");
}

fn run_json(id: &str, status: &str, usage: f64) -> Value {
    run_json_with(id, status, usage, json!({ "profile": 1 }))
}

fn run_json_with(id: &str, status: &str, usage: f64, counts: Value) -> Value {
    json!({
        "id": id,
        "actId": "dSCLg0C3YEZ83HzYX",
        "status": status,
        "statusMessage": format!("Actor is {status}"),
        "startedAt": "2026-09-16T10:00:00.000Z",
        "finishedAt": if status == "RUNNING" || status == "READY" { Value::Null } else { json!("2026-09-16T10:00:20.000Z") },
        "defaultDatasetId": "ds1",
        "defaultKeyValueStoreId": "kv1",
        "usageTotalUsd": usage,
        "chargedEventCounts": counts,
        "options": { "maxTotalChargeUsd": 0.5, "timeoutSecs": 300 },
        "exitCode": if status == "SUCCEEDED" { json!(0) } else { Value::Null }
    })
}

fn envelope(data: Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({ "data": data }))
}

/// Answers each read of a run with the next (status, usage) in the list, then
/// keeps answering with the last one.
struct Sequence {
    reads: Vec<(&'static str, f64)>,
    calls: std::sync::atomic::AtomicUsize,
}

impl Respond for Sequence {
    fn respond(&self, _: &Request) -> ResponseTemplate {
        let n = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let (status, usage) = self.reads[n.min(self.reads.len() - 1)];
        envelope(run_json("run1", status, usage))
    }
}

fn poll_sequence(reads: Vec<(&'static str, f64)>) -> Sequence {
    Sequence {
        reads,
        calls: std::sync::atomic::AtomicUsize::new(0),
    }
}

/// The settle read after a finished run answers with the same figures.
async fn mount_settled(server: &MockServer, status: &str, usage: f64) {
    Mock::given(method("GET"))
        .and(path("/actor-runs/run1"))
        .respond_with(envelope(run_json("run1", status, usage)))
        .mount(server)
        .await;
}

async fn mount_start(server: &MockServer, actor_path: &str, status: &str) {
    Mock::given(method("POST"))
        .and(path(format!("/acts/{actor_path}/runs")))
        .respond_with(
            ResponseTemplate::new(201)
                .set_body_json(json!({ "data": run_json("run1", status, 0.0) })),
        )
        .mount(server)
        .await;
}

/// A start that must see the given input once and answers with a finished run.
async fn mount_start_expecting(server: &MockServer, actor_path: &str, input: Value, usage: f64) {
    Mock::given(method("POST"))
        .and(path(format!("/acts/{actor_path}/runs")))
        .and(body_partial_json(input))
        .respond_with(
            ResponseTemplate::new(201)
                .set_body_json(json!({ "data": run_json("run1", "SUCCEEDED", usage) })),
        )
        .expect(1)
        .mount(server)
        .await;
    mount_settled(server, "SUCCEEDED", usage).await;
}

async fn mount_items(server: &MockServer, items: Value) {
    Mock::given(method("GET"))
        .and(path("/datasets/ds1/items"))
        .respond_with(ResponseTemplate::new(200).set_body_json(items))
        .mount(server)
        .await;
}

// ---------------------------------------------------------------------------
// The cap
// ---------------------------------------------------------------------------

/// The cap is the one thing between a typo in a results limit and the month's
/// budget, so a run without it never reaches the network.
#[tokio::test]
async fn test_run_refuses_to_start_without_a_spend_cap() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(201))
        .expect(0)
        .mount(&server)
        .await;
    for call in [
        r#"c:run("apify/instagram-profile-scraper", { usernames = { "natgeo" } })"#,
        r#"c:run("apify/instagram-profile-scraper", { usernames = { "natgeo" } }, { max_total_charge_usd = 0 })"#,
        r#"c:start("apify/instagram-profile-scraper", { usernames = { "natgeo" } }, { max_total_charge_usd = "1" })"#,
        r#"c:instagram_profiles({ "natgeo" })"#,
        r#"c:instagram_comments({ "https://www.instagram.com/p/abc/" }, 5)"#,
        r#"c:instagram_hashtag_posts({ "natgeo" }, 5)"#,
        r#"c:linkedin_profiles({ "https://www.linkedin.com/in/williamhgates" })"#,
    ] {
        let err = run_lua(&client(&server.uri(), call)).await.unwrap_err();
        assert_says(err, "max_total_charge_usd is required", call);
    }
}

/// The actor id reaches the API as `owner~name`, the cap and the timeout ride
/// the query string, and the token goes as a bearer.
#[tokio::test]
async fn test_start_addresses_the_actor_the_way_the_api_wants() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/acts/apify~instagram-profile-scraper/runs"))
        .and(header("Authorization", "Bearer tok"))
        .and(query_param("maxTotalChargeUsd", "0.5"))
        .and(query_param("timeout", "120"))
        .and(query_param("memory", "512"))
        .and(body_partial_json(json!({ "usernames": ["natgeo"] })))
        .respond_with(
            ResponseTemplate::new(201)
                .set_body_json(json!({ "data": run_json("run1", "READY", 0.0) })),
        )
        .expect(1)
        .mount(&server)
        .await;
    ok(
        &server,
        r#"
        local run = c:start("apify/instagram-profile-scraper", { usernames = { "natgeo" } },
          { max_total_charge_usd = 0.5, timeout_s = 120, memory_mb = 512 })
        assert.eq(run.id, "run1")
        assert.eq(run.status, "READY")
        assert.eq(run.terminated, false)
        assert.eq(run.dataset_id, "ds1")
        assert.eq(run.max_total_charge_usd, 0.5)
        "#,
    )
    .await;
}

#[tokio::test]
async fn test_a_rejected_token_says_so() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(401)
                .set_body_json(json!({ "error": { "type": "token-not-found" } })),
        )
        .mount(&server)
        .await;
    let err = run_lua(&client(
        &server.uri(),
        r#"c:start("apify/instagram-profile-scraper", { usernames = { "natgeo" } }, { max_total_charge_usd = 0.5 })"#,
    ))
    .await
    .unwrap_err();
    assert_says(err, "rejected the token (HTTP 401)", "401");
}

// ---------------------------------------------------------------------------
// The run
// ---------------------------------------------------------------------------

/// A run is over only when the API says a terminal status. RUNNING with a
/// dataset id already assigned is still a run in flight, and the items are
/// read only once it ends.
#[tokio::test]
async fn test_run_polls_to_a_terminal_state_then_reads_the_items() {
    let server = MockServer::start().await;
    mount_start(&server, "apify~instagram-profile-scraper", "READY").await;
    Mock::given(method("GET"))
        .and(path("/actor-runs/run1"))
        .and(query_param("waitForFinish", "5"))
        .respond_with(poll_sequence(vec![
            ("RUNNING", 0.0),
            ("RUNNING", 0.0),
            ("SUCCEEDED", 0.0023),
        ]))
        .expect(3)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/actor-runs/run1"))
        .respond_with(envelope(run_json("run1", "SUCCEEDED", 0.0023)))
        .expect(3)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/datasets/ds1/items"))
        .and(query_param("clean", "true"))
        .and(query_param("format", "json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([{ "username": "natgeo" }])))
        .expect(1)
        .mount(&server)
        .await;
    ok(
        &server,
        r#"
        local r = c:run("apify/instagram-profile-scraper", { usernames = { "natgeo" } },
          { max_total_charge_usd = 0.5, wait_s = 5, poll_s = 0, settle_s = 0 })
        assert.eq(r.status, "SUCCEEDED")
        assert.eq(r.succeeded, true)
        assert.eq(#r.items, 1)
        assert.eq(r.items[1].username, "natgeo")
        assert.eq(r.usage_total_usd, 0.0023)
        assert.eq(r.usage_cents, 1)
        assert.eq(r.charged_event_counts.profile, 1)
        assert.eq(r.finished_at, "2026-09-16T10:00:20.000Z")
        "#,
    )
    .await;
}

/// A failed run has usually both spent money and written part of its
/// dataset, so the result travels with the reason instead of being lost.
#[tokio::test]
async fn test_a_failed_run_still_reports_its_cost_and_partial_items() {
    let server = MockServer::start().await;
    mount_start(&server, "apify~instagram-scraper", "RUNNING").await;
    Mock::given(method("GET"))
        .and(path("/actor-runs/run1"))
        .respond_with(envelope(run_json("run1", "FAILED", 0.31)))
        .mount(&server)
        .await;
    mount_items(&server, json!([{ "id": "1" }, { "id": "2" }])).await;
    ok(
        &server,
        r#"
        local r, reason, partial = c:run("apify/instagram-scraper", { directUrls = { "x" } },
          { max_total_charge_usd = 1, wait_s = 5, poll_s = 0, settle_s = 0 })
        assert.eq(r, nil)
        assert.eq(reason, "run_failed")
        assert.eq(partial.status, "FAILED")
        assert.eq(partial.usage_total_usd, 0.31)
        assert.eq(partial.usage_cents, 31)
        assert.eq(#partial.items, 2)
        "#,
    )
    .await;
}

#[tokio::test]
async fn test_a_timed_out_run_is_named_as_such() {
    let server = MockServer::start().await;
    mount_start(&server, "apify~instagram-scraper", "TIMED-OUT").await;
    mount_settled(&server, "TIMED-OUT", 0.0).await;
    mount_items(&server, json!([])).await;
    ok(
        &server,
        r#"
        local r, reason, partial = c:run("apify/instagram-scraper", { directUrls = { "x" } },
          { max_total_charge_usd = 1, poll_s = 0, settle_s = 0 })
        assert.eq(r, nil)
        assert.eq(reason, "run_timed_out")
        assert.eq(partial.terminated, true)
        assert.eq(#partial.items, 0)
        "#,
    )
    .await;
}

/// When the attempt budget runs out the run is reported as unfinished, not
/// raised: the id stays valid and the caller may abort it or come back.
#[tokio::test]
async fn test_an_unfinished_run_comes_back_as_not_terminated_with_no_items_read() {
    let server = MockServer::start().await;
    mount_start(&server, "apify~instagram-scraper", "RUNNING").await;
    Mock::given(method("GET"))
        .and(path("/actor-runs/run1"))
        .respond_with(envelope(run_json("run1", "RUNNING", 0.1)))
        .expect(2)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/datasets/ds1/items"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([{ "id": "1" }])))
        .expect(0)
        .mount(&server)
        .await;
    ok(
        &server,
        r#"
        local r, reason, partial = c:run("apify/instagram-scraper", { directUrls = { "x" } },
          { max_total_charge_usd = 1, attempts = 2, poll_s = 0 })
        assert.eq(r, nil)
        assert.eq(reason, "not_terminated")
        assert.eq(partial.id, "run1")
        assert.eq(partial.terminated, false)
        assert.eq(#partial.items, 0)
        "#,
    )
    .await;
}

#[tokio::test]
async fn test_abort_posts_to_the_run() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/actor-runs/run1/abort"))
        .respond_with(envelope(run_json("run1", "ABORTING", 0.1)))
        .expect(1)
        .mount(&server)
        .await;
    ok(
        &server,
        r#"
        local run = c:abort("run1")
        assert.eq(run.status, "ABORTING")
        assert.eq(run.terminated, false)
        "#,
    )
    .await;
}

/// The bill lands after the run does: the first read after SUCCEEDED carries
/// part of the events, the rest arrive over the next seconds. The cost is
/// re-read until two consecutive reads agree, and that figure is the answer.
#[tokio::test]
async fn test_run_reads_the_cost_again_until_it_stops_moving() {
    let server = MockServer::start().await;
    mount_start(&server, "apify~instagram-profile-scraper", "RUNNING").await;
    Mock::given(method("GET"))
        .and(path("/actor-runs/run1"))
        .respond_with(poll_sequence(vec![
            ("SUCCEEDED", 0.0),
            ("SUCCEEDED", 0.0023),
            ("SUCCEEDED", 0.0046),
            ("SUCCEEDED", 0.0046),
            ("SUCCEEDED", 0.0092),
        ]))
        .expect(4)
        .mount(&server)
        .await;
    mount_items(&server, json!([{ "username": "a" }, { "username": "b" }])).await;
    ok(
        &server,
        r#"
        local r = c:run("apify/instagram-profile-scraper", { usernames = { "a", "b" } },
          { max_total_charge_usd = 0.5, poll_s = 0, settle_s = 0 })
        assert.eq(r.usage_total_usd, 0.0046)
        assert.eq(r.usage_cents, 1)
        assert.eq(#r.items, 2)
        "#,
    )
    .await;
}

/// The counts land before the figure: two reads agreeing on zero while events
/// stand against the run is not a settled bill, and the read goes on.
#[tokio::test]
async fn test_settle_waits_past_agreeing_zeros_while_events_are_charged() {
    let server = MockServer::start().await;
    mount_start(&server, "apify~instagram-profile-scraper", "RUNNING").await;
    Mock::given(method("GET"))
        .and(path("/actor-runs/run1"))
        .respond_with(poll_sequence(vec![
            ("SUCCEEDED", 0.0),
            ("SUCCEEDED", 0.0),
            ("SUCCEEDED", 0.0),
            ("SUCCEEDED", 0.0023),
            ("SUCCEEDED", 0.0023),
        ]))
        .expect(5)
        .mount(&server)
        .await;
    mount_items(&server, json!([])).await;
    ok(
        &server,
        r#"
        local r = c:run("apify/instagram-profile-scraper", { usernames = { "a" } },
          { max_total_charge_usd = 0.5, poll_s = 0, settle_s = 0 })
        assert.eq(r.usage_total_usd, 0.0023)
        "#,
    )
    .await;
}

/// A run that charged nothing is believed only once the counts have had time
/// to land: three settle reads, not the first agreeing one.
#[tokio::test]
async fn test_a_free_run_is_believed_after_three_reads() {
    let server = MockServer::start().await;
    mount_start(&server, "apify~instagram-profile-scraper", "RUNNING").await;
    Mock::given(method("GET"))
        .and(path("/actor-runs/run1"))
        .respond_with(envelope(run_json_with(
            "run1",
            "SUCCEEDED",
            0.0,
            json!({ "profile": 0 }),
        )))
        .expect(4)
        .mount(&server)
        .await;
    mount_items(&server, json!([])).await;
    ok(
        &server,
        r#"
        local r = c:run("apify/instagram-profile-scraper", { usernames = { "a" } },
          { max_total_charge_usd = 0.5, poll_s = 0, settle_s = 0 })
        assert.eq(r.usage_total_usd, 0)
        assert.eq(r.usage_cents, 0)
        "#,
    )
    .await;
}

/// A caller that cannot wait for the bill can say so; the run then answers
/// with whatever the first read carried.
#[tokio::test]
async fn test_settle_reads_can_be_turned_off() {
    let server = MockServer::start().await;
    mount_start(&server, "apify~instagram-profile-scraper", "RUNNING").await;
    Mock::given(method("GET"))
        .and(path("/actor-runs/run1"))
        .respond_with(poll_sequence(vec![
            ("SUCCEEDED", 0.0),
            ("SUCCEEDED", 0.0046),
        ]))
        .expect(1)
        .mount(&server)
        .await;
    mount_items(&server, json!([])).await;
    ok(
        &server,
        r#"
        local r = c:run("apify/instagram-profile-scraper", { usernames = { "a" } },
          { max_total_charge_usd = 0.5, poll_s = 0, settle_reads = 0 })
        assert.eq(r.usage_total_usd, 0)
        "#,
    )
    .await;
}

// ---------------------------------------------------------------------------
// The dataset
// ---------------------------------------------------------------------------

/// The dataset is read a page at a time until a short page arrives, and a
/// limit stops the walk early.
#[tokio::test]
async fn test_dataset_items_pages_by_offset_until_a_short_page() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/datasets/ds1/items"))
        .and(query_param("offset", "0"))
        .and(query_param("limit", "2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([{ "n": 1 }, { "n": 2 }])))
        .expect(2)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/datasets/ds1/items"))
        .and(query_param("offset", "2"))
        .and(query_param("limit", "2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([{ "n": 3 }])))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/datasets/ds1/items"))
        .and(query_param("offset", "0"))
        .and(query_param("limit", "1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([{ "n": 1 }])))
        .expect(1)
        .mount(&server)
        .await;
    ok(
        &server,
        r#"
        local items = c:dataset_items("ds1", { page_size = 2 })
        assert.eq(#items, 3)
        assert.eq(items[3].n, 3)
        local capped = c:dataset_items("ds1", { page_size = 2, limit = 2 })
        assert.eq(#capped, 2)
        local one = c:dataset_items("ds1", { page_size = 2, limit = 1 })
        assert.eq(#one, 1)
        "#,
    )
    .await;
}

// ---------------------------------------------------------------------------
// Typed readers
// ---------------------------------------------------------------------------

/// A follower count that arrives as a string is still a count, a profile the
/// actor could not read keeps its error beside the username, and the run
/// travels with the profiles so the cost can be written down.
#[tokio::test]
async fn test_instagram_profiles_normalise_counts_and_keep_misses() {
    let server = MockServer::start().await;
    mount_start_expecting(
        &server,
        "apify~instagram-profile-scraper",
        json!({ "usernames": ["natgeo", "no_such_account_xyz"] }),
        0.0046,
    )
    .await;
    mount_items(
        &server,
        json!([
            {
                "id": "787132",
                "username": "natgeo",
                "fullName": "National Geographic",
                "biography": "Taking our followers to the edge.",
                "externalUrl": "https://on.natgeo.com/instagram",
                "externalUrls": [{ "title": "on.natgeo.com/instagram", "url": "https://on.natgeo.com/instagram" }],
                "followersCount": "280123456",
                "followsCount": 140,
                "postsCount": 30012,
                "verified": true,
                "private": false,
                "isBusinessAccount": true,
                "businessCategoryName": "None",
                "profilePicUrlHD": "https://cdn.example/natgeo-hd.jpg",
                "profilePicUrl": "https://cdn.example/natgeo.jpg",
                "latestPosts": [
                    { "id": "1", "type": "Video", "shortCode": "abc", "url": "https://www.instagram.com/p/abc/",
                      "caption": "Photo by @someone", "likesCount": "1200", "commentsCount": 34,
                      "timestamp": "2026-09-15T12:00:00.000Z", "hashtags": ["wild"], "mentions": ["someone"] }
                ]
            },
            { "username": "no_such_account_xyz", "error": "not_found", "errorDescription": "Page not found" }
        ]),
    )
    .await;
    ok(
        &server,
        r#"
        local r = c:instagram_profiles({ "natgeo", "no_such_account_xyz" }, { max_total_charge_usd = 0.5, poll_s = 0, settle_s = 0 })
        assert.eq(#r.profiles, 2)
        local p = r.profiles[1]
        assert.eq(p.username, "natgeo")
        assert.eq(p.followers, 280123456)
        assert.eq(p.follows, 140)
        assert.eq(p.posts_count, 30012)
        assert.eq(p.verified, true)
        assert.eq(p.business, true)
        assert.eq(p.category, nil)
        assert.eq(p.profile_pic_url, "https://cdn.example/natgeo-hd.jpg")
        assert.eq(p.external_urls[1], "https://on.natgeo.com/instagram")
        assert.eq(#p.latest_posts, 1)
        assert.eq(p.latest_posts[1].likes, 1200)
        assert.eq(p.latest_posts[1].shortcode, "abc")
        assert.eq(p.provenance.provider, "apify")
        assert.eq(p.provenance.retrieved_from, "apify/instagram-profile-scraper run run1")
        assert.not_nil(p.provenance.retrieved_at)
        local miss = r.profiles[2]
        assert.eq(miss.username, "no_such_account_xyz")
        assert.eq(miss.error, "not_found")
        assert.eq(miss.followers, nil)
        assert.eq(r.run.usage_total_usd, 0.0046)
        assert.eq(r.run.usage_cents, 1)
        "#,
    )
    .await;
}

/// Comments go to the general scraper with `resultsType = comments`, and the
/// per-URL limit is what the bill multiplies by.
#[tokio::test]
async fn test_instagram_comments_ask_the_general_scraper_for_comments() {
    let server = MockServer::start().await;
    mount_start_expecting(
        &server,
        "apify~instagram-scraper",
        json!({
            "directUrls": ["https://www.instagram.com/p/abc/"],
            "resultsType": "comments",
            "resultsLimit": 5
        }),
        0.012,
    )
    .await;
    mount_items(
        &server,
        json!([
            { "id": "c1", "commentUrl": "https://www.instagram.com/p/abc/c/c1",
              "postUrl": "https://www.instagram.com/p/abc/", "text": "Where do you teach?",
              "ownerUsername": "asker", "owner": { "id": "9", "is_verified": true, "username": "asker" },
              "ownerProfilePicUrl": "https://cdn.example/asker.jpg", "likesCount": "3",
              "timestamp": "2026-09-14T08:00:00.000Z" }
        ]),
    )
    .await;
    ok(
        &server,
        r#"
        local r = c:instagram_comments({ "https://www.instagram.com/p/abc/" }, 5, { max_total_charge_usd = 0.5, poll_s = 0, settle_s = 0 })
        assert.eq(#r.comments, 1)
        assert.eq(r.comments[1].text, "Where do you teach?")
        assert.eq(r.comments[1].owner_username, "asker")
        assert.eq(r.comments[1].likes, 3)
        assert.eq(r.comments[1].replies, nil)
        assert.eq(r.comments[1].owner_id, "9")
        assert.eq(r.comments[1].owner_verified, true)
        assert.eq(r.comments[1].post_url, "https://www.instagram.com/p/abc/")
        assert.eq(r.comments[1].comment_url, "https://www.instagram.com/p/abc/c/c1")
        assert.eq(r.comments[1].provenance.provider, "apify")
        "#,
    )
    .await;
}

/// Hashtag discovery goes to the dedicated hashtag actor: the general scraper
/// accepts a `hashtags` input and answers with one post.
#[tokio::test]
async fn test_instagram_hashtag_posts_use_the_hashtag_actor() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/acts/apify~instagram-scraper/runs"))
        .respond_with(ResponseTemplate::new(201))
        .expect(0)
        .mount(&server)
        .await;
    mount_start_expecting(
        &server,
        "apify~instagram-hashtag-scraper",
        json!({ "hashtags": ["bachata"], "resultsType": "posts", "resultsLimit": 3 }),
        0.0069,
    )
    .await;
    mount_items(
        &server,
        json!([
            { "id": "1", "type": "Video", "shortCode": "xyz", "url": "https://www.instagram.com/p/xyz/",
              "caption": "Sensual bachata class #bachata", "hashtags": ["bachata"],
              "ownerUsername": "dance_school", "ownerId": "42", "ownerFullName": "A Dance School",
              "likesCount": 250, "commentsCount": 12, "videoViewCount": "9000",
              "timestamp": "2026-09-15T18:00:00.000Z", "displayUrl": "https://cdn.example/xyz.jpg",
              "inputUrl": "https://www.instagram.com/explore/tags/bachata" }
        ]),
    )
    .await;
    ok(
        &server,
        r#"
        local r = c:instagram_hashtag_posts({ "bachata" }, 3, { max_total_charge_usd = 0.5, poll_s = 0, settle_s = 0 })
        assert.eq(#r.posts, 1)
        local p = r.posts[1]
        assert.eq(p.owner_username, "dance_school")
        assert.eq(p.video_views, 9000)
        assert.eq(p.likes, 250)
        assert.eq(p.tag, "bachata")
        assert.eq(p.provenance.retrieved_from, "apify/instagram-hashtag-scraper run run1")
        "#,
    )
    .await;
}

/// LinkedIn profiles answer in the lead_provider person shape, the actor's
/// cheaper mode is the default, and an email the vendor found never climbs
/// above the status a vendor claim can earn.
#[tokio::test]
async fn test_linkedin_profiles_answer_as_lead_provider_people() {
    let server = MockServer::start().await;
    mount_start_expecting(
        &server,
        "harvestapi~linkedin-profile-scraper",
        json!({
            "queries": ["https://www.linkedin.com/in/williamhgates"],
            "profileScraperMode": "Profile details no email ($4 per 1k)"
        }),
        0.004,
    )
    .await;
    mount_items(
        &server,
        json!([
            { "id": "251749025", "publicIdentifier": "williamhgates",
              "linkedinUrl": "https://www.linkedin.com/in/williamhgates",
              "firstName": "Bill", "lastName": "Gates", "headline": "Chair, Gates Foundation",
              "about": "Sharing things I'm learning.",
              "location": { "linkedinText": "Seattle, Washington, United States" },
              "emails": ["bill@example.com"], "followerCount": 40663315, "photo": "https://cdn.example/bg.jpg",
              "currentPosition": [{ "companyName": "Gates Foundation" }],
              "experience": [{ "position": "Co-chair", "companyName": "Gates Foundation" }] }
        ]),
    )
    .await;
    ok(
        &server,
        r#"
        local r = c:linkedin_profiles({ "https://www.linkedin.com/in/williamhgates" }, { max_total_charge_usd = 0.5, poll_s = 0, settle_s = 0 })
        assert.eq(#r.people, 1)
        local p = r.people[1]
        assert.eq(p.first_name, "Bill")
        assert.eq(p.last_name, "Gates")
        assert.eq(p.full_name, "Bill Gates")
        assert.eq(p.title, "Co-chair")
        assert.eq(p.company, "Gates Foundation")
        assert.eq(p.location, "Seattle, Washington, United States")
        assert.eq(p.linkedin, "https://www.linkedin.com/in/williamhgates")
        assert.eq(p.public_identifier, "williamhgates")
        assert.eq(p.headline, "Chair, Gates Foundation")
        assert.eq(p.followers, 40663315)
        assert.eq(#p.emails, 1)
        assert.eq(p.emails[1].address, "bill@example.com")
        assert.eq(p.emails[1].email_type, "provider")
        assert.eq(p.emails[1].verification_status, "UNKNOWN")
        assert.eq(p.provenance.provider, "apify")
        "#,
    )
    .await;
}

#[tokio::test]
async fn test_linkedin_with_email_switches_the_actor_mode() {
    let server = MockServer::start().await;
    mount_start_expecting(
        &server,
        "harvestapi~linkedin-profile-scraper",
        json!({ "profileScraperMode": "Profile details + email search ($10 per 1k)" }),
        0.01,
    )
    .await;
    mount_items(&server, json!([])).await;
    ok(
        &server,
        r#"
        local r = c:linkedin_profiles({ "williamhgates" }, { max_total_charge_usd = 0.5, with_email = true, poll_s = 0, settle_s = 0 })
        assert.eq(#r.people, 0)
        assert.eq(r.run.usage_cents, 1)
        "#,
    )
    .await;
}
