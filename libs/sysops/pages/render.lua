local hctx = require("sysops.ctx")
local M = {}

-- Sidebar version. The lib's VERSION file is the canonical source of
-- truth (release tag matches it byte-for-byte). Resolved lazily so the
-- read happens after mount() has populated ctx.lib_root, and falls back
-- to "0.0.0" when the file isn't readable (test fixtures, etc.).
local function read_version()
  local root = hctx.lib_root or "."
  local ok, raw = pcall(fs.read, root .. "/VERSION")
  return (ok and raw or "0.0.0"):gsub("%s+$", "")
end

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
  local root = hctx.lib_root or "."
  local tpl_path = root .. "/templates/partials/" .. template_name .. ".html"
  local ok_t, tpl = pcall(fs.read, tpl_path)
  if not ok_t or not tpl then tpl = "" end
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
  local b = hctx.brand.snapshot()
  ctx.brand = ctx.brand or b
  ctx.title = ctx.title or b.title
  ctx.nav_active = ctx.nav_active or fallback_nav_active
  ctx.version = read_version()
  -- Pull host + machines from the cached state snapshot so every page
  -- (not just the dashboard) renders the same brand-bar host name and
  -- sidebar machines list. Errors fall through to safe defaults so a
  -- boot-race in state can't 500 the whole layout.
  do
    local ok, snap = pcall(hctx.state.snapshot)
    if not ok then snap = nil end
    ctx.host     = ctx.host     or (snap and snap.host)     or { name = "host", ip = "" }
    ctx.machines = ctx.machines or (snap and snap.machines) or {}
  end
  ctx.actor = ctx.actor or M.actor_from(req)
  -- 0.1.5: surface mount-supplied opts.active_modules to the layout so
  -- the conditional Auth + Vault sidebar links render. Empty list when
  -- consumer hasn't opted in — same effect as 0.1.4.
  ctx.active_modules = ctx.active_modules or hctx.active_modules or {}
  -- Engine sidecar URL (set via mount opts.engine_base_url). Layout
  -- conditionally renders /auth/console, /vault/console, /engine/console,
  -- /workflow/ sidebar links when present.
  ctx.engine_base_url = ctx.engine_base_url or hctx.engine_base_url
  -- Consumer-app sidebar links (set via mount opts.extra_sidebar_links).
  ctx.extra_sidebar_links = ctx.extra_sidebar_links or hctx.extra_sidebar_links
  return ctx
end

-- Render `content_html` (already a string) inside knowhere's layout.html
-- with full layout context. Used by callers that have to render their
-- content template themselves (e.g. plugin pages whose templates live
-- outside knowhere's bundled templates/ dir).
function M.wrap_layout(content_html, ctx, req)
  ctx = M.layout_defaults(ctx, req)
  local root = hctx.lib_root or "."
  local ok_l, layout = pcall(fs.read, root .. "/templates/layout.html")
  if not ok_l or not layout then layout = "" end
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
  local root = hctx.lib_root or "."
  local template_dir = root .. "/templates"
  -- Use render_with_loader so `{% include "partials/..." %}` directives
  -- resolve relative to the templates dir. render_string can't handle
  -- includes (no loader configured), which silently breaks any page
  -- whose template has an include — hit on machine detail / shell /
  -- per-machine services|cron|logs.
  local ok_c, content = pcall(template.render_with_loader,
                              template_dir,
                              template_name .. ".html",
                              ctx)
  if not ok_c then
    content = "<pre>render error: " .. tostring(content) .. "</pre>"
  end
  return M.wrap_layout(content, ctx, req)
end

return M
