//! The live smoke tests for the paid lead providers.
//!
//! These run only when a real key is present in the environment, the way the
//! rest of the suite gates on pgvector or a real binary. Without one they skip
//! rather than fail, so the suite stays green on every machine and in CI — and
//! the moment a key exists, `CONTACTOUT_TOKEN=… cargo test` closes the last
//! line of T1.4's Done with no code to write.
//!
//! What they check is the half a fixture cannot: that the shapes the adapters
//! were written against are the shapes the vendors actually send. Field names
//! here came from published docs, and one of them was already wrong once —
//! BetterContact's verdict field — in a way no fixture could have caught,
//! because the fixture carried the same mistake.

#[path = "../common/mod.rs"]
mod common;

use common::run_lua;

/// A gate that approves everything and records what was spent, so a smoke run
/// also proves the metering path is wired.
const GATE: &str = r#"
local lp = require("assay.lead_provider")
local spent = {}
local gate = lp.gate({
  approve = function(op, cents) spent[#spent + 1] = op .. ":" .. cents; return true end,
  meter = function() end,
})
"#;

/// The key, or None with a note saying which run was skipped.
fn key_or_skip(name: &str) -> Option<String> {
    match std::env::var(name).ok().filter(|v| !v.trim().is_empty()) {
        Some(v) => Some(v),
        None => {
            eprintln!("skipped: {name} not set");
            None
        }
    }
}

/// The person these look up is deliberately a public figure at a public
/// company: a smoke test must not spend credits resolving a real prospect, and
/// must not put a private individual's address in a CI log.
const SUBJECT: &str = r#"{ first_name = "Satya", last_name = "Nadella", company = "Microsoft",
  company_domain = "microsoft.com" }"#;

#[tokio::test]
async fn contactout_answers_in_the_shape_the_adapter_expects() {
    let Some(token) = key_or_skip("CONTACTOUT_TOKEN") else { return };
    run_lua(&format!(
        r#"
        {GATE}
        local co = require("assay.contactout")
        local c = co.client(gate, {{ token = "{token}" }})

        local p = c:find_person({SUBJECT})
        -- A miss is a legitimate answer from a real API; what must not happen
        -- is a shape the adapter cannot read.
        if p ~= nil then
          assert.eq(type(p.provenance), "table")
          assert.eq(p.provenance.provider, "contactout")
          assert.not_nil(p.provenance.retrieved_at)
          assert.eq(type(p.emails), "table")
          for _, e in ipairs(p.emails) do
            assert.not_nil(e.address)
            assert.contains(e.address, "@")
            -- A vendor's assertion is not a delivery, whatever it claims.
            assert.eq(e.verification_status ~= "VERIFIED", true)
          end
        end
        assert.eq(#spent > 0, true)
        log.info("contactout smoke: " .. (p and (p.full_name or "matched") or "no match"))
    "#
    ))
    .await
    .unwrap();
}

#[tokio::test]
async fn bettercontact_answers_in_the_shape_the_adapter_expects() {
    let Some(api_key) = key_or_skip("BETTERCONTACT_API_KEY") else { return };
    run_lua(&format!(
        r#"
        {GATE}
        local bc = require("assay.bettercontact")
        local c = bc.client(gate, {{ api_key = "{api_key}" }})

        local id = c:submit({{ {SUBJECT} }})
        assert.not_nil(id)

        -- Only `terminated` means finished; a poll answers 202 with no data
        -- while the run is still going, so this waits on the status field.
        local run = c:await(id, {{ poll_ms = 3000, attempts = 40 }})
        assert.eq(type(run.status), "string")
        log.info("bettercontact smoke: status=" .. run.status ..
          " credits_left=" .. tostring(run.credits_left))

        if run.terminated then
          for _, p in ipairs(run.people) do
            assert.eq(p.provenance.provider, "bettercontact")
            for _, e in ipairs(p.emails) do
              -- The field the adapter reads is contact_email_address_status.
              -- If the vendor renamed it, vendor_status arrives nil here and
              -- this is the test that says so.
              assert.not_nil(e.vendor_status)
              assert.eq(e.verification_status ~= "VERIFIED", true)
            end
          end
        end
        assert.eq(#spent > 0, true)
    "#
    ))
    .await
    .unwrap();
}

/// The gate must hold against the real API, not only a mock — this is the one
/// test proving a decline stops a call that would have cost money.
#[tokio::test]
async fn a_declined_budget_stops_a_real_paid_call() {
    let Some(token) = key_or_skip("CONTACTOUT_TOKEN") else { return };
    run_lua(&format!(
        r#"
        local lp = require("assay.lead_provider")
        local gate = lp.gate({{
          approve = function() return false, "smoke_test_declines" end,
          meter = function() error("metered a call the budget refused") end,
        }})
        local co = require("assay.contactout")
        local c = co.client(gate, {{ token = "{token}" }})
        local p, reason = c:find_person({SUBJECT}, {{ cents = 100 }})
        assert.eq(p, nil)
        assert.eq(reason, "smoke_test_declines")
    "#
    ))
    .await
    .unwrap();
}
