--- @module assay.companies_house
--- @description UK Companies House — every registered UK company and its officers. The only registry module that needs a key; it is free, but issued per caller.
--- @category registries
--- @icon building
--- @keywords companies house, uk, britain, registry, company, prospect, officers, directors, sic
--- @quickref M.client(opts?) -> c | api_key via opts.api_key or COMPANIES_HOUSE_KEY
--- @quickref c:search(name, opts?) -> [company] | Companies whose name matches
--- @quickref c:get(number) -> company|nil | Full profile by company number
--- @quickref c:officers(number, opts?) -> [person] | Directors and secretaries

local M = {}

local lp = require("assay.lead_provider")
local url = require("assay.url")

local PROVIDER = "registry:companies_house"

local function trim(s) return (tostring(s or ""):gsub("^%s+", ""):gsub("%s+$", "")) end

-- Companies House publishes twelve statuses where a caller asks one question:
-- can this company still be approached. The three buckets are the vocabulary
-- the other registry modules already answer in, so a UK row and a Norwegian
-- row compare directly.
local STATUS = {
  active = "ACTIVE",
  open = "ACTIVE",
  registered = "ACTIVE",
  dissolved = "CLOSED",
  closed = "CLOSED",
  removed = "CLOSED",
  ["converted-closed"] = "CLOSED",
  liquidation = "LIQUIDATING",
  receivership = "LIQUIDATING",
  administration = "LIQUIDATING",
  ["voluntary-arrangement"] = "LIQUIDATING",
  ["insolvency-proceedings"] = "LIQUIDATING",
}

local function status_of(value)
  local raw = trim(value):lower()
  if raw == "" then return nil end
  -- An unmapped status is upper-cased rather than dropped: the registry added a
  -- value we do not know, and inventing ACTIVE for it would be the one wrong answer.
  return STATUS[raw] or raw:upper():gsub("%-", "_")
end

--- Search hits and company profiles describe the same entity under different
--- field names — `title`/`company_name`, `company_type`/`type`,
--- `address`/`registered_office_address`. Reading only one set silently yields
--- a record with a nil name from the other endpoint.
local function normalize(e, from)
  e = e or {}
  local addr = e.registered_office_address or e.address or {}
  local sic = e.sic_codes or {}
  return lp.company(PROVIDER, from, {
    registry_id = e.company_number,
    name = e.company_name or e.title,
    status = status_of(e.company_status),
    legal_form = e.type or e.company_type,
    jurisdiction = "GB",
    city = addr.locality,
    country = addr.country,
    industry_code = sic[1],
    founded_at = e.date_of_creation,
    registered_at = e.date_of_creation,
  })
end

-- Officers are withheld a day of birth by design, so the date is a real
-- year-month and not a truncated day. Rendering it as a partial ISO date keeps
-- it comparable without claiming a precision the registry refuses to give.
local function born_at(dob)
  if type(dob) ~= "table" or not dob.year or not dob.month then return nil end
  return string.format("%04d-%02d", tonumber(dob.year), tonumber(dob.month))
end

local function normalize_officer(o, from)
  o = o or {}
  local person = lp.person(PROVIDER, from, {
    full_name = o.name,
    title = o.occupation,
    location = o.country_of_residence,
  })
  person.officer_role = o.officer_role
  person.appointed_on = o.appointed_on
  person.resigned_on = o.resigned_on
  person.nationality = o.nationality
  person.born_at = born_at(o.date_of_birth)
  -- The list mixes serving and departed officers; only the serving ones are a
  -- contact, and the distinction is carried solely by a resignation date.
  person.active = o.resigned_on == nil
  return person
end

--- Unlike every other registry module here, Companies House issues a key. It
--- is free and self-service, but the API rejects an unauthenticated call, so
--- the client refuses to construct without one rather than failing later with
--- a 401 that reads like an outage.
function M.client(opts)
  opts = opts or {}
  local api_key = opts.api_key or env.get("COMPANIES_HOUSE_KEY")
  if not api_key or trim(api_key) == "" then
    error("companies_house: api_key required (opts.api_key or COMPANIES_HOUSE_KEY)")
  end
  local base_url = (opts.base_url or "https://api.company-information.service.gov.uk"):gsub("/+$", "")

  -- The key is the Basic username and the password is empty, so the trailing
  -- colon is load-bearing: dropping it authenticates as a user named after the
  -- key with no password, which the API rejects as a bad key.
  local auth = "Basic " .. base64.encode(trim(api_key) .. ":")

  local function api_get(path_str)
    local target = base_url .. path_str
    local resp = http.get(target, {
      headers = { Authorization = auth, Accept = "application/json" },
    })
    if resp.status == 404 then return nil, target end
    if resp.status == 401 or resp.status == 403 then
      error("companies_house: GET " .. path_str .. " rejected the api_key (HTTP " .. resp.status .. ")")
    end
    if resp.status == 429 then
      error("companies_house: rate limited (HTTP 429) — 600 requests per 5 minutes")
    end
    if resp.status ~= 200 then
      error("companies_house: GET " .. path_str .. " HTTP " .. resp.status .. ": " .. (resp.body or ""))
    end
    local ok, parsed = pcall(json.parse, resp.body or "")
    if not ok then error("companies_house: " .. path_str .. " returned unparseable JSON") end
    return parsed, target
  end

  local function number_of(company_number)
    local id = trim(company_number):gsub("%s", "")
    if id == "" then error("companies_house: a company number is required") end
    return url.encode(id)
  end

  local c = {}

  function c:search(name, o)
    o = o or {}
    local body, from = api_get("/search/companies?q=" .. url.encode(trim(name))
      .. "&items_per_page=" .. tostring(o.limit or 20))
    local out = {}
    for _, item in ipairs(body and body.items or {}) do
      out[#out + 1] = normalize(item, from)
    end
    return out
  end

  function c:get(company_number)
    local body, from = api_get("/company/" .. number_of(company_number))
    if not body or not body.company_number then return nil end
    return normalize(body, from)
  end

  --- The reason to reach a UK registry at all for outreach: the profile names
  --- the company, this names the person to write to.
  function c:officers(company_number, o)
    o = o or {}
    local path_str = "/company/" .. number_of(company_number)
      .. "/officers?items_per_page=" .. tostring(o.limit or 35)
    if o.register_type then path_str = path_str .. "&register_type=" .. url.encode(o.register_type) end
    local body, from = api_get(path_str)
    local out = {}
    for _, item in ipairs(body and body.items or {}) do
      local person = normalize_officer(item, from)
      if not o.active_only or person.active then out[#out + 1] = person end
    end
    return out
  end

  return c
end

return M
