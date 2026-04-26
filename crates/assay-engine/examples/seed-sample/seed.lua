--! Sample-data seeder — populates a running assay-engine with demo
--! users, OIDC clients, an upstream provider, Zanzibar tuples, and a
--! couple of demo workflows so operators can poke the dashboards
--! without writing curl scripts.
--!
--! Replaces the v0.13.x `assay-engine seed-sample` Rust subcommand
--! (deleted in plan-15 slice 5). Same fixtures, same idempotency: every
--! insert is guarded by a list-then-skip check or uses an upsert
--! endpoint, so the script is safe to re-run.
--!
--! Prerequisites:
--!
--!   - A running assay-engine with `auth.admin_api_keys = ["..."]`
--!     configured for the break-glass during seeding.
--!   - examples/init/init.lua already run so the operator's user
--!     exists and the Zanzibar namespace schemas are in place.
--!
--! Usage:
--!
--!   ASSAY_ENGINE_URL=http://localhost:8420 \
--!   ASSAY_ADMIN_KEY=dev-admin-key-change-me \
--!   assay run examples/seed-sample/seed.lua

local engine = require("assay.engine")

local function row(kind, name, status)
  print(string.format("%-15s %-32s %s", kind, name, status))
end

local engine_url = env.get("ASSAY_ENGINE_URL")
local admin_key = env.get("ASSAY_ADMIN_KEY")
if not engine_url or engine_url == "" then
  error("seed.lua: ASSAY_ENGINE_URL is required")
end
if not admin_key or admin_key == "" then
  error("seed.lua: ASSAY_ADMIN_KEY is required")
end

local e = engine.connect({ engine_url = engine_url, api_key = admin_key })

print(string.format("seed-sample → %s", engine_url))
print(string.format("%-15s %-32s %s", "kind", "name", "status"))
print(string.rep("-", 80))

-- ── Workflow namespaces + workflows ────────────────────────────────────

local function safe(label, fn)
  local ok, err = pcall(fn)
  if ok then return true end
  if tostring(err):find("HTTP 409") or tostring(err):find("HTTP 400")
    or tostring(err):find("HTTP 500") then
    -- Existing-row paths surface as 400/409/500 from the workflow
    -- store today; treat as "already there" for idempotency.
    return false
  end
  error(label .. ": " .. tostring(err))
end

for _, ns in ipairs({ "demo", "prod" }) do
  if safe("namespace " .. ns, function() e.workflow.namespaces:create(ns) end) then
    row("namespace", ns, "created")
  else
    row("namespace", ns, "exists")
  end
end

for _, spec in ipairs({
  { id = "demo-greet-1", input = { name = "alice" } },
  { id = "demo-greet-2", input = { name = "bob" } },
  { id = "demo-greet-3", input = { name = "cousin" } },
}) do
  if safe("workflow " .. spec.id, function()
    e.workflow:start({
      workflow_type = "demo.greet",
      workflow_id = spec.id,
      namespace = "demo",
      task_queue = "default",
      input = json.encode(spec.input),
    })
  end) then
    row("workflow", spec.id, "created")
  else
    row("workflow", spec.id, "exists")
  end
end

-- Auth fixtures only land if auth is enabled. Probe via active modules.
local active = e.core:active_modules()
local auth_on = false
for _, m in ipairs((active and active.modules) or {}) do
  if m == "auth" then auth_on = true; break end
end

if not auth_on then
  row("auth-suite", "(all)", "skipped: auth module not enabled")
