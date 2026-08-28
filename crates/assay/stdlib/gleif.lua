--- @module assay.gleif
--- @description GLEIF LEI registry — search global legal entities by name, fetch a record by LEI, fuzzy-complete names. Free, keyless, every record carries provenance.
--- @category registries
--- @icon building
--- @keywords gleif, lei, legal entity, registry, company, prospect, lookup, ownership
--- @quickref M.client(opts?) -> c | Client; base_url override for tests
--- @quickref c:search(name, opts?) -> [entity] | Entities whose legal name matches
--- @quickref c:fuzzy(name, opts?) -> [string] | Name completions for a partial query
--- @quickref c:get(lei) -> entity|nil | One record by LEI, nil when unknown

local M = {}

local lp = require("assay.lead_provider")
local url = require("assay.url")

local PROVIDER = "registry:gleif"

local function trim(s) return (tostring(s or ""):gsub("^%s+", ""):gsub("%s+$", "")) end

-- One JSON:API lei-record → the flat shape every registry module returns, so
-- downstream code never learns per-registry envelopes.
local function normalize(record, from_url)
  local a = record.attributes or {}
  local entity = a.entity or {}
  local name = entity.legalName or {}
  local address = entity.legalAddress or {}
  local registration = a.registration or {}
  -- GLEIF is the one registry that is not itself the register of record, so
  -- registry_id is the national number it cites, not the LEI. It cites Equinor
  -- as "923 609 016" where Brreg holds "923609016"; unstripped, the two
  -- registries never join on the company they both describe.
  local registered_as = entity.registeredAs and trim(entity.registeredAs):gsub("%s", "") or nil
  if registered_as == "" then registered_as = nil end

  local out = lp.company(PROVIDER, from_url, {
    registry_id = registered_as,
    name = name.name,
    status = entity.status,
    jurisdiction = entity.jurisdiction,
    legal_form = entity.legalForm and entity.legalForm.id or nil,
    city = address.city,
    country = address.country,
    registered_at = registration.initialRegistrationDate,
  })
  out.lei = a.lei or record.id
  return out
end

function M.client(opts)
  opts = opts or {}
  local base_url = (opts.base_url or "https://api.gleif.org/api/v1"):gsub("/+$", "")

  local function api_get(path_str)
    local target = base_url .. path_str
    local resp = http.get(target, { headers = { Accept = "application/vnd.api+json" } })
    if resp.status == 404 then return nil, target end
    if resp.status ~= 200 then
      error("gleif: GET " .. path_str .. " HTTP " .. resp.status .. ": " .. (resp.body or ""))
    end
    return json.parse(resp.body), target
  end

  local c = {}

  function c:search(name, o)
    o = o or {}
    local path_str = "/lei-records?filter%5Bentity.legalName%5D=" .. url.encode(trim(name))
      .. "&page%5Bsize%5D=" .. tostring(o.limit or 10)
    local body, from = api_get(path_str)
    local out = {}
    for _, record in ipairs(body and body.data or {}) do
      out[#out + 1] = normalize(record, from)
    end
    return out
  end

  function c:fuzzy(name, o)
    o = o or {}
    local path_str = "/fuzzycompletions?field=entity.legalName&q=" .. url.encode(trim(name))
    local body = api_get(path_str)
    local out = {}
    for _, item in ipairs(body and body.data or {}) do
      local a = item.attributes or {}
      out[#out + 1] = a.value
    end
    return out
  end

  function c:get(lei)
    lei = trim(lei)
    -- An empty LEI would hit the collection endpoint, which answers 200 with a
    -- page of records — a truthy garbage "hit" for a caller expecting nil.
    if lei == "" then return nil end
    local body, from = api_get("/lei-records/" .. url.encode(lei))
    if not body or not body.data or not body.data.attributes then return nil end
    return normalize(body.data, from)
  end

  return c
end

return M
