local ctx = require("hostops.ctx")
local M = {}

-- Sidebar version. The VERSION file is the canonical source of truth
-- (release tag matches it byte-for-byte) so we surface it directly —
-- no SHA suffix, no "+dirty". If you need the build's exact commit,
-- /health returns the git SHA separately.
local VERSION = (fs.read("VERSION") or "0.0.0"):gsub("%s+$", "")

-- Pull the actor (display name shown in the sidebar footer) from a
-- request. Mirrors the per-page `actor_from` helper that older pages
-- (dashboard.lua, machine.lua, …) used to define inline. Centralised
-- here so newer engine pages don't have to copy-paste the same lookup.
function M.actor_from(req)
  local h = (req and req.headers) or {}
  return h["Cf-Access-Authenticated-User-Email"]
      or h["cf-access-authenticated-user-email"]
      or "local-dev"
end

function M.fragment(template_name, ctx)
  ctx = ctx or {}
  local tpl = fs.read("templates/partials/" .. template_name .. ".html") or ""
  local body = template.render_string(tpl, ctx)
  return {
    status  = 200,
    body    = body,
    headers = { ["Content-Type"] = "text/html; charset=utf-8" },
  }
end

-- Build a clickable breadcrumb HTML string from a list of `{href, label}`
-- entries. The last entry is the current page — rendered as plain text
-- (not a link). Use this from page handlers that want a navigational
-- eyebrow instead of the static "A · B · C" text — e.g.
--   ctx.breadcrumb = render.breadcrumb({
--     {"/auth/users", "Auth"},
--     "Users",                         -- string-only = current, no link
--   })
-- The CSS in static/styles.css styles `.page-eyebrow a` with hover.
function M.breadcrumb(entries)
  local parts = {}
  for i, entry in ipairs(entries or {}) do
    local href, label
    if type(entry) == "table" then
      href, label = entry[1] or entry.href, entry[2] or entry.label
    else
      href, label = nil, tostring(entry)
    end
    if href and i < #entries then
      table.insert(parts,
        '<a href="' .. tostring(href) .. '">' .. tostring(label) .. '</a>')
    else
      table.insert(parts, tostring(label))
    end
  end
  return table.concat(parts, ' &middot; ')
end

-- Apply layout-wide defaults to `ctx` based on `req`. Sets every key the
-- layout template needs (brand, title, nav_active, version, host,
-- machines, actor, plugins_sidebar, active_modules) without overwriting
-- caller-supplied values. Called by `M.render` for knowhere's bundled
-- templates and by `pages/plugins/dispatch.lua` for plugin templates so
-- both paths produce identical layout context. Mutates and returns ctx.
function M.layout_defaults(ctx, req, fallback_nav_active)
  ctx = ctx or {}
  local b = ctx.brand.snapshot()
  ctx.brand = ctx.brand or b
  ctx.title = ctx.title or b.title
  ctx.nav_active = ctx.nav_active or fallback_nav_active
  ctx.version = VERSION
  -- Pull host + machines from the cached state.snapshot() so every
  -- page (not just the dashboard) renders the same brand-bar host
  -- name and sidebar machines list. The defaults below are only used
  -- when state.snapshot fails (boot-race, etc.).
  do
    local ok, state = pcall(require, "services.state")
    local snap
    if ok and state and state.snapshot then
      local ok2, s = pcall(state.snapshot)
      if ok2 then snap = s end
    end
    ctx.host     = ctx.host     or (snap and snap.host)     or { name = "host", ip = "" }
    ctx.machines = ctx.machines or (snap and snap.machines) or {}
  end
  ctx.actor = ctx.actor or M.actor_from(req)
  if ctx.plugins_sidebar == nil then
    local ok, plugins = pcall(require, "services.plugins")
    local list = {}
    if ok and plugins and plugins.sidebar then
      local ok2, l = pcall(plugins.sidebar)
      if ok2 then list = l end
    end
    -- Stamp each sidebar entry with `is_active = true` if its href's
    -- path matches the current request path. The layout template uses
    -- this to highlight the matching sub-entry rather than just the
    -- parent "Plugins" link, so operators see WHICH plugin page they're
    -- on at a glance.
    local current_path = (req and req.path) or ""
    for _, e in ipairs(list) do
      local entry_path = (e.href or ""):match("^([^?]+)") or e.href or ""
      e.is_active = (entry_path ~= "" and entry_path == current_path)
    end
    ctx.plugins_sidebar = list
  end
  ctx.active_modules = ctx.active_modules or _G.KNOWHERE_ACTIVE_MODULES or {}
  return ctx
end

-- Render `content_html` (already a string) inside knowhere's layout.html
-- with full layout context. Used by callers that have to render their
-- content template themselves (e.g. plugin pages whose templates live
-- outside knowhere's bundled templates/ dir).
function M.wrap_layout(content_html, ctx, req)
  ctx = M.layout_defaults(ctx, req)
  local layout = fs.read("templates/layout.html") or ""
  ctx.content = content_html
  local body = template.render_string(layout, ctx)
  return {
    status  = 200,
    body    = body,
    headers = { ["Content-Type"] = "text/html; charset=utf-8" },
  }
end

function M.render(template_name, ctx, req)
  ctx = M.layout_defaults(ctx, req, template_name)
  local content_tpl = fs.read("templates/" .. template_name .. ".html") or ""
  local content = template.render_string(content_tpl, ctx)
  return M.wrap_layout(content, ctx, req)
end

return M
