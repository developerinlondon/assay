--- @module assay.contactout
--- @description ContactOut lead data — LinkedIn profile enrichment, email-to-profile reverse lookup, and flexible person enrichment. Paid: every call goes through the lead_provider budget gate and returns records with provenance.
--- @category registries
--- @icon user-search
--- @keywords contactout, lead, prospect, person, email, linkedin, enrich, contact, waterfall
--- @quickref M.client(gate, opts?) -> c | Budget gate is required; token via opts.token or CONTACTOUT_TOKEN
--- @quickref c:enrich_linkedin(url, opts?) -> person|nil | Profile by LinkedIn URL
--- @quickref c:find_person(spec, opts?) -> person|nil | By name + company/domain, or linkedin_url
--- @quickref c:resolve_email(spec, opts?) -> [email] | Work emails for a known person
--- @quickref c:profile_by_email(address, opts?) -> person|nil | Reverse lookup: email to profile

local M = {}

local lp = require("assay.lead_provider")
local url = require("assay.url")

-- ContactOut prices per call, so the caller states what a call is worth and
-- the gate decides. Defaults are placeholders the caller should override from
-- its own plan; guessing a price here would meter fiction into the ledger.
local DEFAULT_COST_CENTS = 0

local function trim(s) return (tostring(s or ""):gsub("^%s+", ""):gsub("%s+$", "")) end

local function first_of(list)
  if type(list) == "table" then return list[1] end
  if type(list) == "string" and list ~= "" then return list end
  return nil
end

local function as_list(value)
  if type(value) == "table" then return value end
  if type(value) == "string" and value ~= "" then return { value } end
  return {}
end

-- The live enrich endpoint 400s on a string `company` — it must be an array,
-- and an absent one must be omitted rather than sent empty.
local function company_param(value)
  local list = as_list(value)
  if #list == 0 then return nil end
  return list
end

-- ContactOut splits a name only sometimes, so `full_name` is authoritative and
-- the parts are best-effort. Downstream code that needs both should prefer the
-- parts it was given over anything re-derived here.
local function split_name(profile)
  local full = trim(profile.full_name)
  local first, last = profile.first_name, profile.last_name
  if (not first or first == "") and full ~= "" then
    first, last = full:match("^(%S+)%s+(.+)$")
  end
  return first, last, (full ~= "" and full or nil)
end

-- Live responses carry `company` as an object ({ name, domain, … }); older
-- recorded shapes carry a bare string. Accept both.
local function company_of(profile)
  local company = profile.company
  if type(company) == "table" then
    return company.name, profile.company_domain or company.domain or company.email_domain
  end
  return company, profile.company_domain
end

local function to_person(profile, from)
  profile = profile or {}
  local first, last, full = split_name(profile)
  local company_name, company_domain = company_of(profile)
  local emails = {}
  for _, address in ipairs(as_list(profile.work_email)) do
    emails[#emails + 1] = lp.email("contactout", from,
      { address = address, email_type = "provider" })
  end
  for _, address in ipairs(as_list(profile.email)) do
    local seen = false
    for _, existing in ipairs(emails) do
      if existing.address == address then seen = true end
    end
    if not seen then
      emails[#emails + 1] = lp.email("contactout", from,
        { address = address, email_type = "provider" })
    end
  end
  return lp.person("contactout", from, {
    first_name = first,
    last_name = last,
    full_name = full,
    title = profile.headline or profile.job_title,
    company = company_name,
    domain = company_domain,
    linkedin = profile.url or profile.linkedin,
    location = profile.location,
    emails = emails,
  })
end

