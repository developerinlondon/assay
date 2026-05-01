--- @module assay.pkg
--- @description Package manager framework — catalog loading, target abstractions, plan/apply/reconcile.
--- @keywords pkg, package, apt, binary, install, upgrade, reconcile, idempotent

local M = {}

-- ── Helpers ──────────────────────────────────────────────────────────────────

-- Read an absolute filesystem path directly (bypasses any FileSource the
-- consumer registered for fs.read). pkg.lua reads cache markers, downloaded
-- binaries, and apt-source idempotency files from absolute paths under
-- /var/cache, /usr/share, /etc — none of which a sandboxed FileSource
-- (e.g. knowhere's LayeredFs over the embedded VFS) is going to resolve.
-- io.open hits the real disk regardless.
--
-- Returns the file content as a string, or nil if the file doesn't exist
-- or can't be opened. Caller decides whether nil is fatal.
local function read_disk(path, mode)
  local f = io.open(path, mode or "r")
  if not f then return nil end
  local body = f:read("*a")
  f:close()
  return body
end

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
    -- asset_pattern placeholder check: only {arch}, {uname_m}, {tag}, {ver} allowed
    if type(pkg.binary.asset_pattern) == "string" then
      for ph in pkg.binary.asset_pattern:gmatch("{([^}]+)}") do
        if ph ~= "arch" and ph ~= "uname_m" and ph ~= "tag" and ph ~= "ver" then
          errs[#errs+1] = {
            field = "package.binary.asset_pattern",
            message = "unknown placeholder {" .. ph .. "} (allowed: arch, uname_m, tag, ver)",
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

-- Validation helpers for the optional [template.rootfs] / [template.nspawn] /
-- [template.systemd] subtables introduced for nspawn provisioning.
--
-- Templates without these sections remain valid — they describe a "packages
-- only" profile, which is the v1 behavior. When present, sections gate which
-- fields are required.

local ROOTFS_SOURCES = {
  ["machinectl-pull-tar"] = true,  -- requires url
  ["machinectl-pull-raw"] = true,  -- requires url
  ["machinectl-clone"]    = true,  -- requires from
  ["debootstrap"]         = true,  -- requires suite + mirror
}

-- Allowed values for `[template.nspawn] resolv_conf`. Mirrors systemd-nspawn's
-- --resolv-conf= flag exactly so we can pass it through verbatim.
local NSPAWN_RESOLV_CONF_VALUES = {
  off = true, copy_host = true, copy_static = true, copy_uplink = true,
  copy_stub = true, replace_host = true, replace_static = true,
  replace_uplink = true, replace_stub = true, bind_host = true,
  bind_static = true, bind_uplink = true, bind_stub = true,
  delete = true, auto = true,
}
-- Convenience: also accept hyphenated variants ("bind-host") and map.
local function nspawn_resolv_conf_normalize(s)
  if type(s) ~= "string" then return nil end
  local norm = s:gsub("-", "_")
  return NSPAWN_RESOLV_CONF_VALUES[norm] and norm or nil
end

local function validate_template_rootfs(rootfs, errs)
  if type(rootfs) ~= "table" then
    errs[#errs+1] = { field = "template.rootfs", message = "must be a table" }
    return
  end
  local source = rootfs.source
  if type(source) ~= "string" or not ROOTFS_SOURCES[source] then
    errs[#errs+1] = {
      field = "template.rootfs.source",
      message = "must be one of: machinectl-pull-tar, machinectl-pull-raw, machinectl-clone, debootstrap",
    }
    return
  end
  if source == "machinectl-pull-tar" or source == "machinectl-pull-raw" then
    if type(rootfs.url) ~= "string" or not rootfs.url:match("^https?://") then
      errs[#errs+1] = {
        field = "template.rootfs.url",
        message = "required for " .. source .. "; must be http(s):// URL",
      }
    end
  elseif source == "machinectl-clone" then
    if type(rootfs.from) ~= "string" or rootfs.from == "" then
      errs[#errs+1] = {
        field = "template.rootfs.from",
        message = "required for machinectl-clone (source machine name)",
      }
    end
  elseif source == "debootstrap" then
    -- debootstrap parses positional args without `--` (legacy argv handling),
    -- so a suite name beginning with `-` would be misinterpreted as a flag.
    -- Constrain suite to known-safe characters: lowercase ASCII letter
    -- followed by alphanumerics / dot / dash. Matches every Debian/Ubuntu
    -- codename ever shipped (noble, plucky, bookworm, etc.).
    if type(rootfs.suite) ~= "string" or not rootfs.suite:match("^[a-z][a-z0-9.%-]*$") then
      errs[#errs+1] = { field = "template.rootfs.suite",
                        message = "required for debootstrap; must match ^[a-z][a-z0-9.-]*$" }
    end
    if type(rootfs.mirror) ~= "string" or not rootfs.mirror:match("^https?://") then
      errs[#errs+1] = { field = "template.rootfs.mirror", message = "required for debootstrap; must be http(s):// URL" }
    end
    -- Optional `components` (e.g. "main,universe" for Ubuntu) and explicit
    -- keyring path. Validated only for shape; debootstrap will reject bad
    -- combinations at run time with a clearer error than we can synthesize.
    if rootfs.components ~= nil and type(rootfs.components) ~= "string" then
      errs[#errs+1] = { field = "template.rootfs.components",
                        message = "must be string (comma-separated, e.g. main,universe)" }
    elseif type(rootfs.components) == "string" and not rootfs.components:match("^[A-Za-z0-9,_%-]+$") then
      errs[#errs+1] = { field = "template.rootfs.components",
                        message = "may only contain [A-Za-z0-9,_-]" }
    end
    if rootfs.keyring ~= nil and (type(rootfs.keyring) ~= "string" or not rootfs.keyring:match("^/")) then
      errs[#errs+1] = { field = "template.rootfs.keyring",
                        message = "must be an absolute path to a keyring file" }
    end
    if rootfs.variant ~= nil and (type(rootfs.variant) ~= "string"
                                   or not rootfs.variant:match("^[a-z%-]+$")) then
      errs[#errs+1] = { field = "template.rootfs.variant",
                        message = "must be lowercase alphanumeric+dash (e.g. minbase, buildd, fakechroot)" }
    end
    if rootfs.include ~= nil and (type(rootfs.include) ~= "string"
                                   or not rootfs.include:match("^[A-Za-z0-9.,_%-+]+$")) then
      errs[#errs+1] = { field = "template.rootfs.include",
                        message = "must be comma-separated package names (e.g. systemd-sysv,dbus)" }
    end
  end
end

local function validate_template_nspawn(nspawn, errs)
  if type(nspawn) ~= "table" then
    errs[#errs+1] = { field = "template.nspawn", message = "must be a table" }
    return
  end
  local function require_bool(field)
    if nspawn[field] ~= nil and type(nspawn[field]) ~= "boolean" then
      errs[#errs+1] = { field = "template.nspawn." .. field, message = "must be boolean" }
    end
  end
  require_bool("boot")
  require_bool("notify_ready")
  require_bool("virtual_ethernet")
  require_bool("private_users")
  if nspawn.resolv_conf ~= nil and not nspawn_resolv_conf_normalize(nspawn.resolv_conf) then
    errs[#errs+1] = {
      field = "template.nspawn.resolv_conf",
      message = "must be a systemd-nspawn --resolv-conf= value (e.g. bind-host, copy-host, off)",
    }
  end
  if nspawn.binds ~= nil and not is_array(nspawn.binds) then
    errs[#errs+1] = { field = "template.nspawn.binds", message = "must be array" }
  end
  if nspawn.binds_ro ~= nil and not is_array(nspawn.binds_ro) then
    errs[#errs+1] = { field = "template.nspawn.binds_ro", message = "must be array" }
  end
  if nspawn.capabilities ~= nil and not is_array(nspawn.capabilities) then
    errs[#errs+1] = { field = "template.nspawn.capabilities", message = "must be array" }
  end
  if nspawn.bridge ~= nil and (type(nspawn.bridge) ~= "string"
                                or not nspawn.bridge:match("^[A-Za-z0-9._%-]+$")) then
    errs[#errs+1] = { field = "template.nspawn.bridge",
                      message = "must be a non-empty bridge name matching [A-Za-z0-9._-]+" }
  end
end

local function validate_template_systemd(sd, errs)
  if type(sd) ~= "table" then
    errs[#errs+1] = { field = "template.systemd", message = "must be a table" }
    return
  end
  if sd.enable ~= nil and not is_array(sd.enable) then
    errs[#errs+1] = { field = "template.systemd.enable", message = "must be array of unit names" }
  end
  if sd.disable ~= nil and not is_array(sd.disable) then
    errs[#errs+1] = { field = "template.systemd.disable", message = "must be array of unit names" }
  end
end

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
      if type(pkg_id) ~= "string" or not pkg_id:match(ID_PATTERN) then
        errs[#errs+1] = {
          field = "template.packages",
          message = "elements must be strings matching [a-z0-9-]+ (got " .. type(pkg_id) .. ")",
        }
      elseif type(catalog_entries) == "table" and catalog_entries[pkg_id] == nil then
        errs[#errs+1] = {
          field = "template.packages",
          message = "references unknown catalog id: " .. tostring(pkg_id),
        }
      end
    end
  end
  -- Optional provisioning sections. Templates without these are valid
  -- "packages-only" profiles. Knowhere's machine-create flow refuses to
  -- provision a new container from such a template — those are for seeding
  -- existing machines only.
  if t.rootfs  ~= nil then validate_template_rootfs(t.rootfs, errs)   end
  if t.nspawn  ~= nil then validate_template_nspawn(t.nspawn, errs)   end
  if t.systemd ~= nil then validate_template_systemd(t.systemd, errs) end
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

-- Forward declarations: Target:exec needs the privilege helpers below
-- (defined in the M.method section so they live near their callers, but
-- referenced earlier here for the auto-elevation branch).
local is_root, sudo_prefix, shell_quote

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
    -- Caller wraps with sudo prefix where mutations need privilege.
    return shell.exec(cmd, opts)
  elseif self.kind == "machine" then
    -- systemd.machine_exec needs root to talk to systemd-machined. Auto-
    -- elevate via sudo systemd-run when not root. Inside the container the
    -- command runs as root regardless (no further sudo needed).
    if is_root() then
      return systemd.machine_exec(self.id, cmd, opts)
    else
      local outer = ("sudo -n systemd-run --machine=%s --pipe --quiet --wait --collect /bin/sh -c %s"):format(
        shell_quote(self.id), shell_quote(cmd))
      return shell.exec(outer, opts)
    end
  else
    error("unknown target kind: " .. tostring(self.kind))
  end
end

--- Return the host target singleton.
function M.target.host()
  return setmetatable({ kind = "host", id = "host" }, Target)
end

--- Return a machine target wrapping the given nspawn machine name.
---
--- Name is validated against [a-zA-Z0-9._-]+ because target.id flows into
--- filesystem paths (release_meta cache, marker files) and shell commands
--- (systemd-run --machine=). An unsanitised name with `..` or `/` would
--- escape the cache directory; one with shell metacharacters could inject
--- through systemd-run argv. systemd-machined itself enforces a similar
--- character set on machine names, so this matches reality.
function M.target.machine(name)
  if type(name) ~= "string" or name == "" then
    error("pkg.target.machine: name required", 2)
  end
  if name == "host" then
    error("pkg.target.machine: 'host' is reserved; use pkg.target.host()", 2)
  end
  if not name:match("^[A-Za-z0-9._%-]+$") then
    error("pkg.target.machine: name must match [A-Za-z0-9._-]+ (got "
          .. tostring(name) .. ")", 2)
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

-- ── Release-metadata cache (binary method) ──────────────────────────────────
--
-- Pure-framework helpers for fetching and caching upstream release metadata
-- from a release_api endpoint (currently shaped for GitHub Releases API). The
-- caller decides where the cache lives via ctx.cache_dir; the framework writes
-- under <cache_dir>/<id>/release_meta.json.

M.release = {}

--- Cache file path for an entry's release metadata.
--- @param entry_id string
--- @param ctx      table  { cache_dir = "/var/cache/.../pkg" }
function M.release.meta_path(entry_id, ctx)
  if type(ctx) ~= "table" or type(ctx.cache_dir) ~= "string" then
    error("pkg.release.meta_path: ctx.cache_dir required", 2)
  end
  return ctx.cache_dir .. "/" .. entry_id .. "/release_meta.json"
end

--- Read cached release metadata. Returns nil when missing or unparseable.
function M.release.read_meta(entry_id, ctx)
  local p = M.release.meta_path(entry_id, ctx)
  -- read_disk bypasses the consumer's FileSource (which may not resolve
  -- absolute paths under /var/cache).
  local raw = read_disk(p)
  if not raw then return nil end
  local ok, parsed = pcall(json.parse, raw)
  if ok and type(parsed) == "table" then return parsed end
  return nil
end

--- Refresh release metadata for an entry. Writes <cache_dir>/<id>/release_meta.json.
--- Raises on HTTP failure. Returns the parsed meta table.
function M.release.refresh_meta(entry, ctx)
  if not (entry and entry.binary and entry.binary.release_api) then
    error("pkg.release.refresh_meta: entry.binary.release_api required", 2)
  end
  -- GitHub's API requires a User-Agent header; without one, returns 403.
  -- Honor GITHUB_TOKEN if set (lifts unauthenticated 60/hr rate limit to
  -- 5000/hr authenticated).
  local headers = {
    Accept = "application/vnd.github+json",
    ["User-Agent"] = "assay-pkg/" .. (entry.id or "unknown"),
  }
  local tok = env.get("GITHUB_TOKEN")
  if type(tok) == "string" and tok ~= "" then
    headers.Authorization = "Bearer " .. tok
  end
  local r = http.get(entry.binary.release_api, { headers = headers, timeout = 30 })
  if r.status ~= 200 then
    error("release_api fetch failed: HTTP " .. tostring(r.status))
  end
  local parsed = json.parse(r.body)
  local meta = {
    tag = parsed.tag_name,
    ver = (parsed.tag_name or ""):gsub("^v", ""),
    fetched_at = os.date("!%Y-%m-%dT%H:%M:%SZ"),
    assets = parsed.assets or {},
  }
  fs.mkdir(ctx.cache_dir .. "/" .. entry.id)
  fs.write(M.release.meta_path(entry.id, ctx), json.encode(meta))
  return meta
end

--- Fetch the expected sha256 for an asset, using the binary block's
--- sha256_source declaration ("asset" → sibling .sha256, "checksums" →
--- sha256sums-style multi-line file). Returns hex string or nil.
function M.release.fetch_expected_sha256(b, meta, asset_name)
  if b.sha256_source == "asset" then
    local sha_url
    for _, a in ipairs(meta.assets or {}) do
      if a.name == asset_name .. ".sha256" then sha_url = a.browser_download_url end
    end
    if not sha_url then return nil end
    local r = http.get(sha_url, {
      timeout = 15,
      headers = { ["User-Agent"] = "assay-pkg" },
    })
    if r.status ~= 200 then return nil end
    return r.body:match("^(%x+)") or nil
  elseif b.sha256_source == "checksums" then
    local candidates = { "sha256sums.txt", "checksums.txt", "checksums" }
    local sums_url
    for _, name in ipairs(candidates) do
      for _, a in ipairs(meta.assets or {}) do
        if a.name == name then sums_url = a.browser_download_url; break end
      end
      if sums_url then break end
    end
    if not sums_url then return nil end
    local r = http.get(sums_url, {
      timeout = 15,
      headers = { ["User-Agent"] = "assay-pkg" },
    })
    if r.status ~= 200 then return nil end
    for line in r.body:gmatch("[^\n]+") do
      local hex, name = line:match("^(%x+)%s+%*?(.+)$")
      -- Literal equality, not name:match(asset_name) — asset_name has
      -- characters Lua treats as pattern metacharacters (e.g. `-` is a
      -- quantifier, `.` is any-char) and a malicious checksum line could
      -- otherwise spoof a match.
      if name == asset_name then return hex end
    end
    return nil
  end
  return nil
end

-- ── Method handlers (apt + binary) ──────────────────────────────────────────
--
-- Each method exposes:
--   .query(target, entry)           -> { installed, version?, installed_at? }
--   .install(target, entry, ctx)    -- install/upgrade (idempotent at apply level)
--   .remove(target, entry, ctx)     -- safe remove
--
-- ctx schema:
--   ctx.log         function(line)            optional: line emitter; defaults to no-op
--   ctx.cache_dir   string                    required for binary method
--   ctx.op          "install"|"upgrade"|"remove"   conveys plan op (binary upgrade triggers re-download)
--
-- Adoption rules for binary method:
--   1. If install_path exists but installed.json is missing → external install,
--      skip (operator must `rm` to hand it over).
--   2. If install_path is missing but `command -v <name>` finds the binary
--      somewhere else on PATH → external install, query reports installed_at
--      and apply skips. PATH name = catalog id, lower-case. Sanitised.
--
-- For apt method there is no equivalent — apt owns the package regardless of
-- who triggered the install.

M.method = { apt = {}, binary = {} }

local function noop_log(_) end
local function ctx_log(ctx) return (ctx and ctx.log) or noop_log end

-- Sanitiser: only [a-z0-9-] survives. Matches catalog id pattern.
local function safe_name(s)
  if type(s) ~= "string" then return "" end
  return (s:gsub("[^a-z0-9%-]", ""))
end

-- ── Privilege escalation ────────────────────────────────────────────────────
--
-- The host process may run as root (production systemd unit) or as an unprivileged
-- user with passwordless sudo (dev/staging). Mutating package operations
-- delegate to `sudo -n` when not root. For non-root operation, sudoers must
-- NOPASSWD-allow at minimum:
--   /usr/bin/apt-get, /usr/bin/install, /usr/bin/rm, /usr/bin/systemd-run,
--   /usr/bin/env
-- Or simply: `<user> ALL=(ALL) NOPASSWD: ALL`.

-- Bodies for the privilege helpers forward-declared near the M.target block.
local _is_root_cached = nil
is_root = function()
  if _is_root_cached == nil then
    local r = shell.exec("id -u", {})
    _is_root_cached = (r and r.stdout and r.stdout:match("^0") ~= nil) or false
  end
  return _is_root_cached
end

sudo_prefix = function()
  return is_root() and "" or "sudo -n "
end

-- Shell-quote a string so it can safely be embedded as a single argument
-- inside a /bin/sh -c '<...>' command. Wraps in single quotes; embedded
-- single-quotes become '"'"'.
shell_quote = function(s)
  if type(s) ~= "string" then return "''" end
  return "'" .. s:gsub("'", [['"'"']]) .. "'"
end

-- ─── apt ───
--
-- Source-list management is host-only (writes /etc/apt/sources.list.d). For
-- nspawn machines, sources must be baked into the image; we just run apt-get.

function M.method.apt.query(target, entry)
  if not (entry and entry.apt and entry.apt.package_name) then
    return { installed = false }
  end
  local pkg_name = entry.apt.package_name
  -- Use `dpkg-query -s` instead of `-W -f='${Package}\t${Version}\t${Status}\n'`.
  -- The custom-format placeholders changed across dpkg versions: 1.22.18 (host)
  -- accepts ${Package}/${Version}/${Status}, but 1.22.6 (Debian bookworm
  -- container) returns empty for those. The `-s` (show) command is portable
  -- across versions and returns multi-line "Field: value" output.
  local cmd = ("dpkg-query -s %q 2>/dev/null"):format(pkg_name)
  local r = target:exec(cmd, {})
  if not r or r.status ~= 0 then return { installed = false } end
  local status_line = (r.stdout or ""):match("\nStatus:%s+([^\n]+)")
                    or (r.stdout or ""):match("^Status:%s+([^\n]+)")
  local ver_line    = (r.stdout or ""):match("\nVersion:%s+([^\n]+)")
                    or (r.stdout or ""):match("^Version:%s+([^\n]+)")
  local installed = status_line and status_line:match("install ok installed") ~= nil
  local row = {
    installed = installed,
    version   = installed and ver_line or nil,
  }
  if not installed then return row end

  -- Upgradable detection via `apt-cache policy <pkg>`. Output format:
  --   <pkg>:
  --     Installed: 1.78.1
  --     Candidate: 1.80.0
  --     Version table: ...
  -- If Candidate ≠ Installed AND Candidate ≠ "(none)" → upgradable.
  -- Trusts apt's index; freshness is the caller's job (e.g. weekly
  -- check_updates_all should run apt-get update before invoking query).
  if row.installed then
    local pol = target:exec(("apt-cache policy %q 2>/dev/null"):format(pkg_name), {})
    if pol and pol.status == 0 and pol.stdout then
      local candidate = pol.stdout:match("Candidate:%s*(%S+)")
      if candidate and candidate ~= "(none)" then
        row.available  = candidate
        row.upgradable = M.version.cmp(row.version, candidate) < 0
      end
    end
  end
  return row
end

-- Idempotent install of apt source list + key into /etc/apt/sources.list.d
-- and /usr/share/keyrings via sudo install(1). Replaces the apt.add_source
-- Rust builtin so the framework works as a non-root user with sudoers.
-- Reads-existing-content for diff (files are world-readable mode 644).
local function apt_add_source_via_sudo(entry, log)
  local b   = entry.apt
  local id  = safe_name(entry.id)
  local sudo = sudo_prefix()
  local list_dst = "/etc/apt/sources.list.d/" .. id .. ".list"
  local key_dst  = "/usr/share/keyrings/" .. id .. ".gpg"

  -- Download key into user-owned tmp.
  local key_tmp = "/tmp/assay-pkg-key-" .. id .. ".gpg"
  http.download(b.key_url, key_tmp, { timeout = 30 })

  local changed = false

  -- Idempotent key install. read_disk uses io.open in binary mode so it's
  -- binary-safe AND bypasses the consumer's FileSource (fs.read/read_bytes
  -- via LayeredFs can't reach /tmp or /usr/share/keyrings/).
  local want_key = read_disk(key_tmp, "rb")
  local cur_key  = read_disk(key_dst, "rb")
  if cur_key ~= want_key then
    local r = shell.exec(
      sudo .. ("install -D -m 0644 -o root -g root %q %q"):format(key_tmp, key_dst), {})
    if not r or r.status ~= 0 then
      fs.remove(key_tmp)
      error(("install %s -> %s failed: %s"):format(key_tmp, key_dst, (r and r.stderr) or "unknown"))
    end
    changed = true
  end
  fs.remove(key_tmp)

  -- Idempotent list install. read_disk for symmetry with the key path
  -- and FileSource bypass.
  local want_list = b.source_list .. "\n"
  local cur_list  = read_disk(list_dst, "rb")
  if cur_list ~= want_list then
    local list_tmp = "/tmp/assay-pkg-list-" .. id .. ".list"
    fs.write(list_tmp, want_list)
    local r = shell.exec(
      sudo .. ("install -D -m 0644 -o root -g root %q %q"):format(list_tmp, list_dst), {})
    fs.remove(list_tmp)
    if not r or r.status ~= 0 then
      error(("install list -> %s failed: %s"):format(list_dst, (r and r.stderr) or "unknown"))
    end
    changed = true
  end

  log("  source: " .. (changed and "wrote" or "unchanged"))
  return changed
end

-- Container-side apt source management. We can't run apt.add_source against
-- /etc/apt/sources.list.d on the host's path tree because that's outside the
-- container's filesystem — and the host-side sudoers entry doesn't help us
-- write inside a running nspawn machine. Instead: download the key on the
-- host, base64-encode it for shell-safe transport, then run install(1)
-- inside the container via target:exec.
local function apt_add_source_in_machine(target, entry, log)
  local b = entry.apt
  local id = safe_name(entry.id)
  local list_dst = "/etc/apt/sources.list.d/" .. id .. ".list"
  local key_dst  = "/usr/share/keyrings/" .. id .. ".gpg"

  local key_tmp = "/tmp/assay-pkg-key-" .. id .. ".gpg"
  http.download(b.key_url, key_tmp, { timeout = 30 })

  -- base64-encode the key on host (shell.exec stdin can't carry binary
  -- because mlua's String boundary requires UTF-8). base64 output is ASCII,
  -- safe to embed in a shell command line.
  local b64r = shell.exec(("base64 -w 0 %q"):format(key_tmp), {})
  fs.remove(key_tmp)
  if not b64r or b64r.status ~= 0 then
    error("base64 encode failed for key: " .. ((b64r and b64r.stderr) or "unknown"))
  end
  local b64 = (b64r.stdout or ""):gsub("%s+$", "")

  -- Write key inside container.
  local cmd_key = ("echo %s | base64 -d | install -D -m 0644 /dev/stdin %s"):format(
    shell_quote(b64), shell_quote(key_dst))
  local r1 = target:exec(cmd_key, { timeout = 60 })
  if not r1 or r1.status ~= 0 then
    error("install key in " .. target.id .. " failed: " .. ((r1 and r1.stderr) or "unknown"))
  end

  -- Write source list inside container (text — direct echo, no encoding).
  local list_content = b.source_list .. "\n"
  local cmd_list = ("printf '%%s' %s | install -D -m 0644 /dev/stdin %s"):format(
    shell_quote(list_content), shell_quote(list_dst))
  local r2 = target:exec(cmd_list, { timeout = 60 })
  if not r2 or r2.status ~= 0 then
    error("install list in " .. target.id .. " failed: " .. ((r2 and r2.stderr) or "unknown"))
  end

  -- Refresh container's apt cache so apt-get install can find the new package.
  local r3 = target:exec("apt-get update -qq", { timeout = 300 })
  if not r3 or r3.status ~= 0 then
    error("apt-get update in " .. target.id .. " failed: " .. ((r3 and r3.stderr) or "unknown"))
  end
  log("  source: installed in " .. target.id)
end

function M.method.apt.install(target, entry, ctx)
  if not (entry and entry.apt) then error("apt block missing on entry " .. tostring(entry and entry.id)) end
  local log = ctx_log(ctx)
  local b   = entry.apt

  -- Source/key management. On the host we use the local sudo install path;
  -- on a machine target we transport into the container via target:exec.
  if target.kind == "host" and b.source_list and b.key_url then
    local changed = apt_add_source_via_sudo(entry, log)
    if changed then
      local up = shell.exec(sudo_prefix() .. "apt-get update", { timeout = 300 })
      if not up or up.status ~= 0 then
        error("apt-get update failed: " .. ((up and up.stderr) or "unknown"))
      end
    end
  elseif target.kind == "machine" and b.source_list and b.key_url then
    apt_add_source_in_machine(target, entry, log)
  end

  -- Apt-get install. `env DEBIAN_FRONTEND=noninteractive` survives sudo's
  -- env-stripping (sudo runs `env`, env sets the var for apt-get).
  local extra = (ctx and ctx.op == "upgrade") and " --only-upgrade" or ""
  if target.kind == "host" then
    -- `--` ends apt-get option parsing so package names beginning with `-`
    -- can't inject flags like --allow-unauthenticated.
    local cmd = sudo_prefix() ..
      ("env DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends%s -- %q"):format(
        extra, b.package_name)
    local r = shell.exec(cmd, { timeout = 600 })
    if not r or r.status ~= 0 then
      error("apt-get install failed: " .. ((r and r.stderr) or "unknown"))
    end
  else
    -- Inside container: shell already runs as root; no sudo needed.
    -- Host-side systemd-run elevation is handled by Target:exec.
    local cmd = ("DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends%s -- %q"):format(
      extra, b.package_name)
    local r = target:exec(cmd, { timeout = 600 })
    if not r or r.status ~= 0 then
      error("apt-get install in " .. tostring(target.id) .. " failed: " .. ((r and r.stderr) or "unknown"))
    end
  end
end

function M.method.apt.remove(target, entry, ctx)
  if not (entry and entry.apt) then error("apt block missing on entry " .. tostring(entry and entry.id)) end
  local log = ctx_log(ctx)
  local b   = entry.apt

  -- Gate on installed-state. Some Debian versions return non-zero from
  -- `apt-get remove` for packages that aren't installed; checking first
  -- avoids spurious errors on idempotent re-removes.
  local q = M.method.apt.query(target, entry)
  if not q.installed then
    log("  not installed; nothing to remove")
    return
  end

  if target.kind == "host" then
    local cmd = sudo_prefix() ..
      ("env DEBIAN_FRONTEND=noninteractive apt-get remove -y -- %q"):format(b.package_name)
    local r = shell.exec(cmd, { timeout = 300 })
    if not r or r.status ~= 0 then
      error("apt-get remove failed: " .. ((r and r.stderr) or "unknown"))
    end
  else
    local cmd = ("DEBIAN_FRONTEND=noninteractive apt-get remove -y -- %q"):format(b.package_name)
    local r = target:exec(cmd, { timeout = 300 })
    if not r or r.status ~= 0 then
      error("apt-get remove in " .. tostring(target.id) .. " failed: " .. ((r and r.stderr) or "unknown"))
    end
  end
end

-- ─── binary ───

-- Detect target architecture in two flavors so catalogs can match the
-- vendor's naming convention:
--   uname_m: raw `uname -m` output (e.g. "x86_64", "aarch64") — Rust/Go style
--   arch:    Debian-style normalization ("amd64", "arm64")    — apt/dpkg style
local function arch_for(target)
  local r = target:exec("uname -m", {})
  local uname_m = (r and r.stdout or "x86_64"):gsub("%s+$", "")
  local arch = (uname_m == "x86_64" and "amd64") or
               (uname_m == "aarch64" and "arm64") or uname_m
  return arch, uname_m
end

function M.method.binary.query(target, entry)
  if not (entry and entry.binary) then return { installed = false } end
  local b = entry.binary

  local function probe_path(p)
    if not p or p == "" then return nil end
    local r1 = target:exec(("test -x %q"):format(p), {})
    if not r1 or r1.status ~= 0 then return nil end
    local r2 = target:exec(("%q --version 2>&1 || true"):format(p), {})
    local ver
    if r2 and r2.stdout then
      ver = r2.stdout:match("(%d+%.[%d%.]+)") or r2.stdout:match("(%d+%.%d+)")
    end
    return { version = ver, path = p }
  end

  -- Step 1: canonical install_path.
  local hit = probe_path(b.install_path)
  if hit then
    return { installed = true, version = hit.version, installed_at = hit.path }
  end

  -- Step 2: PATH fallback. Detection only — we still consider the package
  -- "installed" so reconcile produces work (upgrade/replace), and any apply
  -- writes to the canonical install_path regardless of where we found it.
  local cmd_name = safe_name(b.command_name or entry.id)
  if cmd_name == "" then return { installed = false } end
  local r = target:exec(("command -v %q 2>/dev/null"):format(cmd_name), {})
  if r and r.status == 0 and r.stdout then
    local found = (r.stdout:gsub("%s+$", ""))
    if found ~= "" then
      local hit2 = probe_path(found)
      if hit2 then
        return { installed = true, version = hit2.version, installed_at = hit2.path }
      end
    end
  end

  return { installed = false }
end

-- Marker path is per-target: host markers stay at the legacy <cache>/<id>/installed.json
-- so existing installs continue working; per-machine markers get a target-id suffix.
local function marker_path_for(ctx, entry, target)
  if target.kind == "host" then
    return ctx.cache_dir .. "/" .. entry.id .. "/installed.json"
  else
    return ctx.cache_dir .. "/" .. entry.id .. "/installed." .. target.id .. ".json"
  end
end

function M.method.binary.install(target, entry, ctx)
  if not (entry and entry.binary) then error("binary block missing on entry " .. tostring(entry and entry.id)) end
  if not (ctx and ctx.cache_dir) then error("ctx.cache_dir required for binary install") end
  local log = ctx_log(ctx)
  local b = entry.binary
  local installed_meta_path = marker_path_for(ctx, entry, target)

  -- Adoption policy: the framework owns any catalog-listed package. If the binary
  -- already exists (with or without marker, at any path), the install will
  -- overwrite the canonical install_path with our verified release artifact
  -- and write a marker. Subsequent runs are then idempotent via Step 2.

  -- Step 1: detect arch via the target itself (host or machine).
  local arch, uname_m = arch_for(target)

  local meta = M.release.refresh_meta(entry, ctx)
  local tag = meta.tag
  local ver = (tag or ""):gsub("^v", "")

  local asset_name = b.asset_pattern
                     :gsub("{arch}", arch)
                     :gsub("{uname_m}", uname_m)
                     :gsub("{tag}", tag)
                     :gsub("{ver}", ver)
  local asset_url
  for _, a in ipairs(meta.assets or {}) do
    if a.name == asset_name then asset_url = a.browser_download_url; break end
  end
  if not asset_url then
    error("no asset matching pattern: " .. asset_name)
  end

  -- Step 2: short-circuit if already at target version (using marker).
  -- For host: marker matches AND install_path exists locally → no-op.
  -- For machine: marker matches AND install_path exists *inside the container* → no-op.
  do
    local raw = read_disk(installed_meta_path)
    if raw then
      local ok, m = pcall(json.parse, raw)
      if ok and m and m.version == ver then
        local present
        if target.kind == "host" then
          present = fs.exists(b.install_path)
        else
          local r = target:exec(("test -x %q"):format(b.install_path), {})
          present = r and r.status == 0
        end
        if present then
          log("  no-op: already at " .. ver)
          return { skipped = true, reason = "already at " .. ver, noop = true }
        end
      end
    end
  end

  -- Step 3: download asset.
  fs.mkdir(ctx.cache_dir .. "/" .. entry.id)
  local asset_path = ctx.cache_dir .. "/" .. entry.id .. "/" .. asset_name
  http.download(asset_url, asset_path, { timeout = 300 })
  log("  downloaded " .. asset_name)

  -- Step 4: verify sha256.
  local expected_sha = M.release.fetch_expected_sha256(b, meta, asset_name)
  if not expected_sha then
    error("sha256 not available for " .. asset_name)
  end
  local actual_sha = crypto.hash_file(asset_path, "sha256")
  if actual_sha ~= expected_sha then
    error(("sha256 mismatch: expected %s got %s"):format(expected_sha, actual_sha))
  end
  log("  sha256 ok")

  -- Step 5: extract or use directly.
  local source_path
  if b.archive_member then
    local extracted = ctx.cache_dir .. "/" .. entry.id .. "/" .. ver .. ".bin"
    local member = b.archive_member
                   :gsub("{arch}", arch)
                   :gsub("{uname_m}", uname_m)
                   :gsub("{tag}", tag)
                   :gsub("{ver}", ver)
    compress.untar(asset_path, extracted, { member = member })
    source_path = extracted
  else
    source_path = asset_path
  end

  -- Step 6: atomic install. Two paths:
  --   host    → sudo install -D -m <mode> <source> <dst> on host
  --   machine → stream <source> bytes via target:exec stdin into install -D
  --             inside the container (binary stdin works because shell.exec
  --             and systemd.machine_exec both accept binary stdin payloads)
  if target.kind == "host" then
    local sudo = sudo_prefix()
    local owner_args = is_root() and "" or "-o root -g root "
    local cmd = sudo ..
      ("install -D -m %s %s%q %q"):format(b.mode, owner_args, source_path, b.install_path)
    local r = shell.exec(cmd, {})
    if not r or r.status ~= 0 then
      error(("install %s -> %s failed: %s"):format(source_path, b.install_path,
                                                    (r and r.stderr) or "unknown"))
    end
  else
    -- Use `machinectl copy-to` to transfer the binary into the container.
    --
    -- Earlier versions streamed bytes via target:exec stdin → install -D
    -- /dev/stdin. That worked for small payloads but broke with EPIPE on
    -- larger binaries (rustic ~22 MB) when the multi-layer pipe path
    --   shell.exec → /bin/sh → sudo → systemd-run --pipe → /bin/sh → install
    -- closed early in the unprivileged-elevation branch. machinectl copy-to
    -- avoids the pipe chain entirely; it's binary-safe regardless of size.
    local sudo = is_root() and "" or "sudo -n "
    local install_dir = b.install_path:match("^(.*)/[^/]+$") or "/"

    -- Ensure parent dir exists in container (machinectl copy-to does NOT
    -- auto-create intermediate directories).
    local mkdir_r = target:exec(("mkdir -p %q"):format(install_dir), { timeout = 30 })
    if not mkdir_r or mkdir_r.status ~= 0 then
      error(("mkdir -p %s in %s failed: %s"):format(
        install_dir, target.id, (mkdir_r and mkdir_r.stderr) or "unknown"))
    end

    local copy_cmd = ("%smachinectl copy-to %s %s %s"):format(
      sudo, shell_quote(target.id), shell_quote(source_path), shell_quote(b.install_path))
    local r = shell.exec(copy_cmd, { timeout = 300 })
    if not r or r.status ~= 0 then
      -- Treat "File exists" as success — machinectl copy-to refuses to
      -- overwrite, but install_path may already hold our verified binary
      -- from a prior partial run. Step 7 below will re-record the marker.
      local stderr = (r and r.stderr) or "unknown"
      if not stderr:lower():find("file exists", 1, true) then
        error(("machinectl copy-to %s -> %s in %s failed: %s"):format(
          source_path, b.install_path, target.id, stderr))
      end
    end

    -- machinectl copy-to preserves source mode but ours is u+rw on the host
    -- cache; explicitly chmod to the catalog-declared mode (typically 0755).
    local chmod_r = target:exec(("chmod %s %q"):format(b.mode, b.install_path), { timeout = 30 })
    if not chmod_r or chmod_r.status ~= 0 then
      error(("chmod %s on %s in %s failed: %s"):format(
        b.mode, b.install_path, target.id, (chmod_r and chmod_r.stderr) or "unknown"))
    end
  end
  log("  installed at " .. b.install_path .. " (mode " .. b.mode .. ")")

  -- Step 7: record installed metadata.
  -- sha256 is the hash of the *installed file*. For host targets, hash the
  -- on-disk binary at install_path. For machine targets, install_path is
  -- container-internal and crypto.hash_file (host fs) can't see it; instead
  -- hash source_path which is byte-identical (install -D copies stdin
  -- verbatim). Either way, sha256 in the marker matches what binary.remove
  -- will read from the same target later.
  local installed_sha
  if target.kind == "host" then
    installed_sha = crypto.hash_file(b.install_path, "sha256")
  else
    installed_sha = crypto.hash_file(source_path, "sha256")
  end
  local installed_doc = {
    version = ver,
    sha256 = installed_sha,
    asset_sha256 = actual_sha,
    method = "binary",
    installed_at = os.date("!%Y-%m-%dT%H:%M:%SZ"),
    from_url = asset_url,
  }
  fs.write(installed_meta_path, json.encode(installed_doc))
end

function M.method.binary.remove(target, entry, ctx)
  if not (entry and entry.binary) then error("binary block missing on entry " .. tostring(entry and entry.id)) end
  if not (ctx and ctx.cache_dir) then error("ctx.cache_dir required for binary remove") end
  local log = ctx_log(ctx)
  local installed_meta_path = marker_path_for(ctx, entry, target)
  -- Existence check is per-target.
  local present
  if target.kind == "host" then
    present = fs.exists(entry.binary.install_path)
  else
    local r = target:exec(("test -x %q"):format(entry.binary.install_path), {})
    present = r and r.status == 0
  end
  if not present then
    log("  not installed; nothing to remove")
    return
  end
  -- Safety: if a marker is present, verify the installed binary's sha
  -- matches what we recorded. Mismatch means someone replaced our binary
  -- — refuse to delete blind. Missing marker means we never claimed this
  -- specific install; per the adoption policy, we still own catalog
  -- packages, so just remove. The next operation re-establishes ownership.
  local marker_raw = read_disk(installed_meta_path)
  if marker_raw then
    local ok, m = pcall(json.parse, marker_raw)
    if not ok then
      error("refusing to remove: marker file " .. installed_meta_path ..
            " is corrupt (json parse failed: " .. tostring(m) .. ")")
    end
    if type(m) == "table" and type(m.sha256) == "string" then
      -- Hash the binary on the right side of the boundary. For host, that's
      -- crypto.hash_file directly. For machine, run sha256sum inside the
      -- container so we read the file at install_path INSIDE the container,
      -- not the host filesystem.
      local actual
      if target.kind == "host" then
        actual = crypto.hash_file(entry.binary.install_path, "sha256")
      else
        local r = target:exec(("sha256sum %q"):format(entry.binary.install_path), {})
        if not r or r.status ~= 0 then
          error("refusing to remove: sha256sum in " .. target.id .. " failed: "
                .. ((r and r.stderr) or "unknown"))
        end
        actual = (r.stdout or ""):match("^(%x+)")
      end
      if actual ~= m.sha256 then
        error("refusing to remove: binary at " .. entry.binary.install_path ..
              " was modified outside our control (sha mismatch)")
      end
    end
  end
  -- No marker → unclaimed. Remove anyway: this is a catalog-listed package,
  -- so per adoption policy we own it. The marker file (if any) is cleaned
  -- below.
  if target.kind == "host" then
    local r = shell.exec(sudo_prefix() .. ("rm -f %q"):format(entry.binary.install_path), {})
    if not r or r.status ~= 0 then
      error(("rm %s failed: %s"):format(entry.binary.install_path, (r and r.stderr) or "unknown"))
    end
  else
    local r = target:exec(("rm -f %q"):format(entry.binary.install_path), { timeout = 30 })
    if not r or r.status ~= 0 then
      error(("rm %s in %s failed: %s"):format(entry.binary.install_path, target.id,
                                               (r and r.stderr) or "unknown"))
    end
  end
  -- Clean the marker too so future adopt-on-install logic starts fresh.
  if fs.exists(installed_meta_path) then fs.remove(installed_meta_path) end
  log("  removed " .. entry.binary.install_path)
end

-- ── query / apply (delegating to method handlers) ───────────────────────────

local function method_for(entry)
  if not entry or type(entry.methods) ~= "table" then return nil, nil end
  local name = entry.methods[1]
  return M.method[name], name
end

--- Probe a single catalog entry's installed state on the target.
---
--- Augmentation rules:
---   apt method   → handler.query already populated available/upgradable from apt-cache policy
---   binary method → augment with release_meta cache (and flag external if no marker)
function M.query(target, entry, ctx)
  local handler, method_name = method_for(entry)
  if not handler then return { installed = false } end
  local row = handler.query(target, entry)

  if ctx and ctx.cache_dir and row.installed and method_name == "binary" then
    -- Available/upgradable from release_meta cache. We compare the running
    -- binary's --version output to the cached upstream tag — works for
    -- framework-managed and externally-installed binaries alike.
    local meta = M.release.read_meta(entry.id, ctx)
    if meta then
      row.available  = meta.ver or meta.tag
      row.upgradable = (row.version
                       and row.available
                       and M.version.cmp(row.version, row.available) < 0) or false
    end

    -- Adoption: if the binary exists on disk but the framework has no marker for
    -- it on this target, surface as upgradable so the next Reconcile/Update
    -- emits an op that claims it (download + atomic install + write marker).
    local marker = marker_path_for(ctx, entry, target)
    if not fs.exists(marker) then
      row.upgradable = true
      row.unmanaged  = true
    end
  end
  return row
end

--- Probe every catalog entry on the target. Returns { [id] = row }.
function M.query_all(target, catalog_entries, ctx)
  local out = {}
  for id, entry in pairs(catalog_entries) do
    out[id] = M.query(target, entry, ctx)
  end
  return out
end

--- Apply a plan (array of {op, id, method, ...}) to the target.
--- Returns { ok = [...], skipped = [...], failed = [...] }.
---
--- Outcome buckets:
---   ok      - handler ran and changed disk state (or completed cleanly)
---   skipped - handler deliberately did nothing (adoption guard, no-op upgrade)
---   failed  - handler raised an error
---
--- A handler signals "skipped" by returning a table { skipped = true,
--- reason = "...", noop = bool? }. Anything else (nil, true, etc.) is "ok".
function M.apply(plan, target, catalog_entries, ctx)
  ctx = ctx or {}
  -- Optional progress callback for in-flight UI cards. Signature:
  --   on_progress(op_index, op_table, status, msg?)
  -- where status ∈ {"start", "ok", "skipped", "failed"}. Called once per op.
  local on_progress = ctx.on_progress or function(_,_,_,_) end
  local result = { ok = {}, skipped = {}, failed = {} }
  for i, op in ipairs(plan) do
    local entry = catalog_entries[op.id]
    local handler = method_for(entry)
    local fn
    if handler then
      if op.op == "install" or op.op == "upgrade" then fn = handler.install
      elseif op.op == "remove" then fn = handler.remove end
    end
    on_progress(i, op, "start")
    if not fn then
      local err = "unsupported (op=" .. tostring(op.op) ..
                  ", method=" .. tostring(op.method) .. ")"
      result.failed[#result.failed+1] = { op = op, error = err }
      on_progress(i, op, "failed", err)
    else
      local op_ctx = setmetatable({ op = op.op }, { __index = ctx })
      local ok, ret = pcall(fn, target, entry, op_ctx)
      if not ok then
        result.failed[#result.failed+1] = { op = op, error = tostring(ret or "unknown") }
        on_progress(i, op, "failed", tostring(ret or "unknown"))
      elseif type(ret) == "table" and ret.skipped then
        result.skipped[#result.skipped+1] = {
          op = op, reason = ret.reason or "skipped", noop = ret.noop or false,
        }
        on_progress(i, op, "skipped", ret.reason or "skipped")
      else
        result.ok[#result.ok+1] = op
        on_progress(i, op, "ok")
      end
    end
  end
  return result
end

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
      elseif act.upgradable
             or (act.available and act.version
                 and M.version.cmp(act.version, act.available) < 0) then
        -- Two ways to get an upgrade op:
        --   1. The query layer set upgradable=true (e.g. for adoption when a
        --      binary exists without our marker).
        --   2. Plain version comparison: installed < available.
        -- Either way, run the install/upgrade path.
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

-- M.reconcile (audit + lock + log-file orchestration) stays in the caller
-- (the caller's package-mgmt service module). The framework provides the pure
-- pieces; the caller composes them with its own state/audit pipeline.

return M
