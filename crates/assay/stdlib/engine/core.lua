--- @module assay.engine.core
--- @description Lua client for assay-engine's engine-core admin API mounted at `/api/v1/engine/core/*`. Covers info, modules, instances, audit, config, plus the public `/healthz` and `/api/v1/engine/core/active-modules` probes.
--- @keywords engine, core, admin, modules, instances, audit, config, health
--- @quickref core.client(opts) -> client | Build a core admin client (engine_url + optional api_key)
--- @quickref c:info() -> {version, instance_id, modules, ...} | Public engine identity
--- @quickref c:health() -> {status, modules, ...} | Public health probe
--- @quickref c:active_modules() -> {modules} | Public list of enabled modules
--- @quickref c.modules:list() -> {items} | Admin list of every known module row
--- @quickref c.modules:toggle(name, enabled?) -> {enabled, restart_required, message} | Admin enable/disable a module
--- @quickref c.instances:list({limit?, offset?}) -> {items} | Admin list known instances
--- @quickref c.audit:list({actor?, action?, since?, until?, limit?, offset?}) -> {items, total, limit, offset} | Admin paginated audit log
--- @quickref c:config() -> table | Admin sanitised config (secrets redacted)

local M = {}

local function trim_slash(s) return (s or ""):gsub("/+$", "") end

local function url_encode(s)
  return (tostring(s):gsub("([^A-Za-z0-9%-_.~])", function(ch)
    return string.format("%%%02X", string.byte(ch))
  end))
end

--- Build a core client.
---
--- opts:
---   engine_url   (string, required)  base URL of the assay-engine, e.g. "http://localhost:8420"
---   api_key      (string, optional)  admin bearer token; if absent ASSAY_ADMIN_KEY is read
function M.client(opts)
  opts = opts or {}
  local engine_url = trim_slash(opts.engine_url or env.get("ASSAY_ENGINE_URL") or "")
  if engine_url == "" then
    error("assay.engine.core: engine_url required (or set ASSAY_ENGINE_URL)")
  end
  local api_key = opts.api_key or env.get("ASSAY_ADMIN_KEY")

  local function build_headers(admin)
    local h = { ["Content-Type"] = "application/json" }
    if admin and api_key and api_key ~= "" then
      h["Authorization"] = "Bearer " .. api_key
    end
    return h
  end

  local function decode(resp, allow_empty)
    if resp.status >= 200 and resp.status < 300 then
      if allow_empty and (resp.status == 204 or resp.body == "" or resp.body == nil) then
        return nil
      end
      if resp.body == nil or resp.body == "" then return nil end
      return json.parse(resp.body)
    end
    error("assay.engine.core: HTTP " .. tostring(resp.status) .. ": " .. (resp.body or ""))
  end

  local function get(path, admin)
    return decode(http.get(engine_url .. path, { headers = build_headers(admin) }))
  end

  local function post(path, body, admin)
    return decode(
      http.post(engine_url .. path, body, { headers = build_headers(admin) }),
      true
    )
  end

  local c = {}

  -- ===== Public probes (no auth) =====

  --- GET /api/v1/engine/core/info — version, instance_id, started_at,
  --- modules, backend, bind addr, public URL.
  function c:info() return get("/api/v1/engine/core/info", false) end

  --- GET /healthz — engine liveness redirect; falls through to
  --- `/api/v1/engine/core/health`. Returns the same envelope.
  function c:health() return get("/api/v1/engine/core/health", false) end

  --- GET /api/v1/engine/core/active-modules — `{modules = ["workflow",
  --- "auth", ...]}`. Read by the dashboard cross-nav.
  function c:active_modules() return get("/api/v1/engine/core/active-modules", false) end

  -- ===== Admin: modules =====

  c.modules = {}

  --- GET /api/v1/engine/core/modules — admin list of every known module
  --- row (enabled, last actor, version, raw config).
  function c.modules:list() return get("/api/v1/engine/core/modules", true) end

  --- POST /api/v1/engine/core/modules/{name}/toggle — flip the enabled
  --- flag. Pass `enabled` as a bool to set explicitly; nil flips current.
  function c.modules:toggle(name, enabled)
    local body = {}
    if enabled ~= nil then body.enabled = enabled end
    return post("/api/v1/engine/core/modules/" .. url_encode(name) .. "/toggle", body, true)
  end

  -- ===== Admin: instances =====

  c.instances = {}

  --- GET /api/v1/engine/core/instances — admin list of running instances.
  --- @param opts table? `{limit?, offset?}`
  function c.instances:list(qopts)
    qopts = qopts or {}
    local parts = {}
    if qopts.limit then parts[#parts + 1] = "limit=" .. tostring(qopts.limit) end
    if qopts.offset then parts[#parts + 1] = "offset=" .. tostring(qopts.offset) end
    local qs = (#parts > 0) and ("?" .. table.concat(parts, "&")) or ""
    return get("/api/v1/engine/core/instances" .. qs, true)
  end

  -- ===== Admin: audit =====

  c.audit = {}

  --- GET /api/v1/engine/core/audit — paginated engine audit log.
  --- @param opts table? `{actor?, action?, since?, until?, limit?, offset?}`.
  ---   `since` and `until` are unix-epoch seconds (floats accepted).
  ---   `until_` is also accepted as a key (Lua reserves `until`).
  function c.audit:list(qopts)
    qopts = qopts or {}
    local parts = {}
    if qopts.actor then parts[#parts + 1] = "actor=" .. url_encode(qopts.actor) end
    if qopts.action then parts[#parts + 1] = "action=" .. url_encode(qopts.action) end
    if qopts.since then parts[#parts + 1] = "since=" .. tostring(qopts.since) end
    local until_v = qopts["until"] or qopts.until_
    if until_v then parts[#parts + 1] = "until=" .. tostring(until_v) end
    if qopts.limit then parts[#parts + 1] = "limit=" .. tostring(qopts.limit) end
    if qopts.offset then parts[#parts + 1] = "offset=" .. tostring(qopts.offset) end
    local qs = (#parts > 0) and ("?" .. table.concat(parts, "&")) or ""
    return get("/api/v1/engine/core/audit" .. qs, true)
  end

  -- ===== Admin: config =====

  --- GET /api/v1/engine/core/config — sanitised config snapshot
  --- (admin_api_keys + any field matching `password|secret|api_key` is
  --- replaced by the literal `[REDACTED]`).
  function c:config() return get("/api/v1/engine/core/config", true) end

  return c
end

return M
