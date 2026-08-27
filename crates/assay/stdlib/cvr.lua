--- @module assay.cvr
--- @description Denmark's CVR (Det Centrale Virksomhedsregister) via the public cvrapi.dk gateway — Danish and Norwegian entities by name, VAT number or phone. Keyless, but rate-limited and courtesy-bound to identify the caller.
--- @category registries
--- @icon building
--- @keywords cvr, denmark, danish, registry, company, prospect, vat, cvrapi
--- @quickref M.client(opts?) -> c | opts.user_agent identifies the caller; required
--- @quickref c:search(name, opts?) -> company|nil | Best match by company name
--- @quickref c:get(vat, opts?) -> company|nil | One entity by CVR/VAT number
--- @quickref c:by_phone(phone, opts?) -> company|nil | Reverse lookup by phone number

local M = {}

local lp = require("assay.lead_provider")
local url = require("assay.url")

local PROVIDER = "registry:cvr"

local function trim(s) return (tostring(s or ""):gsub("^%s+", ""):gsub("%s+$", "")) end

-- CVR reports "04/12 - 2013". Left as-is it would sort and compare wrongly
-- against every other registry's ISO date, so it is converted rather than
-- passed through.
local function iso_date(value)
  local d, m, y = trim(value):match("^(%d%d)/(%d%d)%s*%-%s*(%d%d%d%d)$")
  if not d then return nil end
  return y .. "-" .. m .. "-" .. d
end

local function normalize(e, from)
  e = e or {}
  return lp.company(PROVIDER, from, {
    registry_id = e.vat and tostring(e.vat) or nil,
    name = e.name,
    status = e.enddate and "CLOSED" or (e.creditbankrupt and "BANKRUPT" or "ACTIVE"),
    legal_form = e.companydesc,
    jurisdiction = tostring(e.country or "DK"):upper(),
    city = e.city,
    country = tostring(e.country or "DK"):upper(),
    industry = e.industrydesc,
    industry_code = e.industrycode and tostring(e.industrycode) or nil,
    employees = tonumber(e.employees),
    phone = e.phone,
    founded_at = iso_date(e.startdate),
  })
end

--- The gateway is free and unauthenticated but asks callers to identify
--- themselves, and throttles those who do not. Refusing to construct without a
--- User-Agent keeps that courtesy structural rather than optional — the same
--- stance `assay.edgar` takes for the SEC's fair-access rule.
function M.client(opts)
  opts = opts or {}
  local user_agent = opts.user_agent or env.get("CVR_USER_AGENT")
  if not user_agent or trim(user_agent) == "" then
    error("cvr: opts.user_agent required — cvrapi.dk asks callers to identify themselves")
  end
  local base_url = (opts.base_url or "https://cvrapi.dk/api"):gsub("/+$", "")
  local country = tostring(opts.country or "dk"):lower()

  local function api_get(params)
    local target = base_url .. "?" .. params .. "&country=" .. url.encode(country)
    local resp = http.get(target, {
      headers = { Accept = "application/json", ["User-Agent"] = user_agent },
    })
    -- The gateway answers 404 for "no such company", which is an answer and
    -- not a failure; only a throttle or an outage is worth raising.
    if resp.status == 404 then return nil, target end
    if resp.status == 429 then error("cvr: rate limited (HTTP 429)") end
    if resp.status ~= 200 then
      error("cvr: GET HTTP " .. resp.status .. ": " .. (resp.body or ""))
    end
    local ok, parsed = pcall(json.parse, resp.body or "")
    if not ok then error("cvr: response was not JSON") end
    if parsed and parsed.error then return nil, target end
    return parsed, target
  end

  local c = {}

  local function lookup(key, value)
    local body, from = api_get(key .. "=" .. url.encode(trim(value)))
    if not body or not body.vat then return nil end
    return normalize(body, from)
  end

  function c:search(name) return lookup("search", name) end
  function c:get(vat) return lookup("vat", tostring(vat):gsub("%s", "")) end
  function c:by_phone(phone) return lookup("phone", tostring(phone):gsub("%s", "")) end

  return c
end

return M
