//! `assay.jev` against a stubbed System One endpoint: the body that goes over
//! the wire is the one the API documents, a Choice's criteria travel as a map
//! and a Score's as an ordered array, Lua's booleans reach a Noul as the
//! strings it wants, a Noul comes back as the probability it is and not as a
//! boolean, and each way the call can fail says which one it was rather than
//! quietly handing back a judgment nobody made.

mod common;

use common::run_lua;
use serde_json::{Value, json};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

fn client(uri: &str, body: &str) -> String {
    format!(
        "local jev = require(\"assay.jev\")\n\
         local c = jev.client({{ api_key = \"k1\", base_url = \"{uri}\", retry_delay_s = 0 }})\n{body}"
    )
}

async fn ok(server: &MockServer, body: &str) {
    run_lua(&client(&server.uri(), body)).await.unwrap();
}

async fn err(server: &MockServer, body: &str) -> String {
    run_lua(&client(&server.uri(), body))
        .await
        .unwrap_err()
        .to_string()
}

fn assert_says(text: &str, want: &str, ctx: &str) {
    assert!(text.contains(want), "{ctx} gave {text}, wanted {want}");
}

/// The Choice answer the docs describe: the caller's own option names come
/// back verbatim, in `choice` and as the keys of `probabilities`.
fn choice_answer(choice: &str, confidence: f64) -> Value {
    json!({
        "type": "choice",
        "choice": choice,
        "probabilities": { "needs_reply": 0.71, "needs_action": 0.2, "fyi": 0.09 },
        "confidence": confidence,
    })
}

fn body_of(answers: Value) -> Value {
    json!({
        "model": "jev-1.0",
        "answers": answers,
        "usage": { "input_tokens": 120, "output_tokens": 4 },
    })
}

async fn mount(server: &MockServer, answers: Value) {
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body_of(answers)))
        .mount(server)
        .await;
}

async fn mount_status(server: &MockServer, status: u16, body: &str) {
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(ResponseTemplate::new(status).set_body_string(body))
        .mount(server)
        .await;
}

/// The body of the one request the server was sent.
async fn posted(server: &MockServer) -> Value {
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1, "expected exactly one request");
    serde_json::from_slice(&requests[0].body).expect("request body was not JSON")
}

#[tokio::test]
async fn the_module_offers_a_client_and_the_three_primitives() {
    run_lua(
        r#"
        local jev = require("assay.jev")
        assert.not_nil(jev.client)
        assert.not_nil(jev.choice)
        assert.not_nil(jev.score)
        assert.not_nil(jev.noul)
        local c = jev.client({ api_key = "k1" })
        assert.eq(type(c.system_one), "function")
        "#,
    )
    .await
    .unwrap();
}

// ---------------------------------------------------------------------------
// What goes over the wire.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_choice_travels_as_state_model_and_a_map_of_criteria() {
    let server = MockServer::start().await;
    mount(&server, json!({ "bucket": choice_answer("fyi", 0.9) })).await;

    ok(
        &server,
        r#"
        local answers = c:system_one("Dear team, can we meet Thursday?", {
          bucket = jev.choice("Which does the reader owe it?", {
            needs_reply = "They are waiting on an answer",
            fyi = "Nothing is owed",
          }),
        })
        assert.eq(answers.bucket.choice, "fyi")
        "#,
    )
    .await;

    let body = posted(&server).await;
    assert_eq!(body["state"], json!("Dear team, can we meet Thursday?"));
    assert_eq!(body["model"], json!("jev-latest"));
    assert_eq!(
        body["questions"]["bucket"],
        json!({
            "type": "choice",
            "instructions": "Which does the reader owe it?",
            "criteria": {
                "needs_reply": "They are waiting on an answer",
                "fyi": "Nothing is owed",
            },
        })
    );
    // Exactly those three keys and nothing else: an extra field is a field the
    // API was not asked about.
    let sent = body.as_object().unwrap();
    assert_eq!(sent.len(), 3, "sent {sent:?}");
}

