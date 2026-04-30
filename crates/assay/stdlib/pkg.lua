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
--- NOTE: `_origin` is synthetic — not part of the on-disk schema. Downstream
--- serializers (e.g. plan writers, API responses) must strip it before output.
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
          -- Strict-override: an invalid entry shadows any valid earlier-layer entry
          -- with the same id. Operator deliberately tried to override; failure surfaces
          -- as a missing entry rather than silent fallback.
          if type(decoded.package) == "table" and type(decoded.package.id) == "string" then
            entries[decoded.package.id] = nil
          end
        else
          local entry = decoded.package
          if layer_idx == 1 then
            entry._origin = "built-in"
          elseif layer_idx == 2 then
            entry._origin = "plugin:" .. (dir:gsub("/+$", ""):match("([^/]+)$") or dir)
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

-- ── Templates ────────────────────────────────────────────────────────────────

M.templates = {}

local function validate_template_entry(decoded, catalog_entries)
  local errs = {}
  local t = decoded and decoded.template
  if type(t) ~= "table" then
    errs[#errs+1] = { field = "template", message = "missing [template] table" }
    return errs
  end
  if type(t.id) ~= "string" or not t.id:match(ID_PATTERN) then
    errs[#errs+1] = { field = "template.id", message = "id must match [a-z0-9-]+" }
  end
  if type(t.display_name) ~= "string" or t.display_name == "" then
    errs[#errs+1] = { field = "template.display_name", message = "required" }
  end
  if not is_array(t.packages) then
    errs[#errs+1] = { field = "template.packages", message = "must be array (may be empty)" }
  else
    for _, pkg_id in ipairs(t.packages) do
      if type(catalog_entries) == "table" and catalog_entries[pkg_id] == nil then
        errs[#errs+1] = {
          field = "template.packages",
          message = "references unknown catalog id: " .. tostring(pkg_id),
        }
      end
    end
  end
  return errs
end

--- Load templates from layered paths. `catalog_entries` is the catalog map
--- returned by `pkg.catalog.load(...).entries`; templates with packages not in
--- it are rejected.
---
--- Layering matches the catalog loader: last-wins full-entry override + strict
--- override (an invalid override clears any earlier valid entry with the same id).
--- `_origin` is synthetic; downstream serializers must strip it.
function M.templates.load(paths, catalog_entries)
  if type(paths) ~= "table" then
    error("pkg.templates.load: paths must be array of directory paths", 2)
  end
  local entries, errors = {}, {}

  for layer_idx, dir in ipairs(paths) do
    local files = list_toml_files(dir)
    for _, file in ipairs(files) do
      local raw = fs.read(file)
      local ok, decoded = pcall(toml.parse, raw)
      if not ok then
        errors[#errors+1] = {
          path = file, template_id = nil, field = nil,
          message = "TOML parse error: " .. tostring(decoded),
        }
      else
        local entry_errs = validate_template_entry(decoded, catalog_entries)
        if #entry_errs > 0 then
          local id = (decoded.template and decoded.template.id) or file
          for _, e in ipairs(entry_errs) do
            errors[#errors+1] = {
              path = file, template_id = id,
              field = e.field, message = e.message,
            }
          end
          -- Strict-override: an invalid template entry clears any valid
          -- earlier-layer entry with the same id (matches catalog semantics).
          if type(decoded.template) == "table" and type(decoded.template.id) == "string" then
            entries[decoded.template.id] = nil
          end
        else
          local entry = decoded.template
          if layer_idx == 1 then
            entry._origin = "built-in"
          elseif layer_idx == 2 then
            entry._origin = "plugin:" .. (dir:gsub("/+$", ""):match("([^/]+)$") or dir)
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

function M.templates.get(entries, id) return entries[id] end

function M.templates.list(entries)
  local arr = {}
  for _, e in pairs(entries) do arr[#arr+1] = e end
  table.sort(arr, function(a, b) return a.id < b.id end)
  return arr
end

-- ── Targets ──────────────────────────────────────────────────────────────────

M.target = {}

local Target = {}
Target.__index = Target

--- Run a command on the target. Returns shell.exec-shaped table:
---   { status, stdout, stderr, timed_out }
---
--- opts is a subset of the underlying builtins' opts, restricted to keys both
--- shell.exec and systemd.machine_exec accept:
---   timeout : seconds (finite, >= 0; 0 means "no timeout")
---   env     : { [name] = value } map
--- Other shell.exec-only opts (cwd, stdin) are ignored on machine targets
--- to preserve cross-target semantics.
function Target:exec(cmd, opts)
  opts = opts or {}
  if self.kind == "host" then
    return shell.exec(cmd, opts)
  elseif self.kind == "machine" then
    return systemd.machine_exec(self.id, cmd, opts)
  else
    error("unknown target kind: " .. tostring(self.kind))
  end
end

--- Return the host target singleton.
function M.target.host()
  return setmetatable({ kind = "host", id = "host" }, Target)
end

--- Return a machine target wrapping the given nspawn machine name.
function M.target.machine(name)
  if type(name) ~= "string" or name == "" then
    error("pkg.target.machine: name required", 2)
  end
  if name == "host" then
    error("pkg.target.machine: 'host' is reserved; use pkg.target.host()", 2)
  end
  return setmetatable({ kind = "machine", id = name }, Target)
end

-- ── Version comparator ───────────────────────────────────────────────────────

M.version = {}

--- Parse a version string into an array of integers. Strips a leading "v",
--- splits on ".", drops trailing non-numeric components silently. Returns {0}
--- for unparseable input rather than nil so cmp() always works.
function M.version.parse(s)
  if type(s) ~= "string" then return {0} end
  local t = (s:gsub("^v", ""))
  local out = {}
  for piece in t:gmatch("[^%.]+") do
    local n = tonumber(piece:match("^(%d+)"))
    if n then out[#out+1] = n end
  end
  if #out == 0 then out[1] = 0 end
  return out
end

--- Compare two version strings: -1 if a<b, 0 if equal, 1 if a>b.
--- Shorter input is treated as zero-padded to match the longer.
function M.version.cmp(a, b)
  local pa = M.version.parse(a)
  local pb = M.version.parse(b)
  local n = math.max(#pa, #pb)
  for i = 1, n do
    local ai = pa[i] or 0
    local bi = pb[i] or 0
    if ai < bi then return -1
    elseif ai > bi then return 1 end
  end
  return 0
end

-- ── Reconciler stubs (filled in Task 13+; query/apply/reconcile owned by knowhere) ──
function M.query() error("pkg.query: not implemented yet") end
function M.query_all() error("pkg.query_all: not implemented yet") end

--- Build a deterministic plan to converge `actual` toward `desired_set` using
--- entries from `catalog_entries`. Pure function: no side effects, no I/O.
---
--- @param target_id    string       informational only ("host" or machine name)
--- @param desired_set  string[]     desired catalog ids (order ignored, sorted internally)
--- @param actual       table        { [id] = {installed=bool, version=string?, available=string?} }
--- @param catalog_entries table     loaded catalog map
--- @return table[]                  array of operations: {op, id, method, ...}
---
--- Reconcile NEVER removes — packages in `actual` but not `desired_set` are ignored.
function M.plan(target_id, desired_set, actual, catalog_entries)
  if type(desired_set) ~= "table" then
    error("pkg.plan: desired_set must be array", 2)
  end
  actual = actual or {}
  -- Sort desired_set for determinism — operator order shouldn't influence the plan.
  local sorted = {}
  for _, id in ipairs(desired_set) do sorted[#sorted+1] = id end
  table.sort(sorted)

  local plan = {}
  for _, id in ipairs(sorted) do
    local entry = catalog_entries[id]
    if not entry then
      -- Catalog dropped or never had this id; record but don't crash.
      plan[#plan+1] = {
        op = "skip", id = id, method = nil,
        reason = "no catalog entry for id",
      }
    else
      local act = actual[id] or { installed = false }
      local method = entry.methods[1]  -- priority order; reconciler may fall back at apply time
      if not act.installed then
        plan[#plan+1] = {
          op = "install", id = id, method = method,
          target_version = act.available,
        }
      elseif act.available
             and act.version
             and M.version.cmp(act.version, act.available) < 0 then
        plan[#plan+1] = {
          op = "upgrade", id = id, method = method,
          from = act.version, to = act.available,
        }
      end
      -- else: installed and up-to-date → no op.
    end
  end
  return plan
end

function M.apply() error("pkg.apply: not implemented yet") end
function M.reconcile() error("pkg.reconcile: not implemented yet") end

return M
