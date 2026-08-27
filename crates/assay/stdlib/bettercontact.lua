--- @module assay.bettercontact
--- @description BetterContact waterfall enrichment — submit a batch of people, poll until the run terminates, read back verified emails and phones. Paid: every submission goes through the lead_provider budget gate and returns records with provenance.
--- @category registries
--- @icon layers
--- @keywords bettercontact, lead, prospect, waterfall, enrich, email, phone, async, batch
--- @quickref M.client(gate, opts?) -> c | Budget gate required; key via opts.api_key or BETTERCONTACT_API_KEY
--- @quickref c:submit(people, opts?) -> request_id | Start an async enrichment run
--- @quickref c:result(request_id) -> run | One poll: {status, terminated, people, credits_*}
--- @quickref c:await(request_id, opts?) -> run | Poll until terminated or the attempt budget runs out
--- @quickref c:find_person(spec, opts?) -> person|nil | Submit one person and wait for the result
--- @quickref c:resolve_email(spec, opts?) -> [email] | Emails for one person, waiting for the run

local M = {}

local lp = require("assay.lead_provider")

local DEFAULT_COST_CENTS = 0
local DEFAULT_POLL_MS = 3000
local DEFAULT_ATTEMPTS = 40

-- The run is finished only when it says `terminated`. A 202 during processing
-- carries no `data`, so branching on the HTTP code instead of this field reads
-- an in-flight run as an empty result.
local TERMINAL = { terminated = true }

local function trim(s) return (tostring(s or ""):gsub("^%s+", ""):gsub("%s+$", "")) end

-- BetterContact reports its own verification outcome per contact, mapped into
-- NEP-0007's vocabulary and deliberately stopping at PROBABLE: a vendor's
-- assertion is not a delivery, so it cannot earn VERIFIED.
--
-- `catch_all_safe` is the vendor's opinion that a catch-all domain is worth
-- sending to anyway. It stays CATCH_ALL, which never schedules — promoting it
-- would let a vendor's guess about a domain decide who gets written to. The
-- raw verdict is carried on the record so that nuance is not lost.
local STATUS_MAP = {
  deliverable = "PROBABLE",
  catch_all = "CATCH_ALL",
  catch_all_safe = "CATCH_ALL",
  catch_all_not_safe = "CATCH_ALL",
  undeliverable = "INVALID",
  not_found = "UNKNOWN",
}

local function to_person(record, from)
  record = record or {}
  local emails = {}
  local address = trim(record.contact_email_address)
  if address ~= "" then
    local vendor_status = tostring(record.contact_email_address_status or ""):lower()
    local email = lp.email("bettercontact", from, {
      address = address,
      email_type = "provider",
      verification_status = STATUS_MAP[vendor_status] or "UNKNOWN",
    })
    email.vendor_status = (vendor_status ~= "" and vendor_status or nil)
    email.provider_used = record.contact_email_address_provider or record.email_provider
    emails[1] = email
  end
  local first, last = record.contact_first_name, record.contact_last_name
  local full = trim(record.contact_full_name)
  if full == "" and (first or last) then full = trim((first or "") .. " " .. (last or "")) end
  return lp.person("bettercontact", from, {
    first_name = first,
    last_name = last,
    full_name = (full ~= "" and full or nil),
    title = record.contact_job_title,
    company = record.company_name,
    domain = record.company_domain,
    linkedin = record.contact_linkedin_profile_url,
    location = record.contact_location_country,
    emails = emails,
  })
end

