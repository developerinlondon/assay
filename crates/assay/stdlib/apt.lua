--- @module assay.apt
--- @description Debian/Ubuntu apt package index reader. Fetches and parses Packages indexes (gz/xz/zstd/plain) from any apt-style HTTP repository.
--- @keywords apt, debian, ubuntu, packages, package, dpkg, repository, deb, version, index
--- @quickref apt.packages(opts) -> idx | Fetch and parse a Packages index
--- @quickref idx:find(name) -> pkg | nil | Look up a package by name
--- @quickref pkg.version | Newest version (Debian-sorted)
--- @quickref pkg.versions | All versions, sorted newest first

local M = {}

-- File names tried in order. Compressed first (smaller), plain last as fallback.
local CANDIDATE_FILES = {
  { name = "Packages.gz",  decode = "gunzip" },
  { name = "Packages.xz",  decode = "unxz" },
  { name = "Packages.zst", decode = "unzstd" },
  { name = "Packages",     decode = nil },
}

local function trim(s)
  return (s:gsub("^%s+", ""):gsub("%s+$", ""))
end

local function decompress(decode, body)
  if decode == nil then return body end
  return compress[decode](body)
end

local function build_url(opts, file_name)
  local base = (opts.base_url or ""):gsub("/+$", "")
  return base
    .. "/dists/"
    .. opts.dist
    .. "/"
    .. opts.component
    .. "/binary-"
    .. opts.arch
    .. "/"
    .. file_name
end

-- Parse a Debian control file (RFC 822-style). Stanzas separated by blank lines;
-- continuation lines start with whitespace and append to the previous field's value.
local function parse_stanzas(text)
  local stanzas = {}
  local current = {}
  local last_field = nil

  -- Normalise line endings.
  text = text:gsub("\r\n", "\n")

  for line in (text .. "\n"):gmatch("([^\n]*)\n") do
    if line == "" then
      if next(current) ~= nil then
        stanzas[#stanzas + 1] = current
        current = {}
        last_field = nil
      end
    elseif line:match("^[ \t]") then
      if last_field then
        current[last_field] = current[last_field] .. "\n" .. trim(line)
      end
    else
      local field, value = line:match("^([^:]+):%s*(.*)$")
      if field then
        last_field = field
        current[field] = value
      end
    end
  end
  if next(current) ~= nil then
    stanzas[#stanzas + 1] = current
  end
  return stanzas
end

local function fetch_index_body(opts)
  local last_status, last_url
  for _, candidate in ipairs(CANDIDATE_FILES) do
    local url = build_url(opts, candidate.name)
    local resp = http.get(url)
    if resp.status == 200 then
      return decompress(candidate.decode, resp.body)
    end
    last_status, last_url = resp.status, url
  end
  error(
    "apt.packages: no Packages index found (last tried "
      .. (last_url or "?")
      .. " HTTP "
      .. tostring(last_status or "?")
      .. ")"
  )
end

local function sort_versions_desc(versions)
  local version = require("assay.version")
  table.sort(versions, function(a, b)
    return version.compare(a, b, "debian") > 0
  end)
  return versions
end

local function build_index(stanzas)
  local by_name = {}
  for _, s in ipairs(stanzas) do
    local name = s["Package"]
    local ver = s["Version"]
    if name and ver then
      local pkg = by_name[name]
      if not pkg then
        pkg = {
          name = name,
          versions = {},
          stanzas = {},
        }
        by_name[name] = pkg
      end
      pkg.versions[#pkg.versions + 1] = ver
      pkg.stanzas[#pkg.stanzas + 1] = s
    end
  end

  for _, pkg in pairs(by_name) do
    sort_versions_desc(pkg.versions)
    -- Newest version is first; surface its stanza fields on the package itself.
    local newest = pkg.versions[1]
    local newest_stanza
    for _, s in ipairs(pkg.stanzas) do
      if s["Version"] == newest then
        newest_stanza = s
        break
      end
    end
    pkg.version = newest
    pkg.architecture = newest_stanza and newest_stanza["Architecture"]
    pkg.depends = newest_stanza and newest_stanza["Depends"]
    pkg.section = newest_stanza and newest_stanza["Section"]
    pkg.description = newest_stanza and newest_stanza["Description"]
    pkg.filename = newest_stanza and newest_stanza["Filename"]
    pkg.sha256 = newest_stanza and newest_stanza["SHA256"]
    pkg.size = newest_stanza and newest_stanza["Size"]
  end

  return by_name
end

--- Fetch and parse an apt Packages index.
--- @param opts table `{ base_url, dist, component, arch }`
--- @return table idx Index with `:find(name)` method.
function M.packages(opts)
  opts = opts or {}
  if not opts.base_url then error("apt.packages: opts.base_url required") end
  if not opts.dist then error("apt.packages: opts.dist required") end
  if not opts.component then error("apt.packages: opts.component required") end
  if not opts.arch then error("apt.packages: opts.arch required") end

  local body = fetch_index_body(opts)
  local stanzas = parse_stanzas(body)
  local by_name = build_index(stanzas)

  local idx = { _by_name = by_name }
  function idx:find(name)
    return self._by_name[name]
  end
  return idx
end

return M
