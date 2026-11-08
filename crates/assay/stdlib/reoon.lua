--- @module assay.reoon
--- @description Reoon email verification — a paid verify_email rung. One GET per address through the lead_provider budget gate, plus a free account-balance read. The API key travels in the query string, so it is scrubbed from every error and kept out of provenance.
--- @category registries
--- @icon mail-check
--- @keywords reoon, email, verify, verification, deliverable, catch-all, balance, credits
--- @quickref M.client(gate, opts?) -> c | Budget gate is required; key via opts.api_key or REOON_KEY
--- @quickref c:verify_email(address, opts?) -> record | lead_provider email record + vendor detail
--- @quickref c:balance() -> {api_status, daily, instant} | Free; spends no credit

local M = {}

local lp = require("assay.lead_provider")
local url = require("assay.url")

local DEFAULT_COST_CENTS = 0

-- Power mode opens an SMTP conversation with the recipient's server and the
-- vendor documents it as taking up to a minute.
local DEFAULT_TIMEOUT_S = 75

local function trim(s) return (tostring(s or ""):gsub("^%s+", ""):gsub("%s+$", "")) end

-- "safe" is the vendor's opinion, so it lands as PROBABLE and never VERIFIED.
local STATUS_MAP = {
  safe = "PROBABLE",
  valid = "PROBABLE",
  catch_all = "CATCH_ALL",
  invalid = "INVALID",
  disabled = "INVALID",
  spamtrap = "INVALID",
  disposable = "INVALID",
  role_account = "UNKNOWN",
  inbox_full = "UNKNOWN",
  unknown = "UNKNOWN",
}

--- What Reoon's status means in the lead_provider vocabulary.
function M.verdict(status, is_catch_all)
  if is_catch_all == true then return "CATCH_ALL" end
  return STATUS_MAP[tostring(status or ""):lower()] or "UNKNOWN"
end

-- The key is a query parameter, so a transport error naming the URL would
-- carry it. Every message leaving this module goes through here.
local function scrub(text, key)
  text = tostring(text or "")
  if key and key ~= "" then
    local plain = key:gsub("%p", "%%%0")
    local encoded = url.encode(key):gsub("%p", "%%%0")
    text = text:gsub(plain, "<redacted>"):gsub(encoded, "<redacted>")
  end
  return (text:gsub("([?&]key=)[^&%s%)\"']*", "%1<redacted>"))
end

-- The vendor names its own refusal in `reason`; a 402 or a spent quota says why
-- here, and that sentence is what an operator needs, not the bare status.
local function reason_of(body)
  local ok, parsed = pcall(json.parse, body or "")
  if ok and type(parsed) == "table" and type(parsed.reason) == "string" then
    return parsed.reason
  end
  return trim(body):sub(1, 200)
end

function M.client(gate, opts)
  if type(gate) ~= "table" or type(gate.paid) ~= "function" then
    error("reoon: a lead_provider gate is required — every verification here is paid")
  end
  opts = opts or {}
  local api_key = opts.api_key or env.get("REOON_KEY")
  if not api_key or trim(api_key) == "" then
    error("reoon: api key required (opts.api_key or REOON_KEY)")
  end
  api_key = trim(api_key)
  local base_url = (opts.base_url or "https://emailverifier.reoon.com"):gsub("/+$", "")
  local mode = opts.mode or "power"
  local http_client = http.client({ timeout = opts.timeout or DEFAULT_TIMEOUT_S })

  local function get(what, path)
    local target = base_url .. path .. "key=" .. url.encode(api_key)
    local ok, resp = pcall(function()
      return http_client:get(target, { headers = { Accept = "application/json" } })
    end)
    if not ok then error("reoon: " .. what .. " failed: " .. scrub(resp, api_key), 0) end
    if resp.status == 401 or resp.status == 403 then
      error("reoon: " .. what .. " rejected the key (HTTP " .. resp.status .. ")", 0)
    end
    if resp.status == 429 then
      error("reoon: " .. what .. " rate limited (HTTP 429)", 0)
    end
    if resp.status ~= 200 then
      error("reoon: " .. what .. " HTTP " .. resp.status .. ": "
        .. scrub(reason_of(resp.body), api_key), 0)
    end
    local parsed_ok, parsed = pcall(json.parse, resp.body or "")
    if not parsed_ok or type(parsed) ~= "table" then
      error("reoon: " .. what .. " returned unparseable JSON", 0)
    end
    if parsed.status == "error" then
      error("reoon: " .. what .. " error: " .. scrub(parsed.reason or "unspecified", api_key), 0)
    end
    return parsed
  end

  local c = {}

  function c:verify_email(address, o)
    o = o or {}
    address = trim(address)
    return gate:paid("verify_email", o.cents or DEFAULT_COST_CENTS, function()
      local query = "/api/v1/verify?email=" .. url.encode(address)
        .. "&mode=" .. url.encode(o.mode or mode) .. "&"
      local d = get("verify", query)
      local vendor_result = tostring(d.status or ""):lower()
      local record = lp.email("reoon", base_url .. "/api/v1/verify", {
        address = (type(d.email) == "string" and d.email ~= "" and d.email) or address,
        email_type = "provider",
        verification_status = M.verdict(vendor_result, d.is_catch_all),
        confidence = d.overall_score,
      })
      record.vendor_result = (vendor_result ~= "" and vendor_result or nil)
      record.is_catch_all = d.is_catch_all
      record.is_deliverable = d.is_deliverable
      record.is_safe_to_send = d.is_safe_to_send
      record.is_disposable = d.is_disposable
      record.is_role_account = d.is_role_account
      record.mx_accepts_mail = d.mx_accepts_mail
      record.can_connect_smtp = d.can_connect_smtp
      return record
    end)
  end

  --- Free: reads the account's remaining credits without spending one.
  function c:balance()
    local d = get("balance", "/api/v1/check-account-balance/?")
    return {
      api_status = d.api_status,
      daily = d.remaining_daily_credits,
      instant = d.remaining_instant_credits,
    }
  end

  return c
end

return M
