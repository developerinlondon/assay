--- @module assay.lead_provider
--- @description The contract every lead-data adapter implements — uniform person/email shapes with provenance, and the budget gate that must wrap every paid call. Free registry modules present the same shape, so evidence provenance is uniform across free and paid sources.
--- @category registries
--- @icon shield-check
--- @keywords lead, provider, contract, budget, gate, spend, provenance, person, email, prospect
--- @quickref M.gate(budget) -> gate | Budget context; raises unless approve+meter are given
--- @quickref gate:paid(op, cents, fn) -> result | Approve, run, meter — the only path to a paid call
--- @quickref M.provenance(provider, from) -> table | Uniform {provider, retrieved_from, retrieved_at}
--- @quickref M.person(provider, from, fields) -> person | Normalized person record
--- @quickref M.email(provider, from, fields) -> email | Normalized email record
--- @quickref M.OPERATIONS -> [string] | The four operations an adapter may expose

local M = {}

-- The operation names are part of the contract, not decoration: the budget
-- ledger attributes spend per operation, so an adapter inventing its own name
-- would make cost-per-qualified-lead unqueryable.
M.OPERATIONS = { "search_companies", "find_person", "resolve_email", "verify_email" }

local OPERATION_SET = {}
for _, op in ipairs(M.OPERATIONS) do OPERATION_SET[op] = true end

local function trim(s) return (tostring(s or ""):gsub("^%s+", ""):gsub("%s+$", "")) end

local function stamp()
  return os.date("!%Y-%m-%dT%H:%M:%SZ")
end

--- Wrap a caller-supplied budget context.
---
--- The ledger this consults lives in the caller's database, not in assay, so
--- the context is injected rather than discovered. Refusing to build a gate
--- without one is the whole point: a paid lookup must never be reachable by
--- forgetting an argument.
---
--- `approve(op, cents)` returns `true`, or `false, reason` when the spend is
--- over the line and an approval has been filed. `meter(op, cents, meta)`
--- records what was actually spent.
function M.gate(budget)
  if type(budget) ~= "table" then
    error("lead_provider: a budget context is required — paid lookups are gated (NEP-0007 §8)")
  end
  if type(budget.approve) ~= "function" or type(budget.meter) ~= "function" then
    error("lead_provider: budget context needs approve(op, cents) and meter(op, cents, meta)")
  end

  local gate = {}

  --- Run `fn` only if the budget approves, then meter what it cost.
  ---
  --- Metering happens after a successful call and not after a failed one: the
  --- ledger answers "what did this cost", and a call that raised bought
  --- nothing. Providers that charge for failures are the caller's problem to
  --- reconcile, and saying so here beats silently inflating the ledger.
  function gate:paid(op, cents, fn)
    if not OPERATION_SET[op] then
      error("lead_provider: unknown operation '" .. tostring(op) .. "'")
    end
    if type(cents) ~= "number" or cents < 0 then
      error("lead_provider: " .. op .. " needs a non-negative cost in cents")
    end
    local ok, reason = budget.approve(op, cents)
    if not ok then
      return nil, reason or "budget_declined"
    end
    local result = fn()
    budget.meter(op, cents, { operation = op, cents = cents, at = stamp() })
    return result
  end

  return gate
end

function M.provenance(provider, from)
  return { provider = trim(provider), retrieved_from = from, retrieved_at = stamp() }
end

-- The flat shapes below are what downstream code sees, so no caller ever
-- learns a vendor's envelope. Unknown fields stay nil rather than empty
-- string: absent evidence and blank evidence are different claims.
function M.person(provider, from, fields)
  fields = fields or {}
  return {
    first_name = fields.first_name,
    last_name = fields.last_name,
    full_name = fields.full_name,
    title = fields.title,
    company = fields.company,
    domain = fields.domain,
    linkedin = fields.linkedin,
    location = fields.location,
    emails = fields.emails or {},
    provenance = M.provenance(provider, from),
  }
end

--- `email_type` and `verification_status` are NEP-0007 §2's vocabulary. An
--- adapter reports what the provider claims; it never promotes a claim to
--- VERIFIED, which only a delivery can earn.
function M.email(provider, from, fields)
  fields = fields or {}
  return {
    address = fields.address,
    email_type = fields.email_type or "provider",
    verification_status = fields.verification_status or "UNKNOWN",
    confidence = fields.confidence,
    provenance = M.provenance(provider, from),
  }
end

return M
