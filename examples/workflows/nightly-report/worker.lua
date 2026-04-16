-- nightly-report — cron + side_effect + child workflows in one file.
-- Run: assay run worker.lua  (with `assay serve` running on :8080)

local workflow = require("assay.workflow")
workflow.connect(env.get("ASSAY_ENGINE_URL") or "http://localhost:8080")

-- The parent workflow: kicked off by the cron schedule.
workflow.define("NightlyReport", function(ctx, input)
    -- side_effect lets us pull a fresh report ID without breaking
    -- determinism — the value is captured in the event log on the first
    -- replay and returned from cache thereafter, even if a worker crashes
    -- between this call and the next step.
    local report = ctx:side_effect("issue_report_id", function()
        return "rep-" .. tostring(os.time()) .. "-" .. tostring(math.random(10000, 99999))
    end)

    local scan = ctx:execute_activity("scan_anomalies", {
        region = input.region,
        report_id = report,
    })

    -- Spawn one HandleAnomaly child per anomaly. Each child workflow_id
    -- is deterministic (parent report id + anomaly index), so a replay
    -- finds the existing child instead of starting a new one.
    for i, anomaly in ipairs(scan.anomalies) do
        ctx:start_child_workflow("HandleAnomaly", {
            workflow_id = report .. "-anomaly-" .. tostring(i),
            input = { anomaly = anomaly, report_id = report },
        })
    end

    return {
        report_id = report,
        region = input.region,
        anomalies_handled = #scan.anomalies,
    }
end)

-- The child workflow: handles a single anomaly.
workflow.define("HandleAnomaly", function(ctx, input)
    local r = ctx:execute_activity("remediate", input)
    return { fixed = r.fixed, anomaly = input.anomaly }
end)

-- Activities — these would do real work (DB scans, alerts, etc.) in a
-- production setup. Here they're deterministic stand-ins.
workflow.activity("scan_anomalies", function(ctx, input)
    -- Pretend we found three anomalies in this region
    return {
        region = input.region,
        anomalies = {
            { id = "a-1", kind = "stale_lock" },
            { id = "a-2", kind = "missing_index" },
            { id = "a-3", kind = "orphaned_record" },
        },
    }
end)

workflow.activity("remediate", function(ctx, input)
    -- Pretend we fixed it
    return { fixed = true, anomaly = input.anomaly }
end)

log.info("nightly-report worker ready — POST a schedule to fire it")
workflow.listen({ queue = "default" })
