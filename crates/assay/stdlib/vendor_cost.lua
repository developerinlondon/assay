--- @module assay.vendor_cost
--- @description The cost contract every vendor module answers with — money as whole cents, and one item shape whose absent keys mean the vendor never stated the fact rather than that it is zero.
--- @category saas
--- @icon receipt
--- @keywords cost, price, billing, cents, currency, plan, invoice, vendor, spend
--- @quickref M.to_cents(raw) -> integer | nil | A decimal string or a number to whole cents; hex, exponent and negative are not prices
--- @quickref M.item(fields) -> item | Builds one cost line, dropping every fact the vendor did not state
--- @quickref item -> {kind, unit, ref, quantity, unit_price_cents, period, source} | unit is the unit of measure, ref names the instance; an absent price or period is a fact withheld, never a zero

local M = {}

--- Money as whole cents.
---
--- `tonumber` reads `"0x10"` as 16 and `"1e2"` as 100. Neither is a price any
--- vendor writes, and letting one through prices a mailbox at sixteen hundred
--- cents. A price is digits with at most one decimal point; anything else is
--- not a price, and returns nothing rather than a wrong number.
---
--- The conversion happens once, here, because a fleet priced in floats
--- accumulates a rounding error across every row.
function M.to_cents(raw)
  local n
  if type(raw) == "number" then
    n = raw
  elseif type(raw) == "string" and raw:match("^%d+%.?%d*$") then
    n = tonumber(raw)
  else
    return nil
  end
  if type(n) ~= "number" or n ~= n or n < 0 or n == math.huge then return nil end
  return math.floor(n * 100 + 0.5)
end

--- One cost line.
---
--- `unit` is the unit of measure — "mailbox", "domain", "plan" — and `ref`
--- names the instance it applies to, which a line covering a group of them
--- does not have. A price or a period the vendor never stated is left off the
--- table entirely: a key present and zero is a claim the vendor did not make.
---
--- Every field is copied by name, so a caller's stray key cannot widen the
--- shape the three vendor modules agree on.
function M.item(fields)
  return {
    kind = fields.kind,
    unit = fields.unit,
    ref = fields.ref,
    quantity = fields.quantity,
    unit_price_cents = fields.unit_price_cents,
    period = fields.period,
    source = fields.source or "vendor",
  }
end

return M
