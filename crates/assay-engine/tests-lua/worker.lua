--! Worker for the e2e test — registers `demo.greet` + the `say_hello`
--! activity and listens on the `default` queue. Spawned by run.sh as a
--! background process; killed when the script exits.

local engine = require("assay.engine")

local e = engine.connect({
  engine_url = env.get("ASSAY_ENGINE_URL"),
  api_key = env.get("ASSAY_ADMIN_KEY"),
})

e.workflow:register_workflow("demo.greet", function(ctx, input)
  local payload = input
  if type(payload) == "string" then payload = json.parse(payload) end
  local who = (payload and payload.name) or "anonymous"
  local greeting = ctx:execute_activity("say_hello", { name = who })
  return greeting
end)

e.workflow:register_activity("say_hello", function(_ctx, input)
  return "hello, " .. tostring(input.name)
end)

print("worker started; listening on default…")
e.workflow:listen({ queue = "default", namespace = "main" })
