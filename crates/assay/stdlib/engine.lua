--- @module assay.engine
--- @description Lua wrapper for assay-engine's engine-core admin API mounted at `/api/v1/engine/*`. Covers info, modules, instances, audit, config, plus the public `/healthz` and `/api/v1/modules` probes.
--- @keywords engine, admin, modules, instances, audit, config, health
--- @quickref engine.connect(url) -> nil | Set the engine base URL (or via env ASSAY_ENGINE_URL)
--- @quickref engine.set_admin_key(key) -> nil | Set the admin bearer (or via env ASSAY_ADMIN_KEY)
--- @quickref engine.info() -> {version, instance_id, modules, ...} | Public engine identity
--- @quickref engine.healthz() -> {status, modules, ...} | Public health probe
--- @quickref engine.modules_active() -> {modules} | Public list of enabled modules
--- @quickref engine.modules.list() -> {items} | Admin list module rows (enabled flag + last actor)
--- @quickref engine.modules.toggle(name, enabled?) -> {enabled, restart_required, message} | Admin enable/disable a module
--- @quickref engine.instances.list({limit?, offset?}) -> {items} | Admin list known instances
--- @quickref engine.audit.list({actor, action, since, until, limit, offset}) -> {items, total, limit, offset} | Admin paginated audit log
--- @quickref engine.config() -> table | Admin sanitised config (secrets redacted)
---
--- Errors are raised as Lua errors with the form `assay.engine: HTTP <status>: <body>`
--- — matches the `assay.auth` convention so callers can `pcall` and parse uniformly.

local M = {}

local function trim_slash(s)
  return (s or ""):gsub("/+$", "")
end

-- Module-scoped connection state. `connect` and `set_admin_key` mutate
-- these so callers don't have to thread the URL/key through every call.
-- Same shape as `assay.workflow` (`_engine_url` + `_auth_token`) so
-- agents reading both modules see one pattern.
M._engine_url = nil
M._admin_key = nil

--- Set the engine base URL. Accepts trailing slash (stripped). Falls
--- back to ASSAY_ENGINE_URL when called without an argument so a script
--- relying purely on env can just call `engine.connect()`.
function M.connect(url)
  local resolved = trim_slash(url or env.get("ASSAY_ENGINE_URL") or "")
  if resolved == "" then
    error("assay.engine.connect: url required (or set ASSAY_ENGINE_URL)")
  end
  M._engine_url = resolved
end

--- Set the admin bearer used on `/api/v1/engine/*` admin endpoints. The
--- public probes (`info`, `healthz`, `modules_active`) ignore this.
--- Falls back to ASSAY_ADMIN_KEY when called without an arg.
function M.set_admin_key(key)
  M._admin_key = key or env.get("ASSAY_ADMIN_KEY")
end

local function require_url()
  if not M._engine_url or M._engine_url == "" then
    -- Allow auto-resolution from env so unconfigured scripts still work
    -- when the env var is set — same fallback `set_admin_key` uses.
    local fallback = env.get("ASSAY_ENGINE_URL") or ""
    if fallback ~= "" then
      M._engine_url = trim_slash(fallback)
    else
      error("assay.engine: not connected — call engine.connect(url) first")
    end
  end
  return M._engine_url
end

local function require_admin()
  local key = M._admin_key or env.get("ASSAY_ADMIN_KEY")
  if not key or key == "" then
    error("assay.engine: admin key required — call engine.set_admin_key(key) "
      .. "or set ASSAY_ADMIN_KEY")
  end
  return key
end

local function build_headers(admin)
  local h = { ["Content-Type"] = "application/json" }
  if admin then
    h["Authorization"] = "Bearer " .. require_admin()
  end
  return h
end

local function decode(resp, allow_empty)
  if resp.status >= 200 and resp.status < 300 then
    if allow_empty and (resp.status == 204 or resp.body == "" or resp.body == nil) then
      return nil
    end
    if resp.body == nil or resp.body == "" then
      return nil
    end
    return json.parse(resp.body)
  end
  error("assay.engine: HTTP " .. tostring(resp.status) .. ": " .. (resp.body or ""))
