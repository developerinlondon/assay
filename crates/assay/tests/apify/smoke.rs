//! The live smoke test for `assay.apify`. It runs only when `APIFY_TOKEN` is
//! set and skips otherwise, so the suite stays green everywhere; with a token
//! it proves the half a fixture cannot — that the profile actor still answers
//! in the shape the reader was written against, and that the run reports a
//! cost. Capped at a few cents per run.

use super::common::run_lua;

fn token_or_skip() -> Option<String> {
    match std::env::var("APIFY_TOKEN")
        .ok()
        .filter(|v| !v.trim().is_empty())
    {
        Some(v) => Some(v),
        None => {
            eprintln!("skipped: APIFY_TOKEN not set");
            None
        }
    }
}

/// The profile read is a public publisher's account: a smoke test must not
/// spend credits reading a private individual, or put one in a CI log.
#[tokio::test]
async fn instagram_profiles_answer_in_the_shape_the_reader_expects() {
    let Some(token) = token_or_skip() else {
        return;
    };
    run_lua(&format!(
        r#"
        local apify = require("assay.apify")
        local c = apify.client({{ token = "{token}" }})
        local r, reason = c:instagram_profiles({{ "natgeo" }}, {{ max_total_charge_usd = 0.05, wait_s = 5 }})
        assert.not_nil(r, "run did not succeed: " .. tostring(reason))
        assert.eq(#r.profiles, 1)
        local p = r.profiles[1]
        assert.eq(p.error, nil)
        assert.eq(p.username, "natgeo")
        assert.eq(type(p.followers), "number")
        assert.gt(p.followers, 1000000)
        assert.eq(type(p.latest_posts), "table")
        assert.eq(p.provenance.provider, "apify")
        assert.eq(r.run.succeeded, true)
        assert.eq(type(r.run.usage_total_usd), "number")
        assert.not_nil(r.run.usage_cents)
        log.info("apify smoke: " .. p.username .. " " .. p.followers .. " followers, run " .. r.run.id .. " cost " .. r.run.usage_total_usd)
    "#
    ))
    .await
    .unwrap();
}
