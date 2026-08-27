--- @module assay.gleif
--- @description GLEIF LEI registry — search global legal entities by name, fetch a record by LEI, fuzzy-complete names. Free, keyless, every fact carries provenance.
--- @category Cloud & AWS
--- @icon building
--- @keywords gleif, lei, legal entity, registry, company, prospect, lookup, ownership
--- @quickref M.client(opts?) -> c | Client; base_url override for tests
--- @quickref c:search(name, opts?) -> [entity] | Entities whose legal name matches
--- @quickref c:fuzzy(name, opts?) -> [string] | Name completions for a partial query
--- @quickref c:get(lei) -> entity|nil | One record by LEI, nil when unknown

local M = {}

local function trim(s) return (tostring(s or ""):gsub("^%s+", ""):gsub("%s+$", "")) end

local function urlencode(str)
  return tostring(str):gsub("([^%w%-%.%_%~ ])", function(ch)
    return string.format("%%%02X", string.byte(ch))
  end):gsub(" ", "%%20")
end

-- One JSON:API lei-record → the flat shape every registry module returns, so
-- downstream code never learns per-registry envelopes.
local function normalize(record, from_url)
  local a = record.attributes or {}
  local entity = a.entity or {}
  local name = entity.legalName or {}
  local address = entity.legalAddress or {}
  local registration = a.registration or {}
  return {
    lei = a.lei or record.id,
    name = name.name,
    status = entity.status,
    jurisdiction = entity.jurisdiction,
    legal_form = entity.legalForm and entity.legalForm.id or nil,
    city = address.city,
    country = address.country,
    registered_at = registration.initialRegistrationDate,
    provenance = { provider = "registry:gleif", retrieved_from = from_url },
  }
end

function M.client(opts)
  opts = opts or {}
  local base_url = (opts.base_url or "https://api.gleif.org/api/v1"):gsub("/+$", "")

  local function api_get(path_str)
    local url = base_url .. path_str
    local resp = http.get(url, { headers = { Accept = "application/vnd.api+json" } })
    if resp.status == 404 then return nil, url end
    if resp.status ~= 200 then
      error("gleif: GET " .. path_str .. " HTTP " .. resp.status .. ": " .. (resp.body or ""))
    end
    return json.parse(resp.body), url
  end

  local c = {}

  function c:search(name, o)
    o = o or {}
    local path_str = "/lei-records?filter%5Bentity.legalName%5D=" .. urlencode(trim(name))
      .. "&page%5Bsize%5D=" .. tostring(o.limit or 10)
    local body, url = api_get(path_str)
    local out = {}
    for _, record in ipairs(body and body.data or {}) do
      out[#out + 1] = normalize(record, url)
    end
    return out
  end

  function c:fuzzy(name, o)
    o = o or {}
    local path_str = "/fuzzycompletions?field=entity.legalName&q=" .. urlencode(trim(name))
    local body = api_get(path_str)
    local out = {}
    for _, item in ipairs(body and body.data or {}) do
      local a = item.attributes or {}
      out[#out + 1] = a.value
    end
    return out
  end

  function c:get(lei)
    local body, url = api_get("/lei-records/" .. urlencode(trim(lei)))
    if not body or not body.data then return nil end
    return normalize(body.data, url)
  end

  return c
end

return M
