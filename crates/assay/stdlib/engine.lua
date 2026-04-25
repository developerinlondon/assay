--- @module assay.engine
--- @description Convenience umbrella for the assay-engine client trio (core, auth, workflow). One `engine.connect()` call returns all three clients sharing a base URL + admin key. Prefer requiring the individual submodules directly (`assay.engine.core`, etc.) if you only need one.
--- @keywords engine, assay, core, auth, workflow, idp, oidc, zanzibar, scheduler
--- @quickref engine.core — engine-core admin (info, modules, instances, audit, config)
--- @quickref engine.auth — auth (login, passkey, OIDC client + provider, biscuit, zanzibar, admin)
--- @quickref engine.workflow — workflow (CRUD, schedules, namespaces, workers, queues; worker mode via :register_* + :listen)
--- @quickref engine.connect(url|opts, api_key?) -> {core, auth, workflow} | Build all three clients in one call

local core = require("assay.engine.core")
local auth = require("assay.engine.auth")
local workflow = require("assay.engine.workflow")

local M = {
  core = core,
  auth = auth,
  workflow = workflow,
}

--- Build the full client trio against one assay-engine.
---
--- Two call shapes:
---
---   engine.connect("http://localhost:8420")
---   engine.connect("http://localhost:8420", "admin-key")
---   engine.connect({ engine_url = "...", api_key = "...", session_cookie = "..." })
---
--- Returns a record `{ core, auth, workflow }`. Each entry is the
--- corresponding submodule's `client(opts)` result, sharing the same
--- engine URL + bearer token + session cookie. Falls back to the
--- environment for unset fields:
---
---   ASSAY_ENGINE_URL  → engine_url
---   ASSAY_ADMIN_KEY   → api_key
function M.connect(url_or_opts, api_key)
  local opts
  if type(url_or_opts) == "table" then
    opts = url_or_opts
  else
    opts = { engine_url = url_or_opts, api_key = api_key }
  end
  return {
    core = core.client(opts),
    auth = auth.client(opts),
    workflow = workflow.client(opts),
  }
end

return M