#[tokio::test]
async fn the_bearer_is_on_the_wire() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .and(header("authorization", "Bearer k1"))
        .and(header("content-type", "application/json"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(body_of(json!({ "q": choice_answer("fyi", 1.0) }))),
        )
        .expect(1)
        .mount(&server)
        .await;

    ok(
        &server,
        r#"c:system_one("x", { q = jev.choice("pick", { fyi = "nothing owed" }) })"#,
    )
    .await;
}

#[tokio::test]
async fn opts_model_overrides_the_client_default() {
    let server = MockServer::start().await;
    mount(&server, json!({ "q": choice_answer("fyi", 1.0) })).await;
    ok(
        &server,
        r#"c:system_one("x", { q = jev.choice("pick", { fyi = "nothing owed" }) }, { model = "jev-2" })"#,
    )
    .await;
    assert_eq!(posted(&server).await["model"], json!("jev-2"));
}

/// The state is whatever the caller had. A record goes as a record, not as a
/// string somebody pasted a table into.
#[tokio::test]
async fn a_table_state_travels_as_an_object() {
    let server = MockServer::start().await;
    mount(&server, json!({ "q": choice_answer("fyi", 1.0) })).await;
    ok(
        &server,
        r#"c:system_one({ from = "jo@example.test", subject = "Re: intro" },
             { q = jev.choice("pick", { fyi = "nothing owed" }) })"#,
    )
    .await;
    assert_eq!(
        posted(&server).await["state"],
        json!({ "from": "jo@example.test", "subject": "Re: intro" })
    );
}

/// A Score's criteria is an ordered array, because the order *is* the scale.
/// Encoded as an object, the ladder has no rungs.
#[tokio::test]
async fn a_score_travels_as_an_ordered_array() {
    let server = MockServer::start().await;
    mount(
        &server,
        json!({ "heat": {
            "type": "score",
            "score": 2,
            "probabilities": { "0": 0.1, "1": 0.2, "2": 0.7 },
            "legend": ["cold", "warm", "hot"],
            "confidence": 0.82,
        }}),
    )
    .await;

    ok(
        &server,
        r#"
        local answers = c:system_one("They asked for pricing today", {
          heat = jev.score("How warm is this lead?", { "cold", "warm", "hot" }),
        })
        assert.eq(answers.heat.score, 2)
        assert.eq(answers.heat.legend[3], "hot")
        assert.eq(answers.heat.probabilities["2"], 0.7)
        "#,
    )
    .await;

    assert_eq!(
        posted(&server).await["questions"]["heat"],
        json!({
            "type": "score",
            "instructions": "How warm is this lead?",
            "criteria": ["cold", "warm", "hot"],
        })
    );
}

/// Lua spells booleans as booleans. `{ [true] = ... }` is what a Lua author
/// writes and `{"true": ...}` is what the API takes, so both spellings work
/// and one thing goes out.
#[tokio::test]
async fn a_noul_normalises_lua_booleans_to_the_strings_the_api_wants() {
    let server = MockServer::start().await;
    mount(&server, json!({ "paid": { "type": "noul", "noul": 0.93 } })).await;

    ok(
        &server,
        r#"
        local answers = c:system_one("Invoice 44 settled on the 3rd", {
          paid = jev.noul("Has this invoice been paid?", {
            [true] = "The money arrived",
            [false] = "It has not",
          }),
        })
        assert.eq(answers.paid.noul, 0.93)
        -- The answer *is* the probability, so no confidence sits beside it.
        assert.eq(answers.paid.confidence, nil)
        "#,
    )
    .await;

    assert_eq!(
        posted(&server).await["questions"]["paid"],
        json!({
            "type": "noul",
            "instructions": "Has this invoice been paid?",
            "criteria": { "true": "The money arrived", "false": "It has not" },
        })
    );
}