else
  -- ── Users ────────────────────────────────────────────────────────────

  local users = {
    { email = "alice@example.com", display = "Alice Demo", verified = true,
      password = "assay-demo" },
    { email = "bob@example.com", display = "Bob Demo", verified = true,
      password = "assay-demo" },
    { email = "cousin@example.com", display = "Cousin Demo", verified = false,
      password = nil },
    { email = "admin@example.com", display = "Admin Demo", verified = true,
      password = "assay-demo" },
  }

  -- Pre-load existing emails so we skip duplicates.
  local existing_emails = {}
  do
    local list = e.auth.users:list({ limit = 500 })
    for _, u in ipairs((list and list.items) or {}) do
      if u.email then existing_emails[u.email] = u end
    end
  end

  for _, u in ipairs(users) do
    if existing_emails[u.email] then
      row("user", u.email, "exists")
    else
      local body = {
        email = u.email,
        display_name = u.display,
        email_verified = u.verified,
      }
      if u.password then body.password = u.password end
      e.auth.users:create(body)
      row("user", u.email, "created")
    end
  end

  -- ── OIDC clients ─────────────────────────────────────────────────────

  local existing_clients = {}
  for _, c in ipairs(e.auth.oidc_clients:list() or {}) do
    if c.client_id then existing_clients[c.client_id] = true end
  end

  local clients = {
    {
      client_id = "demo-spa",
      name = "Demo SPA (PKCE-only)",
      redirect_uris = { "http://localhost:5173/callback" },
      token_endpoint_auth_method = "none",
      pkce_required = true,
    },
    {
      client_id = "demo-service",
      name = "Demo Service (confidential)",
      redirect_uris = { "http://localhost:6001/oauth/callback" },
      token_endpoint_auth_method = "client_secret_basic",
      pkce_required = true,
    },
  }
  for _, c in ipairs(clients) do
    if existing_clients[c.client_id] then
      row("oidc_client", c.client_id, "exists")
    else
      local result = e.auth.oidc_clients:create(c)
      if result and result.client_secret then
        row("oidc_client", c.client_id, "created (secret: " .. result.client_secret .. ")")
      else
        row("oidc_client", c.client_id, "created")
      end
    end
  end

  -- ── OIDC upstream provider (mock) ────────────────────────────────────

  e.auth.oidc_upstream:upsert({
    slug = "example",
    display_name = "Example IdP",
    issuer = "https://accounts.example.com",
    client_id = "demo-upstream-client",
    client_secret = "replace-me",
  })
  row("oidc_upstream", "example", "upserted")

  -- ── Zanzibar namespaces (family + circle demo) ───────────────────────
  --
  -- The auth-console Zanzibar pane lists registered namespace SCHEMAS
  -- (auth.zanzibar_namespaces), separate from the (auth.zanzibar_tuples)
  -- below. Without these `define_namespace` calls the pane is empty
  -- even though tuples exist — the dashboard groups by schema, not by
  -- the implicit set of object_types found in tuples.
  --
  -- Schema shape: NamespaceSchema { name, definitions: { <name> = RelationDef } }
  -- RelationDef.kind is serde-tagged: { kind = "direct", value = [TypeRef] }
  -- TypeRef = { object_type, relation = nil, wildcard = false } — for now
  -- both demo namespaces accept direct `user` subjects only.
  local function direct_user()
    return {
      kind = "direct",
      value = { { object_type = "user", relation = json.null, wildcard = false } },
    }
  end

  local zanzibar_namespaces = {
    {
      name = "family",
      definitions = {
        admin  = { name = "admin",  kind = direct_user() },
        member = { name = "member", kind = direct_user() },
      },
    },
    {
      name = "circle",
      definitions = {
        member = { name = "member", kind = direct_user() },
      },
    },
  }
  for _, ns in ipairs(zanzibar_namespaces) do
    e.auth.zanzibar:define_namespace(ns)
    row("zanzibar_namespace", ns.name, "defined")
  end

  -- ── Zanzibar tuples (family + circle demo) ───────────────────────────

  local alice_id, bob_id
  do
    local list = e.auth.users:list({ search = "alice@example.com", limit = 5 })
    for _, u in ipairs((list and list.items) or {}) do
      if u.email == "alice@example.com" then alice_id = u.id; break end
    end
    list = e.auth.users:list({ search = "bob@example.com", limit = 5 })
    for _, u in ipairs((list and list.items) or {}) do
      if u.email == "bob@example.com" then bob_id = u.id; break end
    end
  end

  local tuples = {
    { object_type = "family", object_id = "alice", relation = "admin",
      subject_type = "user", subject_id = alice_id },
    { object_type = "family", object_id = "alice", relation = "member",
      subject_type = "user", subject_id = alice_id },
    { object_type = "family", object_id = "bob", relation = "admin",
      subject_type = "user", subject_id = bob_id },
    { object_type = "family", object_id = "bob", relation = "member",
      subject_type = "user", subject_id = bob_id },
    { object_type = "circle", object_id = "inner", relation = "member",
      subject_type = "user", subject_id = alice_id },
    { object_type = "circle", object_id = "inner", relation = "member",
      subject_type = "user", subject_id = bob_id },
  }
  for _, t in ipairs(tuples) do
    if t.subject_id then
      e.auth.zanzibar:write(t)
      row("zanzibar_tuple",
        t.object_type .. ":" .. t.object_id .. "#" .. t.relation,
        "written")
    end
  end
end

print("")
print("seed complete.")
