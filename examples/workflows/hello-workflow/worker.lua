-- hello-workflow — smallest possible assay workflow.
-- Run: assay run worker.lua  (with `assay serve` running on :8080)

local workflow = require("assay.workflow")

workflow.connect(env.get("ASSAY_ENGINE_URL") or "http://localhost:8080")

workflow.define("GreetWorkflow", function(ctx, input)
    local r = ctx:execute_activity("greet", { who = input.who })
    return { greeting = r.message }
end)

workflow.activity("greet", function(ctx, input)
    return { message = "hello, " .. input.who }
end)

log.info("hello-workflow worker ready — POST a workflow to start one")
workflow.listen({ queue = "default" })
