#[path = "../common/mod.rs"]
mod common;

use common::run_lua;

/// The gate exists so a paid lookup cannot be reached by forgetting an
/// argument. Every way of not supplying a budget has to fail loudly.
#[tokio::test]
async fn test_a_gate_cannot_be_built_without_a_usable_budget() {
    for (args, want) in [
        ("", "budget context is required"),
        ("nil", "budget context is required"),
        ("\"not a table\"", "budget context is required"),
        ("{}", "approve(op, cents) and meter"),
        ("{ approve = function() return true end }", "approve(op, cents) and meter"),
        ("{ meter = function() end }", "approve(op, cents) and meter"),
    ] {
        let err = run_lua(&format!(
            r#"
            local lp = require("assay.lead_provider")
            lp.gate({args})
        "#
        ))
        .await
        .unwrap_err()
        .to_string();
        assert!(err.contains(want), "gate({args}) gave {err}, wanted {want}");
    }
}

#[tokio::test]
async fn test_an_approved_call_runs_and_is_metered_once() {
    run_lua(
        r#"
        local lp = require("assay.lead_provider")
        local asked, metered, ran = {}, {}, 0
        local gate = lp.gate({
          approve = function(op, cents) asked[#asked + 1] = op .. ":" .. cents; return true end,
          meter = function(op, cents, meta) metered[#metered + 1] = { op, cents, meta } end,
        })
        local out = gate:paid("resolve_email", 7, function() ran = ran + 1; return "found" end)
        assert.eq(out, "found")
        assert.eq(ran, 1)
        assert.eq(asked[1], "resolve_email:7")
        assert.eq(#metered, 1)
        assert.eq(metered[1][1], "resolve_email")
        assert.eq(metered[1][2], 7)
        assert.eq(metered[1][3].operation, "resolve_email")
        assert.not_nil(metered[1][3].at)
    "#,
    )
    .await
    .unwrap();
}

/// A declined budget must not call the provider at all — that is the
/// difference between a gate and an audit log.
#[tokio::test]
async fn test_a_declined_call_never_reaches_the_provider_or_the_ledger() {
    run_lua(
        r#"
        local lp = require("assay.lead_provider")
        local ran, metered = 0, 0
        local gate = lp.gate({
          approve = function() return false, "over_daily_cap" end,
          meter = function() metered = metered + 1 end,
        })
        local out, reason = gate:paid("find_person", 25, function() ran = ran + 1; return "x" end)
        assert.eq(out, nil)
        assert.eq(reason, "over_daily_cap")
        assert.eq(ran, 0)
        assert.eq(metered, 0)
    "#,
    )
    .await
    .unwrap();
}

/// The ledger answers "what did this cost". A call that raised bought nothing,
/// so metering it would inflate cost-per-qualified-lead with spend that never
/// happened.
#[tokio::test]
async fn test_a_failed_call_is_not_metered() {
    run_lua(
        r#"
        local lp = require("assay.lead_provider")
        local metered = 0
        local gate = lp.gate({
          approve = function() return true end,
          meter = function() metered = metered + 1 end,
        })
        local ok = pcall(function()
          gate:paid("find_person", 25, function() error("provider exploded") end)
        end)
        assert.eq(ok, false)
        assert.eq(metered, 0)
    "#,
    )
    .await
    .unwrap();
}

/// Spend is attributed per operation, so an adapter inventing its own name
/// would make the spend ledger unqueryable.
#[tokio::test]
async fn test_operations_and_costs_are_checked_before_anything_is_spent() {
    run_lua(
        r#"
        local lp = require("assay.lead_provider")
        local gate = lp.gate({ approve = function() return true end, meter = function() end })
        local noop = function() return true end

        local ok, err = pcall(function() gate:paid("scrape_everything", 1, noop) end)
        assert.eq(ok, false)
        assert.contains(tostring(err), "unknown operation")

        for _, bad in ipairs({ -1, "free" }) do
          local ok2, err2 = pcall(function() gate:paid("find_person", bad, noop) end)
          assert.eq(ok2, false)
          assert.contains(tostring(err2), "non-negative cost")
        end

        for _, op in ipairs(lp.OPERATIONS) do
          assert.eq(gate:paid(op, 0, noop), true)
        end
    "#,
    )
    .await
    .unwrap();
}

/// Free and paid sources must be indistinguishable downstream, so the
/// provenance stamp is one shape regardless of who produced the fact.
#[tokio::test]
async fn test_every_record_carries_provenance_and_absent_fields_stay_nil() {
    run_lua(
        r#"
        local lp = require("assay.lead_provider")
        local p = lp.person("contactout", "https://api.example/x", {
          first_name = "Jonathan", last_name = "Church", domain = "cheaney.co.uk",
        })
        assert.eq(p.first_name, "Jonathan")
        assert.eq(p.title, nil)
        assert.eq(#p.emails, 0)
        assert.eq(p.provenance.provider, "contactout")
        assert.eq(p.provenance.retrieved_from, "https://api.example/x")
        assert.not_nil(p.provenance.retrieved_at)

        local e = lp.email("bettercontact", "https://api.example/y", { address = "a@b.com" })
        assert.eq(e.address, "a@b.com")
        assert.eq(e.email_type, "provider")
        assert.eq(e.verification_status, "UNKNOWN")

        local free = lp.provenance("registry:gleif", "https://api.gleif.org/x")
        assert.eq(free.provider, "registry:gleif")
        assert.not_nil(free.retrieved_at)
    "#,
    )
    .await
    .unwrap();
}

/// VERIFIED is earned by a delivery, never by a vendor's assertion, so an
/// adapter passing it through must not be able to launder it into the record.
#[tokio::test]
async fn test_an_unproven_email_defaults_to_unknown_not_verified() {
    run_lua(
        r#"
        local lp = require("assay.lead_provider")
        assert.eq(lp.email("x", "y", {}).verification_status, "UNKNOWN")
        assert.eq(lp.email("x", "y", { verification_status = "PROBABLE" }).verification_status,
          "PROBABLE")
    "#,
    )
    .await
    .unwrap();
}