function M.client(gate, opts)
  if type(gate) ~= "table" or type(gate.paid) ~= "function" then
    error("bettercontact: a lead_provider gate is required — every call here is paid")
  end
  opts = opts or {}
  local api_key = opts.api_key or env.get("BETTERCONTACT_API_KEY")
  if not api_key or trim(api_key) == "" then
    error("bettercontact: api_key required (opts.api_key or BETTERCONTACT_API_KEY)")
  end
  local base_url = (opts.base_url or "https://app.bettercontact.rocks/api/v2"):gsub("/+$", "")

  local function headers()
    return {
      ["X-API-Key"] = api_key,
      Accept = "application/json",
      ["Content-Type"] = "application/json",
    }
  end

  local function fail(where, resp)
    if resp.status == 401 or resp.status == 403 then
      error("bettercontact: " .. where .. " rejected the key (HTTP " .. resp.status .. ")")
    end
    if resp.status == 402 then
      error("bettercontact: " .. where .. " out of credits (HTTP 402)")
    end
    if resp.status == 429 then
      error("bettercontact: " .. where .. " rate limited (HTTP 429)")
    end
    error("bettercontact: " .. where .. " HTTP " .. resp.status .. ": " .. (resp.body or ""))
  end

  local function decode(resp, where)
    local ok, parsed = pcall(json.parse, resp.body or "")
    if not ok then error("bettercontact: " .. where .. " returned unparseable JSON") end
    return parsed
  end

  local c = {}

  --- Start a run. `people` entries take the API's own keys: first_name,
  --- last_name, company, company_domain, linkedin_url, custom_fields.
  function c:submit(people, o)
    o = o or {}
    if type(people) ~= "table" or #people == 0 then
      error("bettercontact: submit needs at least one person")
    end
    return gate:paid(o.operation or "resolve_email", o.cents or DEFAULT_COST_CENTS, function()
      local target = base_url .. "/async"
      local resp = http.post(target, {
        data = people,
        enrich_email_address = o.enrich_email_address ~= false,
        enrich_phone_number = o.enrich_phone_number == true,
        enrich_profile = o.enrich_profile == true,
        verify_catch_all = o.verify_catch_all == true,
        webhook = o.webhook,
      }, { headers = headers() })
      if resp.status ~= 200 and resp.status ~= 201 then fail("POST /async", resp) end
      local body = decode(resp, "POST /async")
      if not body or not body.id then
        error("bettercontact: submission returned no request id")
      end
      return body.id
    end)
  end

  --- One poll. Polling is free — only the submission is metered — so this
  --- deliberately does not go through the gate.
  function c:result(request_id)
    local target = base_url .. "/async/" .. tostring(request_id)
    local resp = http.get(target, { headers = headers() })
    if resp.status ~= 200 and resp.status ~= 202 then
      fail("GET /async/" .. tostring(request_id), resp)
    end
    local body = decode(resp, "GET /async") or {}
    local status = tostring(body.status or "")
    local people = {}
    for _, record in ipairs(body.data or {}) do
      people[#people + 1] = to_person(record, target)
    end
    return {
      id = body.id or request_id,
      status = status,
      terminated = TERMINAL[status] == true,
      people = people,
      summary = body.summary,
      credits_consumed = body.credits_consumed,
      credits_left = body.credits_left,
    }
  end

  --- Poll until the run terminates. Returns the last run seen when the attempt
  --- budget runs out, with `terminated = false` — an unfinished run is
  --- reported as unfinished rather than raised, because the id stays valid and
  --- the caller may want to come back to it.
  function c:await(request_id, o)
    o = o or {}
    local attempts = o.attempts or DEFAULT_ATTEMPTS
    local interval = (o.poll_ms or DEFAULT_POLL_MS) / 1000
    local run
    for i = 1, attempts do
      run = self:result(request_id)
      if run.terminated then return run end
      if i < attempts then sleep(interval) end
    end
    return run
  end

  -- Submit one person and wait for that run. A declined budget surfaces as
  -- `nil, "budget_declined"` rather than a raise: the caller asked whether it
  -- could afford this, and being told no is an answer, not a failure.
  local function one(spec, o, operation)
    spec, o = spec or {}, o or {}
    local id, declined = c:submit({ {
      first_name = spec.first_name,
      last_name = spec.last_name,
      company = spec.company,
      company_domain = spec.company_domain or spec.domain,
      linkedin_url = spec.linkedin_url,
      custom_fields = spec.custom_fields,
    } }, { operation = operation, cents = o.cents, verify_catch_all = o.verify_catch_all })
    if not id then return nil, declined or "budget_declined" end
    local run = c:await(id, o)
    if not run.terminated then return nil, "not_terminated", run end
    return run.people[1], nil, run
  end

  function c:find_person(spec, o)
    return one(spec, o, "find_person")
  end

  function c:resolve_email(spec, o)
    local person = one(spec, o, "resolve_email")
    return person and person.emails or {}
  end

  return c
end

return M