--- Build a client. The budget gate is the first argument and is not optional:
--- every operation here costs money, and a client that could be built without
--- one would make an unmetered call reachable by omission.
function M.client(gate, opts)
  if type(gate) ~= "table" or type(gate.paid) ~= "function" then
    error("contactout: a lead_provider gate is required — every call here is paid")
  end
  opts = opts or {}
  local token = opts.token or env.get("CONTACTOUT_TOKEN")
  if not token or trim(token) == "" then
    error("contactout: token required (opts.token or CONTACTOUT_TOKEN)")
  end
  local base_url = (opts.base_url or "https://api.contactout.com"):gsub("/+$", "")

  -- The header is the bare name `token`, not Authorization and not a Bearer
  -- prefix. Getting this wrong reads as an invalid key rather than a malformed
  -- request, which is a slow thing to debug.
  local function headers()
    return { token = token, Accept = "application/json", ["Content-Type"] = "application/json" }
  end

  local function fail(where, resp)
    if resp.status == 401 or resp.status == 403 then
      error("contactout: " .. where .. " rejected the token (HTTP " .. resp.status .. ")")
    end
    if resp.status == 429 then
      error("contactout: " .. where .. " rate limited (HTTP 429)")
    end
    error("contactout: " .. where .. " HTTP " .. resp.status .. ": " .. (resp.body or ""))
  end

  local function api(method, path_str, payload)
    local target = base_url .. path_str
    local resp
    if method == "POST" then
      resp = http.post(target, payload or {}, { headers = headers() })
    else
      resp = http.get(target, { headers = headers() })
    end
    if resp.status == 404 then return nil, target end
    if resp.status ~= 200 and resp.status ~= 201 then fail(method .. " " .. path_str, resp) end
    local ok, parsed = pcall(json.parse, resp.body or "")
    if not ok then error("contactout: " .. path_str .. " returned unparseable JSON") end
    return parsed, target
  end

  local c = {}

  function c:enrich_linkedin(profile_url, o)
    o = o or {}
    return gate:paid("find_person", o.cents or DEFAULT_COST_CENTS, function()
      local body, from = api("GET", "/v1/linkedin/enrich?profile=" .. url.encode(trim(profile_url)))
      if not body or not body.profile then return nil end
      return to_person(body.profile, from)
    end)
  end

  function c:profile_by_email(address, o)
    o = o or {}
    return gate:paid("find_person", o.cents or DEFAULT_COST_CENTS, function()
      local body, from = api("GET", "/v1/people/person?email=" .. url.encode(trim(address)))
      if not body or not body.profile then return nil end
      return to_person(body.profile, from)
    end)
  end

  -- `spec` mirrors the API's own accepted keys rather than inventing a schema,
  -- so a caller reading ContactOut's docs can pass what it says.
  function c:find_person(spec, o)
    spec, o = spec or {}, o or {}
    if spec.linkedin_url and not spec.full_name and not spec.last_name then
      return self:enrich_linkedin(spec.linkedin_url, o)
    end
    return gate:paid("find_person", o.cents or DEFAULT_COST_CENTS, function()
      local body, from = api("POST", "/v1/people/enrich", {
        full_name = spec.full_name,
        first_name = spec.first_name,
        last_name = spec.last_name,
        company = company_param(spec.company),
        location = spec.location,
        linkedin_url = spec.linkedin_url,
        email = spec.email,
        include = spec.include or { "work_email" },
      })
      if not body or not body.profile then return nil end
      return to_person(body.profile, from)
    end)
  end

  --- The emails a person lookup already carried. Reported as `resolve_email`
  --- so spend lands against the operation the caller actually wanted.
  function c:resolve_email(spec, o)
    spec, o = spec or {}, o or {}
    local person = gate:paid("resolve_email", o.cents or DEFAULT_COST_CENTS, function()
      local body, from
      if spec.linkedin_url then
        body, from = api("GET", "/v1/linkedin/enrich?profile=" .. url.encode(trim(spec.linkedin_url)))
      else
        body, from = api("POST", "/v1/people/enrich", {
          full_name = spec.full_name,
          first_name = spec.first_name,
          last_name = spec.last_name,
          company = company_param(spec.company),
          include = { "work_email" },
        })
      end
      if not body or not body.profile then return nil end
      return to_person(body.profile, from)
    end)
    return person and person.emails or {}
  end

  return c
end

M.first_of = first_of

return M
