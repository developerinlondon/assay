--- @module assay.version
--- @description Cross-scheme version comparison: semver, debian, rpm, plain numeric. Pure Lua.
--- @keywords version, semver, debian, rpm, compare, sort
--- @quickref version.compare(a, b, scheme?) -> -1|0|1 | Compare two versions; default scheme = "semver"
--- @quickref version.max(list, scheme?) -> string | Return the largest version in the list

local M = {}

local function sign(n)
  if n < 0 then return -1 end
  if n > 0 then return 1 end
  return 0
end

-- ===== semver =====

local function strip_v(s)
  local rest = s:match("^[vV](.*)$")
  return rest or s
end

-- Split a semver-style version string into {main, pre} where main is the
-- dotted core (1.2.3) and pre is the pre-release identifier list (or nil).
-- Build metadata (after '+') is discarded for ordering per semver spec.
local function semver_split(s)
  s = strip_v(s)
  local plus = s:find("+", 1, true)
  if plus then s = s:sub(1, plus - 1) end

  local main, pre = s, nil
  local dash = s:find("-", 1, true)
  if dash then
    main = s:sub(1, dash - 1)
    pre = s:sub(dash + 1)
  end

  local main_parts = {}
  for part in main:gmatch("[^%.]+") do
    main_parts[#main_parts + 1] = part
  end

  local pre_parts = nil
  if pre and pre ~= "" then
    pre_parts = {}
    for part in pre:gmatch("[^%.]+") do
      pre_parts[#pre_parts + 1] = part
    end
  end

  return main_parts, pre_parts
end

local function cmp_main_segment(a, b)
  local an = tonumber(a)
  local bn = tonumber(b)
  if an and bn then return sign(an - bn) end
  if a == b then return 0 end
  if a < b then return -1 end
  return 1
end

local function cmp_pre_id(a, b)
  local an = tonumber(a)
  local bn = tonumber(b)
  if an and bn then return sign(an - bn) end
  -- Numeric identifiers always have lower precedence than alphanumeric.
  if an and not bn then return -1 end
  if bn and not an then return 1 end
  if a == b then return 0 end
  if a < b then return -1 end
  return 1
end

local function cmp_semver(a, b)
  local am, ap = semver_split(a)
  local bm, bp = semver_split(b)

  local n = math.max(#am, #bm)
  for i = 1, n do
    local av = am[i] or "0"
    local bv = bm[i] or "0"
    local c = cmp_main_segment(av, bv)
    if c ~= 0 then return c end
  end

  -- A version with a pre-release has lower precedence than one without.
  if ap and not bp then return -1 end
  if bp and not ap then return 1 end
  if not ap and not bp then return 0 end

  local m = math.max(#ap, #bp)
  for i = 1, m do
    local av = ap[i]
    local bv = bp[i]
    if av == nil and bv ~= nil then return -1 end
    if bv == nil and av ~= nil then return 1 end
    local c = cmp_pre_id(av, bv)
    if c ~= 0 then return c end
  end
  return 0
end

-- ===== debian / rpm =====

-- Per debian-policy:
--   * tilde sorts before everything, including the empty string
--   * letters sort before non-letters
-- We implement that by mapping each character to an ordering rank, with
-- tilde getting a rank lower than the "empty / end-of-string" sentinel.
local function deb_char_rank(c, allow_tilde)
  if c == nil then return 0 end                    -- end-of-string
  if allow_tilde and c == "~" then return -1 end   -- tilde sorts before empty
  local b = string.byte(c)
  if (b >= 65 and b <= 90) or (b >= 97 and b <= 122) then
    return b                                       -- letters: keep ASCII order
  end
  return b + 256                                   -- everything else sorts after letters
end

-- Compare two non-digit runs char-by-char with debian rules.
local function cmp_deb_nondigit(a, b, allow_tilde)
  local la = #a
  local lb = #b
  local n = math.max(la, lb)
  for i = 1, n do
    local ca = i <= la and a:sub(i, i) or nil
    local cb = i <= lb and b:sub(i, i) or nil
    local ra = deb_char_rank(ca, allow_tilde)
    local rb = deb_char_rank(cb, allow_tilde)
    if ra ~= rb then return sign(ra - rb) end
  end
  return 0
end

-- Compare two digit runs numerically (ignoring leading zeros).
local function cmp_deb_digits(a, b)
  a = a:gsub("^0+", "")
  b = b:gsub("^0+", "")
  if #a ~= #b then return sign(#a - #b) end
  if a == b then return 0 end
  if a < b then return -1 end
  return 1
end

-- Core debian-style comparator for upstream/revision strings.
-- allow_tilde=true gives full debian semantics; false gives rpm-like behavior
-- where '~' is just another non-digit, non-letter character.
local function cmp_deb_part(a, b, allow_tilde)
  local i, j = 1, 1
  local la, lb = #a, #b
  while i <= la or j <= lb do
    -- consume non-digit run from each
    local na, nb = "", ""
    while i <= la and not a:sub(i, i):match("%d") do
      na = na .. a:sub(i, i)
      i = i + 1
    end
    while j <= lb and not b:sub(j, j):match("%d") do
      nb = nb .. b:sub(j, j)
      j = j + 1
    end
    if na ~= nb then
      local c = cmp_deb_nondigit(na, nb, allow_tilde)
      if c ~= 0 then return c end
    end

    -- consume digit run from each
    local da, db = "", ""
    while i <= la and a:sub(i, i):match("%d") do
      da = da .. a:sub(i, i)
      i = i + 1
    end
    while j <= lb and b:sub(j, j):match("%d") do
      db = db .. b:sub(j, j)
      j = j + 1
    end
    if da ~= db then
      local c = cmp_deb_digits(da, db)
      if c ~= 0 then return c end
    end
  end
  return 0
end

-- Split "[epoch:]upstream[-revision]" into its three parts.
local function deb_split(s, with_epoch)
  local epoch = 0
  local rest = s
  if with_epoch then
    local ep, r = s:match("^(%d+):(.*)$")
    if ep then
      epoch = tonumber(ep)
      rest = r
    end
  end
  local upstream, revision = rest, ""
  local dash = rest:find("-[^-]*$")
  if dash then
    upstream = rest:sub(1, dash - 1)
    revision = rest:sub(dash + 1)
  end
  return epoch, upstream, revision
end

local function cmp_debian(a, b)
  local ae, au, ar = deb_split(a, true)
  local be, bu, br = deb_split(b, true)
  if ae ~= be then return sign(ae - be) end
  local c = cmp_deb_part(au, bu, true)
  if c ~= 0 then return c end
  return cmp_deb_part(ar, br, true)
end

local function cmp_rpm(a, b)
  -- rpm comparison: no epoch, no tilde rule (per slice spec).
  local _, au, ar = deb_split(a, false)
  local _, bu, br = deb_split(b, false)
  local c = cmp_deb_part(au, bu, false)
  if c ~= 0 then return c end
  return cmp_deb_part(ar, br, false)
end

-- ===== numeric =====

local function cmp_numeric(a, b)
  local ap, bp = {}, {}
  for part in a:gmatch("[^%.]+") do ap[#ap + 1] = tonumber(part) or 0 end
  for part in b:gmatch("[^%.]+") do bp[#bp + 1] = tonumber(part) or 0 end
  local n = math.max(#ap, #bp)
  for i = 1, n do
    local av = ap[i] or 0
    local bv = bp[i] or 0
    if av ~= bv then return sign(av - bv) end
  end
  return 0
end

-- ===== public API =====

local function compare_with_scheme(a, b, scheme)
  if scheme == "semver" then return cmp_semver(a, b) end
  if scheme == "debian" then return cmp_debian(a, b) end
  if scheme == "rpm" then return cmp_rpm(a, b) end
  if scheme == "numeric" then return cmp_numeric(a, b) end
  error("version: unknown scheme: " .. tostring(scheme))
end

function M.compare(a, b, scheme)
  scheme = scheme or "semver"
  if type(a) ~= "string" or type(b) ~= "string" then
    error("version.compare: both arguments must be strings")
  end
  return compare_with_scheme(a, b, scheme)
end

function M.max(list, scheme)
  scheme = scheme or "semver"
  if type(list) ~= "table" then
    error("version.max: first argument must be a table")
  end
  local best = nil
  for i = 1, #list do
    local v = list[i]
    if best == nil or compare_with_scheme(v, best, scheme) > 0 then
      best = v
    end
  end
  return best
end

return M
