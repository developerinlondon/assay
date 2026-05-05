--! Long-running e2e boot script for libs/sysops.
--!
--! Mounts sysops with the smoke-test stubs and binds http.serve on
--! E2E_PORT (default 47921). The Playwright runner navigates here.

local sysops = require("sysops.mount")
local stubs   = require("stubs")

local PORT = tonumber(env.get("E2E_PORT") or "47921")

local routes = { GET = {}, POST = {} }
sysops.mount(routes, stubs.opts())

-- Liveness probe — Playwright's run.sh waits on this before running specs.
routes.GET["/__e2e_alive"] = function()
  return { status = 200, body = "ok",
    headers = { ["Content-Type"] = "text/plain" } }
end

print(("[sysops.e2e] listening on http://127.0.0.1:%d"):format(PORT))
http.serve(PORT, routes)
