--! services/pkg_view.lua — adapter over `assay.pkg` (plan 20).
--!
--! Provides the catalog / templates / desired-state / reconcile shape
--! that `pages/machines/new.lua` and `services/nspawn/provision.lua`
--! expect. Replaces the predecessor's `services.host.packages` —
--! everything underneath now lands in `assay.pkg` (binary stdlib) plus
--! a small file-backed desired-state store using the `fs` and `json`
--! builtins.
--!
--! Operator-configurable paths come from mount() opts:
--!   opts.catalog_paths        list of catalog directories
--!   opts.template_paths       list of template directories
--!   opts.desired_state_path   single file path for the JSON desired-state
--!
--! When a path is not configured the corresponding lookup is empty:
--! `catalog()` / `templates()` return `{ entries = {} }` and
--! `read_desired_state()` returns `{ targets = {} }`. `write_desired_state()`
--! returns `nil, err` when persistence isn't configured.

local pkg = require("assay.pkg")
local ctx = require("hostops.ctx")

local M = {}

local function paths_or_empty(t)
  if type(t) == "table" then return t end
  return {}
end

--- Load + merge catalog entries from `ctx.catalog_paths`.
function M.catalog()
  local entries = pkg.catalog.load(paths_or_empty(ctx.catalog_paths)) or {}
  return { entries = entries }
end

--- Load + merge template entries from `ctx.template_paths`. Catalog
--- entries get passed in for cross-reference (templates extend catalog
--- packages with rootfs / nspawn / resources blocks).
function M.templates(catalog_entries)
  local entries = pkg.templates.load(
    paths_or_empty(ctx.template_paths),
    catalog_entries or {}) or {}
  return { entries = entries }
end

--- Read the persisted desired-state file. Returns `{ targets = {…} }`;
--- empty when the file doesn't exist or `desired_state_path` isn't set.
function M.read_desired_state()
  local path = ctx.desired_state_path
  if not path then return { targets = {} } end
  local body = fs.read(path)
  if not body or body == "" then return { targets = {} } end
  local ok, parsed = pcall(json.parse, body)
  if not ok or type(parsed) ~= "table" then return { targets = {} } end
  parsed.targets = parsed.targets or {}
  return parsed
end

--- Persist the desired-state. Atomic via `<path>.tmp` + rename.
function M.write_desired_state(state)
  local path = ctx.desired_state_path
  if not path then
    return nil, "desired_state_path not configured"
  end
  local body = json.encode(state or { targets = {} })
  local tmp  = path .. ".tmp"
  fs.write(tmp, body)
  local ok, err = os.rename(tmp, path)
  if not ok then return nil, tostring(err) end
  return true
end

--- Apply target `name`'s desired packages over current state.
--- Returns `{ ok, result = { applied, failed, … } }` mirroring the
--- shape `assay.pkg.apply` produces.
function M.reconcile(name, opts_in)
  opts_in = opts_in or {}

  local desired = M.read_desired_state()
  local target_desired = desired.targets and desired.targets[name]
  if not target_desired then
    return {
      ok = false,
      result = {
        failed = {},
        error = "no desired state for target `" .. tostring(name) .. "`",
      },
    }
  end

  local catalog = M.catalog()
  local target  = pkg.target.machine(name)
  local actual  = pkg.query_all(target, catalog.entries, opts_in) or {}

  local desired_set = {}
  for _, id in ipairs(target_desired.packages or {}) do
    desired_set[id] = true
  end

  local plan_   = pkg.plan(name, desired_set, actual, catalog.entries)
  local result  = pkg.apply(plan_, target, catalog.entries, opts_in)
  return { ok = true, result = result }
end

return M
