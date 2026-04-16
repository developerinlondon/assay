-- approval-pipeline — pause between activities for human approval.
-- Run: assay run worker.lua  (with `assay serve` running on :8080)

local workflow = require("assay.workflow")
workflow.connect(env.get("ASSAY_ENGINE_URL") or "http://localhost:8080")

workflow.define("ApproveAndDeploy", function(ctx, input)
    -- Step 1: build
    local artifact = ctx:execute_activity("build", { ref = input.git_sha })

    -- Step 2: wait indefinitely for an `approve` signal. The worker
    -- yields here; the workflow consumes no resources until a signal
    -- arrives via POST /workflows/:id/signal/approve.
    local approval = ctx:wait_for_signal("approve")

    -- Step 3: deploy
    return ctx:execute_activity("deploy", {
        image = artifact.image,
        env = input.target_env,
        approver = approval and approval.by or "unknown",
    })
end)

workflow.activity("build", function(ctx, input)
    -- Real impl would call a CI system. Simulated for the example.
    return {
        image = "registry.example.com/app:" .. input.ref:sub(1, 8),
        sha = input.ref,
    }
end)

workflow.activity("deploy", function(ctx, input)
    -- Real impl would call k8s / nomad / etc.
    return {
        url = "https://" .. input.env .. ".example.com/app",
        approver = input.approver,
    }
end)

log.info("approval-pipeline worker ready")
workflow.listen({ queue = "default" })
