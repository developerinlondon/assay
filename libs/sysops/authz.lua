--! sysops.authz - per-resource permission lookup for the auth gateway.
--!
--! Maps request paths to a (resource, relation) tuple, then asks the
--! engine's Zanzibar store whether the signed-in user holds that
--! tuple. Result is cached per (sub, tuple) for AUTHZ_CACHE_TTL
--! seconds to keep the steady-state per-request cost ~0.
--!
--! Resource convention matches libs/sysops/pages/zanzibar/bootstrap.lua
--! (the existing operator UI for granting tuples):
--!
--!   auth:system#admin    — manage users, sessions, OIDC clients, JWKS
--!   engine:core#admin    — engine core ops (modules, instances, config)
--!   workflow:main#access — workflow runs, schedules, queues
--!   vault:main#access    — vault sealing, KV, transit, collections
--!
--! Paths that don't map to any tuple ("public after auth") are open
--! to every signed-in user — e.g. /, /auth/login, /auth/callback,
--! /api/v1/engine/auth/whoami (the intercept itself), static assets.

local ctx = require("sysops.ctx")

local M = {}

local AUTHZ_CACHE_TTL = 30 -- seconds

-- Path → required tuple. ORDER MATTERS: first matching prefix wins.
-- More specific paths must come before broader ones (e.g.
-- /api/v1/engine/auth/whoami BEFORE /api/v1/engine/auth).
local PATH_RULES = {
  -- ── public-after-auth (no resource check) ────────────────────
  { prefix = "/auth/login",                bypass = true },
  { prefix = "/auth/callback",             bypass = true },
  { prefix = "/auth/logout",               bypass = true },
  { prefix = "/auth/bootstrap",            bypass = true }, -- first-admin claim
  { prefix = "/api/v1/engine/auth/whoami", bypass = true },
  { prefix = "/static/",                   bypass = true },
  { prefix = "/brand/",                    bypass = true },
  { prefix = "/shared/",                   bypass = true }, -- cross-nav assets
  { prefix = "/healthz",                   bypass = true },
  { prefix = "/favicon.ico",               bypass = true },

  -- ── host-ops pages — require host:local#admin ────────────────
  -- Sysops's own dashboard surfaces (audit log, machines, services,
  -- backups, cron, networking, journal, shell). NOT publicly visible
  -- to every signed-in user — operators only.
  { prefix = "/audit",        object_type = "host", object_id = "local", relation = "admin" },
  { prefix = "/machines",     object_type = "host", object_id = "local", relation = "admin" },
  { prefix = "/services",     object_type = "host", object_id = "local", relation = "admin" },
  { prefix = "/backups",      object_type = "host", object_id = "local", relation = "admin" },
  { prefix = "/cron",         object_type = "host", object_id = "local", relation = "admin" },
  { prefix = "/interfaces",   object_type = "host", object_id = "local", relation = "admin" },
  { prefix = "/logs",         object_type = "host", object_id = "local", relation = "admin" },
  { prefix = "/tunnels",      object_type = "host", object_id = "local", relation = "admin" },
  { prefix = "/tailscale",    object_type = "host", object_id = "local", relation = "admin" },
  { prefix = "/shell",        object_type = "host", object_id = "local", relation = "admin" },
  { prefix = "/api/events",   object_type = "host", object_id = "local", relation = "admin" },
  { prefix = "/api/overview", object_type = "host", object_id = "local", relation = "admin" },
  { prefix = "/api/machines", object_type = "host", object_id = "local", relation = "admin" },
  { prefix = "/api/logs",     object_type = "host", object_id = "local", relation = "admin" },
  { prefix = "/api/audit",    object_type = "host", object_id = "local", relation = "admin" },

  -- ── /api/v1/engine/auth/admin/* — full auth admin ────────────
  { prefix = "/api/v1/engine/auth",        object_type = "auth", object_id = "system",
                                            relation = "admin" },

  -- ── /api/v1/engine/core/* — full engine admin ────────────────
  { prefix = "/api/v1/engine/core",        object_type = "engine", object_id = "core",
                                            relation = "admin" },

  -- ── /api/v1/engine/workflow/* — workflow access ──────────────
  { prefix = "/api/v1/engine/workflow",    object_type = "workflow", object_id = "main",
                                            relation = "access" },

  -- ── /api/v1/vault/* — vault access ───────────────────────────
  { prefix = "/api/v1/vault",              object_type = "vault", object_id = "main",
                                            relation = "access" },

  -- ── SPA shells + sysops's own pages ──────────────────────────
  -- /auth/* (sysops users/sessions/oidc/jwks + dashboard /auth/console)
  { prefix = "/auth/console",              object_type = "auth", object_id = "system",
                                            relation = "admin" },
  { prefix = "/auth/style.css",            bypass = true }, -- shared asset across SPAs
  { prefix = "/auth/app.js",               bypass = true },
  { prefix = "/auth/components/",          bypass = true },
  { prefix = "/auth/",                     object_type = "auth", object_id = "system",
                                            relation = "admin" },

  -- /zanzibar/* — zanzibar admin lives in sysops
  { prefix = "/zanzibar",                  object_type = "auth", object_id = "system",
                                            relation = "admin" },

  -- /engine/* (dashboard SPA + its assets)
  { prefix = "/engine/style.css",          bypass = true },
  { prefix = "/engine/app.js",             bypass = true },
  { prefix = "/engine/components/",        bypass = true },
  { prefix = "/engine/",                   object_type = "engine", object_id = "core",
                                            relation = "admin" },

  -- /workflow* — dashboard SPA assets
  { prefix = "/workflow/style.css",        bypass = true },
  { prefix = "/workflow/theme.css",        bypass = true },
  { prefix = "/workflow/app.js",           bypass = true },
  { prefix = "/workflow/components/",      bypass = true },
  { prefix = "/workflow",                  object_type = "workflow", object_id = "main",
                                            relation = "access" },

  -- /vault/* (sysops's own pages + dashboard /vault/console)
  { prefix = "/vault/style.css",           bypass = true },
  { prefix = "/vault/app.js",              bypass = true },
  { prefix = "/vault/components/",         bypass = true },
  { prefix = "/vault",                     object_type = "vault", object_id = "main",
                                            relation = "access" },
}

--- Strip the mount prefix from `path` so PATH_RULES (which list
--- unprefixed paths like /auth/login) match a request to /host/auth/login
--- when sysops is mounted at /host. Falls through cleanly when prefix
--- is empty / root.
local function strip_mount_prefix(path)
  local prefix = ctx.prefix
  if type(prefix) ~= "string" or prefix == "" or prefix == "/" then
    return path
  end
  -- ctx.prefix is normalized to drop a trailing slash by mount.lua, so
  -- it looks like "/host". Strip it iff path starts with prefix + "/" or
  -- equals prefix exactly.
  if path == prefix then return "/" end
  if path:sub(1, #prefix + 1) == prefix .. "/" then
    return path:sub(#prefix + 1)
  end
  return path
end

--- Look up the permission rule for `path`. Returns one of:
---   { bypass = true }              — no check needed (signed-in is enough)
---   { object_type, object_id, relation }  — required tuple
---   nil                            — no rule matched (defaults to bypass
---                                    UNLESS the path falls under a
---                                    sensitive prefix; see is_allowed)
-- Inner lookup that assumes `p` has already been mount-prefix-stripped.
-- Used by is_allowed so the strip happens exactly once per request.
local function rule_for_stripped(p)
  for _, rule in ipairs(PATH_RULES) do
    if p == rule.prefix or p:sub(1, #rule.prefix) == rule.prefix then
      return rule
    end
  end
  return nil
end

function M.rule_for_path(path)
  if type(path) ~= "string" then return nil end
  return rule_for_stripped(strip_mount_prefix(path))
end

----------------------------------------------------------------------
-- Per-(sub, tuple) cache
----------------------------------------------------------------------

local cache = {} -- key → { allowed = bool, expires_at = epoch }

local function cache_key(sub, ot, oid, rel)
  return sub .. "|" .. ot .. ":" .. oid .. "#" .. rel
end

local function cache_get(key)
  local entry = cache[key]
  if entry and entry.expires_at > os.time() then return entry.allowed end
  if entry then cache[key] = nil end
  return nil
end

local function cache_put(key, allowed)
  cache[key] = { allowed = allowed, expires_at = os.time() + AUTHZ_CACHE_TTL }
end

--- Clear the entire cache. Useful when tuples change (e.g. an admin
--- grants/revokes via the UI). Pages that mutate Zanzibar tuples
--- should call this so the next request sees the new state.
function M.invalidate()
  cache = {}
end

----------------------------------------------------------------------
-- The permission check itself
----------------------------------------------------------------------

-- Path prefixes that MUST have an explicit rule. If a request under one
-- of these prefixes doesn't match a rule in PATH_RULES, fail closed —
-- gateway.proxy injects the admin bearer on session-authenticated
-- proxied requests, so an unmapped path would otherwise grant any
-- signed-in user full engine-admin power. Sensitive prefixes:
local STRICT_PREFIXES = {
  "/api/v1/",
  "/auth/",
  "/vault/",
  "/engine/",
  "/workflow",
  "/zanzibar",
}

local function in_strict_prefix(path)
  for _, p in ipairs(STRICT_PREFIXES) do
    if path == p or path:sub(1, #p) == p then return true end
  end
  return false
end

--- Returns (allow, reason). `allow` is a boolean; `reason` is a short
--- string describing why (cache-hit, granted, missing-tuple, etc.) —
--- useful for audit logging and tests.
function M.is_allowed(sub, path)
  if type(sub) ~= "string" or sub == "" then return false, "no-sub" end
  if type(path) ~= "string"         then return false, "no-path" end

  -- Strip the mount prefix once so both the rule lookup and the
  -- sensitive-prefix check operate on the same unprefixed path.
  local stripped = strip_mount_prefix(path)
  local rule = rule_for_stripped(stripped)
  if rule == nil then
    -- Fail-closed on sensitive prefixes; everything else (root,
    -- consumer-added routes like /skip-trace, etc.) stays open.
    if in_strict_prefix(stripped) then return false, "no-rule-strict" end
    return true, "no-rule"
  end
  if rule.bypass then return true, "bypass" end

  local key = cache_key(sub, rule.object_type, rule.object_id, rule.relation)
  local cached = cache_get(key)
  if cached ~= nil then return cached, "cache" end

  -- Cache miss — query the engine's Zanzibar store directly. The
  -- engine wants split (subject_type, subject_id, permission,
  -- resource_type, resource_id) — NOT the SDK's combined subject/
  -- object strings — so we POST the body shape directly instead of
  -- going through sysops.auth.new(ctx.engine).zanzibar.check.
  if not ctx.engine then
    cache_put(key, false)
    return false, "no-engine"
  end
  local resp = ctx.engine.post(
    "/api/v1/engine/auth/admin/zanzibar/check",
    {
      subject_type  = "user",
      subject_id    = sub,
      permission    = rule.relation,
      resource_type = rule.object_type,
      resource_id   = rule.object_id,
    }
  )
  if type(resp) ~= "table" or resp.status ~= 200 then
    -- Transient or schema-mismatch error: fail closed but DON'T cache.
    return false, "check-error"
  end

  -- Engine returns { result = "Allowed"|"Denied", allowed = bool }.
  -- Body comes back as either a parsed table or a raw JSON string.
  local body = resp.body
  if type(body) == "string" then
    local ok, decoded = pcall(json.parse, body)
    body = ok and decoded or nil
  end
  local allowed = type(body) == "table" and body.allowed == true
  cache_put(key, allowed)
  return allowed, allowed and "granted" or "missing-tuple"
end

--- Convenience for the layout sidebar: returns a table of bools for
--- each of the four canonical panes. Each entry is the result of
--- is_allowed on the canonical URL for that pane. The per-tuple cache
--- means this is one engine RTT per pane on first render, then ~0 for
--- subsequent renders within AUTHZ_CACHE_TTL.
function M.can_access(sub)
  if type(sub) ~= "string" or sub == "" then
    return { auth = false, engine = false, workflow = false, vault = false, host = false }
  end
  return {
    auth     = (M.is_allowed(sub, "/auth/users")),
    engine   = (M.is_allowed(sub, "/engine/console")),
    workflow = (M.is_allowed(sub, "/workflow")),
    vault    = (M.is_allowed(sub, "/vault/console")),
    host     = (M.is_allowed(sub, "/audit")),
  }
end

return M