end

local function get(path, admin)
  return decode(http.get(require_url() .. path, { headers = build_headers(admin) }))
end

local function post(path, body, admin)
  return decode(
    http.post(require_url() .. path, body or {}, { headers = build_headers(admin) }),
    true
  )
end

-- ===== Public probes =====

--- GET /api/v1/engine/info — version, instance_id, started_at, modules,
--- backend kind, redacted backend URL, bind addr, public URL. No auth.
function M.info()
  return get("/api/v1/engine/info", false)
end

--- GET /healthz — engine liveness + version + active modules. No auth.
function M.healthz()
  return get("/healthz", false)
end

--- GET /api/v1/modules — `{ modules = ["workflow", "auth", ...] }`.
--- The dashboard reads this to decide which panes to render. No auth.
function M.modules_active()
  return get("/api/v1/modules", false)
end

-- ===== Admin: modules =====

M.modules = {}

--- GET /api/v1/engine/modules — admin list of every known module row
--- (enabled flag, last actor that toggled it, version, raw config blob).
function M.modules.list()
  return get("/api/v1/engine/modules", true)
end

--- POST /api/v1/engine/modules/{name}/toggle — flip the enabled flag.
--- @param name string Module identifier (e.g. "auth", "workflow")
--- @param enabled boolean? Explicit target; nil flips current value
--- @return table { enabled, restart_required, message }
function M.modules.toggle(name, enabled)
  local body = {}
  if enabled ~= nil then
    body.enabled = enabled
  end
  return post("/api/v1/engine/modules/" .. name .. "/toggle", body, true)
end

-- ===== Admin: instances =====

M.instances = {}

local function build_query(opts, allowed)
  if not opts then return "" end
  local parts = {}
  for _, k in ipairs(allowed) do
    local v = opts[k]
    if v ~= nil and v ~= "" then
      parts[#parts + 1] = k .. "=" .. tostring(v)
    end
  end
  if #parts == 0 then return "" end
  return "?" .. table.concat(parts, "&")
end

--- GET /api/v1/engine/instances — admin list of running instances
--- (single row for SQLite, one per process for PG).
--- @param opts table? { limit?, offset? }
function M.instances.list(opts)
  return get("/api/v1/engine/instances" .. build_query(opts, { "limit", "offset" }), true)
end

-- ===== Admin: audit =====

M.audit = {}

--- GET /api/v1/engine/audit — paginated engine audit log.
--- @param opts table? { actor?, action?, since?, until?, limit?, offset? }
---   `since` and `until` are unix epoch seconds (floats accepted).
---   `until_` is also accepted as the key (Lua keyword guard) and is
---   normalised to `until` on the wire.
function M.audit.list(opts)
  opts = opts or {}
  -- Lua reserves `until` — operators may pass `until_` instead. Map it
  -- to the wire-name without mutating the caller's table.
  local q = {
    actor = opts.actor,
    action = opts.action,
    since = opts.since,
    ["until"] = opts["until"] or opts.until_,
    limit = opts.limit,
    offset = opts.offset,
  }
  local parts = {}
  for _, k in ipairs({ "actor", "action", "since", "until", "limit", "offset" }) do
    local v = q[k]
    if v ~= nil and v ~= "" then
      parts[#parts + 1] = k .. "=" .. tostring(v)
    end
  end
  local query = ""
  if #parts > 0 then query = "?" .. table.concat(parts, "&") end
  return get("/api/v1/engine/audit" .. query, true)
end

-- ===== Admin: config =====

--- GET /api/v1/engine/config — sanitised config (admin_api_keys + any
--- key matching `password|secret|api_key` are replaced by [REDACTED]).
function M.config()
  return get("/api/v1/engine/config", true)
end

return M
