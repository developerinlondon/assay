--- @module assay.pkg
--- @description Package manager framework — catalog loading, target abstractions, plan/apply/reconcile.
--- @keywords pkg, package, apt, binary, install, upgrade, reconcile, idempotent

local M = {}

-- ── Helpers ──────────────────────────────────────────────────────────────────

local ID_PATTERN = "^[a-z0-9%-]+$"

local function is_array(t)
  if type(t) ~= "table" then return false end
  local i = 0
  for _ in pairs(t) do
    i = i + 1
    if t[i] == nil then return false end
  end
  return true
end

-- Filesystem-walk: returns sorted array of *.toml paths under `dir`. Non-existent
-- dirs return empty array (no error). Uses the `fs` builtin.
-- NOTE: fs.list returns an array of {name=string, type=string} tables.
local function list_toml_files(dir)
  if not fs.exists(dir) then return {} end
  local ok, entries = pcall(fs.list, dir)
  if not ok then return {} end
  local out = {}
  for _, entry in ipairs(entries) do
    local name = entry.name
    if name and name:match("%.toml$") then
      out[#out+1] = dir .. "/" .. name
    end
  end
  table.sort(out)
  return out
end

-- ── Catalog ──────────────────────────────────────────────────────────────────

M.catalog = {}

--- Validate a single decoded catalog entry. Returns a list of errors
--- (empty if valid) where each error is { field=string, message=string }.
local function validate_catalog_entry(decoded)
  local errs = {}
  local pkg = decoded and decoded.package
  if type(pkg) ~= "table" then
    errs[#errs+1] = { field = "package", message = "missing [package] table" }
    return errs
  end
  if type(pkg.id) ~= "string" or not pkg.id:match(ID_PATTERN) then
    errs[#errs+1] = { field = "package.id", message = "id must match [a-z0-9-]+" }
  end
  if type(pkg.display_name) ~= "string" or pkg.display_name == "" then
    errs[#errs+1] = { field = "package.display_name", message = "required" }
  end
  if not is_array(pkg.methods) or #pkg.methods == 0 then
    errs[#errs+1] = { field = "package.methods", message = "must be non-empty array" }
  else
    for _, m in ipairs(pkg.methods) do
      if m ~= "apt" and m ~= "binary" then
        errs[#errs+1] = { field = "package.methods", message = "unknown method: " .. tostring(m) }
      elseif type(pkg[m]) ~= "table" then
        errs[#errs+1] = { field = "package." .. m, message = "missing block for declared method" }
      end
    end
  end
  -- Per-method shape checks
  if type(pkg.apt) == "table" then
    for _, f in ipairs({ "source_list", "package_name" }) do
      if type(pkg.apt[f]) ~= "string" or pkg.apt[f] == "" then
        errs[#errs+1] = { field = "package.apt." .. f, message = "required string" }
      end
    end
  end
  if type(pkg.binary) == "table" then
    for _, f in ipairs({ "release_api", "asset_pattern", "sha256_source", "install_path", "mode" }) do
      if type(pkg.binary[f]) ~= "string" or pkg.binary[f] == "" then
        errs[#errs+1] = { field = "package.binary." .. f, message = "required string" }
      end
    end
    -- asset_pattern placeholder check: only {arch}, {tag}, {ver} allowed
    if type(pkg.binary.asset_pattern) == "string" then
      for ph in pkg.binary.asset_pattern:gmatch("{([^}]+)}") do
        if ph ~= "arch" and ph ~= "tag" and ph ~= "ver" then
          errs[#errs+1] = {
            field = "package.binary.asset_pattern",
            message = "unknown placeholder {" .. ph .. "} (allowed: arch, tag, ver)",
          }
        end
      end
    end
  end
  return errs
end

--- Load catalog from layered paths. Returns:
---   { entries = { [id] = entry_with_origin }, errors = [ {path, package_id, field, message} ] }
---
--- Layering: paths are evaluated in order; later-layer entries OVERWRITE earlier
--- entries with the same id (full-entry override, no field-merge).
---
--- Each entry is tagged with `_origin`:
---   layer 1 → "built-in", layer 2 → "plugin:<dirname>", layer 3+ → "operator:<filename>"
function M.catalog.load(paths)
  if type(paths) ~= "table" then
    error("pkg.catalog.load: paths must be array of directory paths", 2)
  end
  local entries, errors = {}, {}

  for layer_idx, dir in ipairs(paths) do
    local files = list_toml_files(dir)
    for _, file in ipairs(files) do
      local raw = fs.read(file)
      -- NOTE: the toml builtin exposes toml.parse (not toml.decode)
      local ok, decoded = pcall(toml.parse, raw)
      if not ok then
        errors[#errors+1] = {
          path = file, package_id = nil, field = nil,
          message = "TOML parse error: " .. tostring(decoded),
        }
      else
        local entry_errs = validate_catalog_entry(decoded)
        if #entry_errs > 0 then
          local id = (decoded.package and decoded.package.id) or file
          for _, e in ipairs(entry_errs) do
            errors[#errors+1] = {
              path = file, package_id = id,
              field = e.field, message = e.message,
            }
          end
        else
          local entry = decoded.package
          if layer_idx == 1 then
            entry._origin = "built-in"
          elseif layer_idx == 2 then
            entry._origin = "plugin:" .. (dir:match("([^/]+)/?$") or dir)
          else
            entry._origin = "operator:" .. (file:match("([^/]+)$") or file)
          end
          entries[entry.id] = entry
        end
      end
    end
  end

  return { entries = entries, errors = errors }
end

--- Get a single entry from the loaded catalog by id. Returns nil if not present.
function M.catalog.get(entries, id) return entries[id] end

--- List entries as a sorted-by-id array.
function M.catalog.list(entries)
  local arr = {}
  for _, e in pairs(entries) do arr[#arr+1] = e end
  table.sort(arr, function(a, b) return a.id < b.id end)
  return arr
end

-- ── Templates (stub — filled in Task 10) ─────────────────────────────────────
M.templates = {}

-- ── Targets (stub — filled in Task 12) ───────────────────────────────────────
M.target = {}

-- ── Version (stub — filled in Task 11) ───────────────────────────────────────
M.version = {}

-- ── Reconciler stubs (filled in Task 13+; query/apply/reconcile owned by knowhere) ──
function M.query() error("pkg.query: not implemented yet") end
function M.query_all() error("pkg.query_all: not implemented yet") end
function M.plan() error("pkg.plan: not implemented yet") end
function M.apply() error("pkg.apply: not implemented yet") end
function M.reconcile() error("pkg.reconcile: not implemented yet") end

return M