#[tokio::test]
async fn a_noul_without_criteria_sends_none() {
    let server = MockServer::start().await;
    mount(&server, json!({ "q": { "type": "noul", "noul": 0.04 } })).await;
    ok(
        &server,
        r#"
        local answers = c:system_one("x", { q = jev.noul("Is this urgent?") })
        assert.eq(answers.q.noul, 0.04)
        "#,
    )
    .await;
    let sent = posted(&server).await;
    assert_eq!(sent["questions"]["q"]["criteria"], Value::Null);
    assert_eq!(sent["questions"]["q"]["type"], json!("noul"));
}

/// The answers come back under the caller's own keys, and the token cost and
/// the model that actually answered come back beside them.
#[tokio::test]
async fn the_answers_keep_the_callers_keys_and_the_usage_comes_back_beside_them() {
    let server = MockServer::start().await;
    mount(
        &server,
        json!({
            "bucket": choice_answer("needs_reply", 0.88),
            "urgent": { "type": "noul", "noul": 0.93 },
        }),
    )
    .await;

    ok(
        &server,
        r#"
        local answers, info = c:system_one("Please confirm by Friday", {
          bucket = jev.choice("Which bucket?", { needs_reply = "waiting", fyi = "nothing owed" }),
          urgent = jev.noul("Is there a deadline?"),
        })
        assert.eq(answers.bucket.choice, "needs_reply")
        assert.eq(answers.bucket.confidence, 0.88)
        assert.eq(answers.bucket.probabilities.needs_reply, 0.71)
        assert.eq(answers.urgent.noul, 0.93)
        assert.eq(info.model, "jev-1.0")
        assert.eq(info.usage.input_tokens, 120)
        assert.eq(info.usage.output_tokens, 4)
        "#,
    )
    .await;
}

// ---------------------------------------------------------------------------
// Every way it can fail says which way it was.
// ---------------------------------------------------------------------------

/// A Noul answer is the probability the statement holds, on 0 to 1. A boolean,
/// a number off that scale or a string carrying one is not a reading a caller
/// can branch on, and coercing it here would hand back a judgment nobody made.
#[tokio::test]
async fn a_noul_answer_that_is_not_a_probability_is_refused() {
    for bad in [
        json!(true),
        json!(false),
        json!(1.4),
        json!(-0.1),
        json!("0.9"),
        Value::Null,
    ] {
        let server = MockServer::start().await;
        mount(&server, json!({ "q": { "type": "noul", "noul": bad } })).await;
        let text = err(
            &server,
            r#"c:system_one("x", { q = jev.noul("Is this urgent?") })"#,
        )
        .await;
        assert_says(&text, "jev: noul answer out of range", "an off-scale noul");
    }
}

#[tokio::test]
async fn each_status_says_what_it_was() {
    for (status, body, want) in [
        (401u16, "{}", "jev: unauthorized"),
        (403, "nope", "HTTP 403"),
        (404, "gone", "HTTP 404"),
        (500, "boom", "HTTP 500: boom"),
    ] {
        let server = MockServer::start().await;
        mount_status(&server, status, body).await;
        let text = err(
            &server,
            r#"c:system_one("x", { q = jev.choice("pick", { fyi = "nothing owed" }) })"#,
        )
        .await;
        assert_says(&text, want, &format!("HTTP {status}"));
    }
}

/// The 422 envelope is not published, so each shape an API of this kind writes
/// is read in turn and the raw body stands when none of them fits. A message
/// invented here would be worse than the one the server sent.
#[tokio::test]
async fn a_422_reports_the_field_the_api_named_whatever_shape_it_used() {
    for (body, want) in [
        (
            r#"{"error":{"message":"criteria must have at least 2 entries"}}"#,
            "jev: criteria must have at least 2 entries",
        ),
        (
            r#"{"detail":"questions: at most 255 options"}"#,
            "jev: questions: at most 255 options",
        ),
        (
            r#"{"detail":[{"loc":["body","questions","bucket"],"msg":"field required"}]}"#,
            "jev: body.questions.bucket: field required",
        ),
        (
            r#"{"message":"state is too long"}"#,
            "jev: state is too long",
        ),
        ("not json at all", "jev: not json at all"),
    ] {
        let server = MockServer::start().await;
        mount_status(&server, 422, body).await;
        let text = err(
            &server,
            r#"c:system_one("x", { q = jev.choice("pick", { fyi = "nothing owed" }) })"#,
        )
        .await;
        assert_says(&text, want, body);
    }
}

