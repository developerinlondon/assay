--- @module assay.mails_so
--- @description mails.so email verification — the paid verify_email rung of NEP-0007 §10. One GET per address; every call goes through the lead_provider budget gate and returns a record with provenance.
--- @category registries
--- @icon mail-check
--- @keywords mails.so, email, verify, verification, deliverable, catch-all, waterfall
--- @quickref M.client(gate, opts?) -> c | Budget gate is required; key via opts.api_key or MAILS_SO_KEY
--- @quickref c:verify_email(address, opts?) -> record | lead_provider email record + vendor detail

local M = {}

local lp = require("assay.lead_provider")
local url = require("assay.url")

-- mails.so prices per validation; the caller states what a call is worth and
-- the gate decides. A guessed price here would meter fiction into the ledger.
local DEFAULT_COST_CENTS = 0

local function trim(s) return (tostring(s or ""):gsub("^%s+", ""):gsub("%s+$", "")) end

-- A vendor's "deliverable" is an assertion, not a delivery — it lands as
-- PROBABLE, never VERIFIED (§2: only our own evidence verifies). The raw
-- verdict is carried on the record so the nuance is not lost.
local STATUS_MAP = {
  deliverable = "PROBABLE",
  undeliverable = "INVALID",
  risky = "UNKNOWN",
  unknown = "UNKNOWN",
}

--- Build a client. The gate comes first and is not optional: every call here
--- is paid, and a client constructible without one would make an unmetered
--- call reachable by omission.
function M.client(gate, opts)
  if type(gate) ~= "table" or type(gate.paid) ~= "function" then
    error("mails_so: a lead_provider gate is required — every call here is paid")
  end
  opts = opts or {}
  local api_key = opts.api_key or env.get("MAILS_SO_KEY")
  if not api_key or trim(api_key) == "" then
    error("mails_so: api key required (opts.api_key or MAILS_SO_KEY)")
  end
  local base_url = (opts.base_url or "https://api.mails.so"):gsub("/+$", "")

  local c = {}

  function c:verify_email(address, o)
    o = o or {}
    address = trim(address)
    return gate:paid("verify_email", o.cents or DEFAULT_COST_CENTS, function()
      local target = base_url .. "/v1/validate?email=" .. url.encode(address)
      local resp = http.get(target, {
        headers = { ["x-mails-api-key"] = api_key, Accept = "application/json" },
      })
      if resp.status == 401 or resp.status == 403 then
        error("mails_so: validate rejected the key (HTTP " .. resp.status .. ")")
      end
      if resp.status == 429 then
        error("mails_so: validate rate limited (HTTP 429)")
      end
      if resp.status ~= 200 then
        error("mails_so: validate HTTP " .. resp.status .. ": " .. (resp.body or ""))
      end
      local ok, parsed = pcall(json.parse, resp.body or "")
      if not ok then error("mails_so: validate returned unparseable JSON") end
      if parsed.error ~= nil and parsed.error ~= json.null then
        error("mails_so: validate error: " .. tostring(parsed.error))
      end
      local d = parsed.data or {}

      local vendor_result = tostring(d.result or ""):lower()
      local status = STATUS_MAP[vendor_result] or "UNKNOWN"
      -- isv_nocatchall == false means the domain accepts anything; that is a
      -- CATCH_ALL whatever the vendor concluded, and CATCH_ALL never schedules.
      if d.isv_nocatchall == false then status = "CATCH_ALL" end

      local record = lp.email("mails_so", target, {
        address = (d.email and d.email ~= "" and d.email) or address,
        email_type = "provider",
        verification_status = status,
        confidence = d.score,
      })
      record.vendor_result = (vendor_result ~= "" and vendor_result or nil)
      record.reason = d.reason
      record.mx_record = d.mx_record
      record.email_provider = d.provider
      record.is_disposable = d.is_disposable
      record.is_free = d.is_free
      record.did_you_mean = d.did_you_mean
      return record
    end)
  end

  return c
end

return M
