--- @module assay.brreg
--- @description Norway's Enhetsregisteret (Brønnøysund) — every registered Norwegian entity, keyless and free. Uniquely for a public registry it publishes the company website and a live employee count, which is what turns a registry row into a prospect.
--- @category registries
--- @icon building
--- @keywords brreg, norway, enhetsregisteret, bronnoysund, registry, company, prospect, orgnr, employees
--- @quickref M.client(opts?) -> c | Keyless; base_url override for tests
--- @quickref c:search(name, opts?) -> [company] | Entities whose name matches
--- @quickref c:get(orgnr) -> company|nil | One entity by organisation number
--- @quickref c:by_website(domain) -> [company] | Entities publishing that website
--- @quickref c:sub_entities(orgnr) -> [company] | Registered sub-entities (underenheter)

local M = {}

local lp = require("assay.lead_provider")
local url = require("assay.url")

local PROVIDER = "registry:brreg"

local function trim(s) return (tostring(s or ""):gsub("^%s+", ""):gsub("%s+$", "")) end

-- Registries publish a website as a bare hostname, sometimes with a scheme and
-- almost always with the www. A prospect list holds the apex, so normalising
-- here is what makes the two joinable at all.
local function bare_domain(value)
  local d = trim(value):lower()
  if d == "" then return nil end
  d = d:gsub("^https?://", ""):gsub("^www%.", ""):gsub("/.*$", ""):gsub(":%d+$", "")
  return d ~= "" and d or nil
end

-- Norwegian registry status is spread across three booleans rather than one
-- field. Collapsing them loses nothing a caller acts on: all three mean the
-- entity should not be approached.
local function status_of(e)
  if e.konkurs then return "BANKRUPT" end
  if e.underAvvikling then return "LIQUIDATING" end
  if e.underTvangsavviklingEllerTvangsopplosning then return "COMPULSORY_LIQUIDATION" end
  return "ACTIVE"
end

local function normalize(e, from)
  e = e or {}
  local addr = e.forretningsadresse or e.postadresse or {}
  return lp.company(PROVIDER, from, {
    registry_id = e.organisasjonsnummer,
    name = e.navn,
    domain = bare_domain(e.hjemmeside),
    status = status_of(e),
    legal_form = e.organisasjonsform and e.organisasjonsform.kode or nil,
    jurisdiction = "NO",
    city = addr.poststed,
    country = addr.landkode or addr.land,
    industry = e.naeringskode1 and e.naeringskode1.beskrivelse or nil,
    industry_code = e.naeringskode1 and e.naeringskode1.kode or nil,
    -- Only trust the count when the registry says it holds one: the field is
    -- absent for entities that never report, and 0 would be a claim we cannot make.
    employees = e.harRegistrertAntallAnsatte and e.antallAnsatte or nil,
    phone = e.telefon,
    founded_at = e.stiftelsesdato,
    registered_at = e.registreringsdatoEnhetsregisteret,
  })
end

function M.client(opts)
  opts = opts or {}
  local base_url = (opts.base_url or "https://data.brreg.no/enhetsregisteret/api"):gsub("/+$", "")

  local function api_get(path_str)
    local target = base_url .. path_str
    local resp = http.get(target, { headers = { Accept = "application/json" } })
    if resp.status == 404 then return nil, target end
    if resp.status ~= 200 then
      error("brreg: GET " .. path_str .. " HTTP " .. resp.status .. ": " .. (resp.body or ""))
    end
    local ok, parsed = pcall(json.parse, resp.body or "")
    if not ok then error("brreg: " .. path_str .. " returned unparseable JSON") end
    return parsed, target
  end

  -- A search with no hits omits _embedded entirely rather than returning an
  -- empty list, so the absence is normal and must not read as an error.
  local function collect(body, from, key)
    local out = {}
    local embedded = body and body._embedded
    for _, e in ipairs(embedded and embedded[key] or {}) do
      out[#out + 1] = normalize(e, from)
    end
    return out
  end

  local c = {}

  function c:search(name, o)
    o = o or {}
    local body, from = api_get("/enheter?navn=" .. url.encode(trim(name))
      .. "&size=" .. tostring(o.limit or 10))
    return collect(body, from, "enheter")
  end

  function c:get(orgnr)
    local id = trim(orgnr):gsub("%s", "")
    local body, from = api_get("/enheter/" .. url.encode(id))
    if not body or not body.organisasjonsnummer then return nil end
    return normalize(body, from)
  end

  --- The reverse join outreach actually needs: prospect domain to legal entity.
  --- Brreg matches its stored website string, so the query is retried with the
  --- www-prefixed form that many entities register instead of the apex.
  function c:by_website(domain)
    local d = bare_domain(domain)
    if not d then return {} end
    for _, form in ipairs({ d, "www." .. d }) do
      local body, from = api_get("/enheter?hjemmeside=" .. url.encode(form) .. "&size=10")
      local hits = collect(body, from, "enheter")
      if #hits > 0 then return hits end
    end
    return {}
  end

  --- Sub-entities carry the site-level employee counts; the parent often
  --- reports none, so a headcount question is frequently answered here.
  function c:sub_entities(orgnr)
    local id = trim(orgnr):gsub("%s", "")
    local body, from = api_get("/underenheter?overordnetEnhet=" .. url.encode(id) .. "&size=50")
    return collect(body, from, "underenheter")
  end

  return c
end

return M