#[tokio::test]
async fn a_body_that_is_not_json_is_not_an_answer() {
    let server = MockServer::start().await;
    mount_status(&server, 200, "<html>gateway</html>").await;
    let text = err(
        &server,
        r#"c:system_one("x", { q = jev.choice("pick", { fyi = "nothing owed" }) })"#,
    )
    .await;
    assert_says(&text, "returned unparseable JSON", "html body");
}

/// A 200 with no `answers` is not an empty set of judgments — it is a shape
/// this module was not written against, and the caller is about to branch.
#[tokio::test]
async fn a_200_without_answers_raises_rather_than_handing_back_nothing() {
    let server = MockServer::start().await;
    mount_status(&server, 200, r#"{"model":"jev-1.0","usage":{}}"#).await;
    let text = err(
        &server,
        r#"c:system_one("x", { q = jev.choice("pick", { fyi = "nothing owed" }) })"#,
    )
    .await;
    assert_says(&text, "returned no answers", "no answers key");
}

// ---------------------------------------------------------------------------
// Retries.
// ---------------------------------------------------------------------------

/// Answers with the next status in the list, then keeps answering with the
/// last one.
struct Sequence {
    statuses: Vec<u16>,
    calls: std::sync::atomic::AtomicUsize,
}

impl Respond for Sequence {
    fn respond(&self, _: &Request) -> ResponseTemplate {
        let n = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let status = self.statuses[n.min(self.statuses.len() - 1)];
        if status == 200 {
            ResponseTemplate::new(200)
                .set_body_json(body_of(json!({ "q": choice_answer("fyi", 0.95) })))
        } else {
            ResponseTemplate::new(status).set_body_string("slow down")
        }
    }
}

async fn mount_sequence(server: &MockServer, statuses: Vec<u16>) {
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(Sequence {
            statuses,
            calls: std::sync::atomic::AtomicUsize::new(0),
        })
        .mount(server)
        .await;
}

#[tokio::test]
async fn a_429_is_tried_again_and_the_second_answer_stands() {
    let server = MockServer::start().await;
    mount_sequence(&server, vec![429, 200]).await;
    ok(
        &server,
        r#"
        local answers = c:system_one("x", { q = jev.choice("pick", { fyi = "nothing owed" }) })
        assert.eq(answers.q.choice, "fyi")
        "#,
    )
    .await;
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
}

#[tokio::test]
async fn three_rate_limits_in_a_row_give_up_and_say_so() {
    let server = MockServer::start().await;
    mount_sequence(&server, vec![429]).await;
    let text = err(
        &server,
        r#"c:system_one("x", { q = jev.choice("pick", { fyi = "nothing owed" }) })"#,
    )
    .await;
    assert_says(
        &text,
        "jev: rate limited (HTTP 429) after 3 attempts",
        "429 x3",
    );
    assert_eq!(server.received_requests().await.unwrap().len(), 3);
}

#[tokio::test]
async fn an_overloaded_service_is_tried_again_and_then_named() {
    let server = MockServer::start().await;
    mount_sequence(&server, vec![529]).await;
    let text = err(
        &server,
        r#"c:system_one("x", { q = jev.choice("pick", { fyi = "nothing owed" }) })"#,
    )
    .await;
    assert_says(
        &text,
        "jev: overloaded (HTTP 529) after 3 attempts",
        "529 x3",
    );
    assert_eq!(server.received_requests().await.unwrap().len(), 3);
}

#[tokio::test]
async fn the_attempt_budget_is_the_callers_to_set() {
    let server = MockServer::start().await;
    mount_sequence(&server, vec![429]).await;
    let text = err(
        &server,
        r#"c:system_one("x", { q = jev.choice("pick", { fyi = "nothing owed" }) }, { attempts = 1 })"#,
    )
    .await;
    assert_says(&text, "after 1 attempts", "attempts = 1");
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

/// Nothing else is worth repeating: a 500 is not a different answer a second
/// later, and retrying one triples the wait before the caller is told.
#[tokio::test]
async fn a_500_is_not_retried() {
    let server = MockServer::start().await;
    mount_sequence(&server, vec![500]).await;
    err(
        &server,
        r#"c:system_one("x", { q = jev.choice("pick", { fyi = "nothing owed" }) })"#,
    )
    .await;
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

// ---------------------------------------------------------------------------
// What is refused before any request goes out.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_client_with_no_key_is_refused_before_it_reaches_the_network() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;

    let text = run_lua(&format!(
        r#"
        local jev = require("assay.jev")
        local c = jev.client({{ api_key = "", base_url = "{}" }})
        "#,
        server.uri()
    ))
    .await
    .unwrap_err()
    .to_string();
    assert_says(&text, "jev: api_key required", "empty key");
}

#[tokio::test]
async fn a_call_with_nothing_to_ask_is_refused() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;

    for (call, want) in [
        (r#"c:system_one("x", {})"#, "needs at least one question"),
        (
            r#"c:system_one("x", "not a table")"#,
            "needs at least one question",
        ),
        (
            r#"c:system_one(nil, { q = jev.noul("is it?") })"#,
            "needs state",
        ),
    ] {
        let text = err(&server, call).await;
        assert_says(&text, want, call);
    }
}

/// The builders refuse what the API would refuse anyway, so the error names
/// the mistake instead of arriving as a 422 about a field.
#[tokio::test]
async fn a_question_the_api_would_refuse_is_refused_here_first() {
    for (build, want) in [
        (
            r#"jev.choice("pick", { "a", "b" })"#,
            "option names must be non-empty strings",
        ),
        (r#"jev.choice("pick", {})"#, "at least one option"),
        (r#"jev.choice("pick", "nope")"#, "criteria must be a table"),
        (r#"jev.choice("", { fyi = "x" })"#, "needs instructions"),
        (
            r#"jev.score("how hot", { "only one" })"#,
            "between 2 and 10 levels",
        ),
        (
            r#"jev.score("how hot", { a = "x", b = "y" })"#,
            "must be an ordered list",
        ),
        (
            r#"jev.score("how hot", { "1","2","3","4","5","6","7","8","9","10","11" })"#,
            "between 2 and 10 levels",
        ),
        (
            r#"jev.noul("paid?", { maybe = "x" })"#,
            "keys must be true and false",
        ),
        (r#"jev.noul("paid?", 7)"#, "criteria must be a table"),
    ] {
        let text = run_lua(&format!("local jev = require(\"assay.jev\")\n{build}"))
            .await
            .unwrap_err()
            .to_string();
        assert_says(&text, want, build);
    }
}

/// `env.set` writes the real process environment and cargo runs these tests as
/// threads in one process, so the env fallback is proved once, in Lua, and the
/// variable is cleared before the script ends.
#[tokio::test]
async fn the_key_falls_back_to_the_environment() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .and(header("authorization", "Bearer from-env"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(body_of(json!({ "q": choice_answer("fyi", 1.0) }))),
        )
        .expect(1)
        .mount(&server)
        .await;

    run_lua(&format!(
        r#"
        local jev = require("assay.jev")
        env.set("TYPESAFE_API_KEY", "from-env")
        local ok, err = pcall(function()
          local c = jev.client({{ base_url = "{}" }})
          local answers = c:system_one("x", {{ q = jev.choice("pick", {{ fyi = "nothing owed" }}) }})
          assert.eq(answers.q.choice, "fyi")
        end)
        env.set("TYPESAFE_API_KEY", "")
        assert.eq(ok, true, tostring(err))
        "#,
        server.uri()
    ))
    .await
    .unwrap();
}

/// A trailing slash on the base URL is a typo, not a different endpoint.
#[tokio::test]
async fn a_trailing_slash_on_the_base_url_does_not_double_up() {
    let server = MockServer::start().await;
    mount(&server, json!({ "q": choice_answer("fyi", 1.0) })).await;
    run_lua(&format!(
        r#"
        local jev = require("assay.jev")
        local c = jev.client({{ api_key = "k1", base_url = "{}///" }})
        c:system_one("x", {{ q = jev.choice("pick", {{ fyi = "nothing owed" }}) }})
        "#,
        server.uri()
    ))
    .await
    .unwrap();
}

// ---------------------------------------------------------------------------
// The live smoke test. It runs only with ASSAY_TEST_TYPESAFE_API_KEY set and
// skips otherwise, so the suite stays green everywhere; with a key it proves
// the half a stub cannot — that the endpoint still answers in the shape the
// reader was written against.
// ---------------------------------------------------------------------------

/// Gated on its own name, not on `TYPESAFE_API_KEY`, and deliberately.
/// `env.set` writes the real process environment, cargo runs these tests as
/// threads in one process, and `the_key_falls_back_to_the_environment` holds
/// that name for a few milliseconds. Read here, a stub fixture could arm a
/// real HTTPS call to the vendor carrying a bogus key — a flake, and the live
/// CI call this suite exists not to make. The repo gates its other live tests
/// the same way (`ASSAY_TEST_POSTGRES_URL`): on a name no test writes.
fn key_or_skip() -> Option<String> {
    match std::env::var("ASSAY_TEST_TYPESAFE_API_KEY")
        .ok()
        .filter(|v| !v.trim().is_empty())
    {
        Some(v) => Some(v),
        None => {
            eprintln!("skipped: ASSAY_TEST_TYPESAFE_API_KEY not set");
            None
        }
    }
}

#[tokio::test]
async fn the_live_endpoint_answers_in_the_shape_the_reader_expects() {
    let Some(key) = key_or_skip() else {
        return;
    };
    run_lua(&format!(
        r#"
        local jev = require("assay.jev")
        local c = jev.client({{ api_key = "{key}" }})
        local answers, info = c:system_one(
          {{ from = "dana@example.test", subject = "Re: pricing", body = "Can you send the quote by Friday?" }},
          {{
            bucket = jev.choice("Which does the reader owe this email?", {{
              needs_reply = "They are waiting on an answer",
              needs_action = "Something must be done, by a date or on request",
              fyi = "Nothing is owed",
            }}),
            heat = jev.score("How urgent is it?", {{ "whenever", "this week", "today" }}),
            deadline = jev.noul("Does it name a deadline?"),
          }})
        assert.not_nil(answers.bucket.choice)
        assert.eq(type(answers.bucket.confidence), "number")
        assert.eq(type(answers.bucket.probabilities), "table")
        assert.eq(type(answers.heat.score), "number")
        assert.eq(type(answers.deadline.noul), "number")
        assert.eq(answers.deadline.noul >= 0 and answers.deadline.noul <= 1, true,
          "a noul is a probability on 0..1, got " .. tostring(answers.deadline.noul))
        assert.eq(type(info.usage.input_tokens), "number")
        -- `print`, not `log.info`: nothing installs a tracing subscriber for
        -- these tests, so a logged line is dropped and the reader of a
        -- `--nocapture` run sees nothing of what the endpoint actually said.
        print("jev smoke: " .. answers.bucket.choice .. " at " .. tostring(answers.bucket.confidence)
          .. ", urgency " .. tostring(answers.heat.score) .. ", deadline " .. tostring(answers.deadline.noul)
          .. ", model " .. tostring(info.model))
        "#
    ))
    .await
    .unwrap();
}
