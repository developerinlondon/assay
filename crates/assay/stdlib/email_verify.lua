--- @module assay.email_verify
--- @description Free email verification rung — syntax, MX via DNS-over-HTTPS, and pattern candidates. Keyless and deterministic; SMTP-level verdicts (CATCH_ALL, deliverable) belong to the smtp_probe builtin or a paid verifier.
--- @category registries
--- @icon mail
--- @keywords email, verify, mx, dns, doh, pattern, candidates, deliverability
--- @quickref M.check_syntax(email) -> ok, reason | RFC-shaped address test
--- @quickref M.candidates(first, last, domain) -> [email] | Pattern guesses, most common first
--- @quickref M.client(opts?) -> c | DoH client; doh_url override for tests
--- @quickref c:mx(domain) -> [host] | MX hosts (A/AAAA fallback marked as weak)
--- @quickref c:verify(email) -> verdict | {email, status, method, mx, checked_at}

local M = {}

local function trim(s) return (tostring(s or ""):gsub("^%s+", ""):gsub("%s+$", "")) end

-- Deliberately stricter than RFC 5322's outer bounds: the grammar here is the
-- shape real business addresses take, and anything outside it is a guess we
-- would never send to anyway.
function M.check_syntax(email)
  email = trim(email)
  if email == "" then return false, "empty" end
  if #email > 254 then return false, "too_long" end
  local localpart, domain = email:match("^([^@]+)@([^@]+)$")
  if not localpart then return false, "not_one_at_sign" end
  if #localpart > 64 then return false, "local_too_long" end
  if not localpart:match("^[%w!#%$%%&'%*%+%-/=%?%^_`{|}~%.]+$") or localpart:match("%.%.")
    or localpart:match("^%.") or localpart:match("%.$") then
    return false, "local_chars"
  end
  if not domain:match("^[%w%-%.]+%.%a%a+$") or domain:match("%.%.") or domain:match("^%-")
    or domain:match("%-%.") or domain:match("%.%-") then
    return false, "domain_shape"
  end
  return true
end

-- Most-common-first, per the executive-email conventions the operator's own
-- workbooks record. The caller owns dedup against addresses already known.
function M.candidates(first, last, domain)
  first = trim(first):lower():gsub("[^%a]", "")
  last = trim(last):lower():gsub("[^%a]", "")
  domain = trim(domain):lower():gsub("^www%.", "")
  if first == "" or last == "" or domain == "" then return {} end
  local f, l = first:sub(1, 1), last:sub(1, 1)
  local shapes = {
    first .. "." .. last, first .. last, first, f .. last,
    first .. "_" .. last, f .. "." .. last, first .. "." .. l, last,
  }
  local out, seen = {}, {}
  for _, shape in ipairs(shapes) do
    local addr = shape .. "@" .. domain
    if not seen[addr] then
      seen[addr] = true
      out[#out + 1] = addr
    end
  end
  return out
end

function M.client(opts)
  opts = opts or {}
  local doh_url = (opts.doh_url or "https://cloudflare-dns.com/dns-query"):gsub("/+$", "")

  local function resolve(name, rtype)
    local resp = http.get(doh_url .. "?name=" .. name .. "&type=" .. rtype,
      { headers = { Accept = "application/dns-json" } })
    if resp.status ~= 200 then
      error("email_verify: DoH " .. rtype .. " " .. name .. " HTTP " .. resp.status)
    end
    local body = json.parse(resp.body)
    return body and body.Answer or {}
  end

  local c = {}

  -- MX hosts, lowest preference first. When the zone has no MX, RFC 5321
  -- falls back to the address records — reported with weak = true so the
  -- verdict can stay honest about how thin that evidence is.
  function c:mx(domain)
    domain = trim(domain):lower()
    local out = {}
    for _, ans in ipairs(resolve(domain, "MX")) do
      local pref, host = tostring(ans.data or ""):match("^(%d+)%s+(%S+)$")
      if host then out[#out + 1] = { host = host:gsub("%.$", ""), preference = tonumber(pref) } end
    end
    if #out > 0 then
      table.sort(out, function(x, y) return (x.preference or 0) < (y.preference or 0) end)
      return out, false
    end
    for _, rtype in ipairs({ "A", "AAAA" }) do
      for _, ans in ipairs(resolve(domain, rtype)) do
        if ans.data then return { { host = domain, preference = 0 } }, true end
      end
    end
    return {}, false
  end

  -- The free rung's whole vocabulary: INVALID when the address cannot work,
  -- UNKNOWN when the domain accepts mail but nothing proves this mailbox —
  -- never PROBABLE or VERIFIED, which only the SMTP probe or a paid verifier
  -- may say (NEP-0007 §2 keeps those gates honest).
  function c:verify(email)
    email = trim(email)
    local ok, reason = M.check_syntax(email)
    local stamp = os.date("!%Y-%m-%dT%H:%M:%SZ")
    if not ok then
      return { email = email, status = "INVALID", method = "syntax:" .. reason,
        mx = {}, checked_at = stamp }
    end
    local domain = email:match("@(.+)$")
    local hosts, weak = self:mx(domain)
    if #hosts == 0 then
      return { email = email, status = "INVALID", method = "dns:no_mx_no_a",
        mx = {}, checked_at = stamp }
    end
    return { email = email, status = "UNKNOWN",
      method = weak and "dns:a_fallback" or "dns:mx", mx = hosts, checked_at = stamp }
  end

  return c
end

return M
