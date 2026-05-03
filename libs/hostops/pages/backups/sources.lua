-- pages/backups_sources.lua
--
-- GET /backups/sources       — sources editor
-- POST /api/backups/sources  — update sources

local render  = require("pages.render")
local form    = require("pages.form")
local state   = require("services.state")
local backups = require("services.host.backups")
local fs_snap = require("services.host.fs_snapshot")

local M = {}

local function actor_from(req)
  local h = (req and req.headers) or {}
  return h["Cf-Access-Authenticated-User-Email"]
      or h["cf-access-authenticated-user-email"]
      or "local-dev"
end

function M.editor(req)
  local snap = state.snapshot()
  local profile = backups.read_profile()
  if not profile then
    return { status = 303, headers = { ["Location"] = "/backups" } }
  end

  -- Detect FS backend for /var/lib/machines (the typical container root)
  local ok, det = pcall(fs_snap.detect, "/var/lib/machines")
  local fs_backend = (ok and type(det) == "table") and det or { backend = "none" }

  -- List nspawn machines so we can render "containers detected"
  local machines = {}
  if systemd and type(systemd.list_machines) == "function" then
    local ok2, ms = pcall(systemd.list_machines)
    if ok2 and type(ms) == "table" then machines = ms end
  end

  return render.render("backups/sources", {
    nav_active = "backups",
    host = snap.host,
    machines = snap.machines,
    sources = (profile.backup and profile.backup.sources) or {},
    fs_backend = fs_backend,
    nspawn_machines = machines,
  }, req)
end

function M.update(req)
  local f = form.parse(req)
  -- Multi-value fields: HTML form submits each `sources[]` as a separate
  -- "sources" entry. assay's form parser keeps only the last; we work
  -- around by parsing the body directly here for the sources array.
  local sources = {}
  local body = req and req.body or ""
  for pair in body:gmatch("[^&]+") do
    local k, v = pair:match("^([^=]+)=(.+)$")
    if k == "sources%5B%5D" or k == "sources[]" then
      v = v:gsub("+", " "):gsub("%%(%x%x)", function(h)
        return string.char(tonumber(h, 16))
      end)
      table.insert(sources, v)
    end
  end
  -- Append a custom path if the operator added one
  if f.custom_path and f.custom_path ~= "" then
    table.insert(sources, f.custom_path)
  end
  if #sources == 0 then
    return { status = 400, body = "pick at least one source path" }
  end

  local res = backups.update_sources({
    actor = actor_from(req),
    sources = sources,
  })
  if not res.ok then
    return { status = 400, body = "update failed: " .. (res.error or "?") }
  end
  return { status = 303, headers = { ["Location"] = "/backups?sources=ok" } }
end

return M
