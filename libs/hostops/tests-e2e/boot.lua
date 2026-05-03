--! Long-running e2e boot script for libs/hostops.
--!
--! Mounts hostops with the smoke-test stubs and binds http.serve on
--! E2E_PORT (default 47921). The Playwright runner navigates here.

local hostops = require("hostops.mount")
local stubs   = require("stubs")

local PORT = tonumber(env.get("E2E_PORT") or "47921")

local routes = { GET = {}, POST = {} }
hostops.mount(routes, stubs.opts())

-- Liveness probe — Playwright's run.sh waits on this before running specs.
routes.GET["/__e2e_alive"] = function()
  return { status = 200, body = "ok",
    headers = { ["Content-Type"] = "text/plain" } }
end

print(("[hostops.e2e] listening on http://127.0.0.1:%d"):format(PORT))
http.serve(PORT, routes)
