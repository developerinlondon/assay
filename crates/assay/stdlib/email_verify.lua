--- @module assay.email_verify
--- @description Free email verification rung — syntax, MX via DNS-over-HTTPS, and pattern candidates. Keyless and deterministic; SMTP-level verdicts (CATCH_ALL, deliverable) belong to the smtp_probe builtin or a paid verifier.
--- @category registries
--- @icon mail
--- @keywords email, verify, mx, dns, doh, pattern, candidates, deliverability
--- @quickref M.check_syntax(email) -> ok, reason | RFC-shaped address test
--- @quickref M.candidates(first, last, domain) -> [email] | Pattern guesses, most common first
--- @quickref M.is_disposable(domain) -> bool | Throwaway-provider domain
--- @quickref M.is_role(email) -> bool | Shared mailbox local-part (info@, sales@)
--- @quickref M.suggest(email) -> email|nil | Typo correction for common providers
--- @quickref M.client(opts?) -> c | DoH client; doh_url override for tests
--- @quickref c:mx(domain) -> [host] | MX hosts (A/AAAA fallback marked as weak)
--- @quickref c:verify(email) -> verdict | {email, status, method, mx, checked_at}
--- @quickref c:probe(email, opts) -> verdict | Adds SMTP evidence; opts.from is required

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

-- A throwaway-provider shortlist, not the whole 4000-domain census: the long
-- tail is churn, and a wrong entry here would libel a real prospect's domain.
-- Membership only ever raises a flag — never a status — so the cost of being
-- out of date is a missing hint rather than a suppressed address.
local DISPOSABLE_LIST = [[
10mail.com 10mail.org 10minutemail.com 10minutemail.net 1secmail.com 1secmail.net
1secmail.org 4warding.com 4warding.net 4warding.org 675hosting.com 675hosting.net
675hosting.org 75hosting.com 75hosting.net 75hosting.org borged.com borged.net
borged.org burnermail.io chong-mail.com chong-mail.net chong-mail.org cmail.com
cmail.net cmail.org dispostable.com email-fake.com emailondeck.com etranquil.com
etranquil.net etranquil.org fakeinbox.com fakeinbox.info getairmail.com getnada.com
ginzi.net gotmail.com gotmail.net gotmail.org grr.la guerillamail.com
guerillamail.info guerillamail.net guerillamail.org guerrillamail.com
guerrillamail.info guerrillamail.net guerrillamail.org gynzy.info inboxkitten.com
incognitomail.com incognitomail.net incognitomail.org inoutmail.info inoutmail.net
intopwa.com intopwa.net intopwa.org jetable.com jetable.net jetable.org
maildrop.cc mailinator.com mailinator.info mailinator.net mailinator.org mailna.me
mailnesia.com midlertidig.com midlertidig.net midlertidig.org moakt.com mohmal.com
muell.io myspaceinc.com myspaceinc.net myspaceinc.org mytemp.email nowmymail.com
pecinan.com pecinan.net pecinan.org pratikmail.com pratikmail.net pratikmail.org
sharklasers.com shitmail.me shitmail.org smapfree24.com smapfree24.info
smapfree24.org spam4.me spambob.com spambob.net spambob.org spambog.com spambog.net
spambox.info spambox.me spambox.org spamcowboy.com spamcowboy.net spamcowboy.org
spamfree24.com spamfree24.info spamfree24.net spamfree24.org spamgourmet.com
spamgourmet.net spamgourmet.org stop-my-spam.com temp-mail.com temp-mail.org
tempemail.com tempemail.net tempmail.com tempmailer.com tempmailer.net tempr.email
throwawaymail.com tmail.io trash-mail.com trashmail.com trashmail.io trashmail.me
trashmail.net trashmail.org veryday.info viewcastmedia.com viewcastmedia.net
viewcastmedia.org wegwerf-email.net wegwerfemail.com wegwerfemail.info
wegwerfemail.net wegwerfemail.org wegwerfmail.info wegwerfmail.net wegwerfmail.org
wegwrfmail.net wegwrfmail.org yopmail.com yopmail.net zoemail.com zoemail.net
zoemail.org
]]

local ROLE_LIST = [[
abuse accounts admin administrator billing careers compliance contact contacts
customerservice enquiries enquiry finance help helpdesk hello hr info inquiries
invoices it jobs legal mail marketing news newsletter noc noreply no-reply office
operations orders postmaster press privacy purchasing recruitment sales security
service støtte support sysadmin team webmaster
]]

-- Providers worth a "did you mean": each has enough share that a one-character
-- slip is far likelier than a real domain sitting one edit away from it.
local POPULAR_LIST = [[
aol.com btinternet.com comcast.net gmail.com gmx.com gmx.de googlemail.com
hotmail.co.uk hotmail.com icloud.com live.com mail.com me.com msn.com outlook.com
proton.me protonmail.com sky.com verizon.net virginmedia.com yahoo.co.uk yahoo.com
yandex.com zoho.com
]]

local function to_set(blob)
  local set = {}
  for word in blob:gmatch("%S+") do set[word] = true end
  return set
end

local function to_list(blob)
  local out = {}
  for word in blob:gmatch("%S+") do out[#out + 1] = word end
  return out
end

M.DISPOSABLE = to_set(DISPOSABLE_LIST)
M.ROLE = to_set(ROLE_LIST)
M.POPULAR = to_list(POPULAR_LIST)

function M.is_disposable(domain)
  return M.DISPOSABLE[trim(domain):lower():gsub("^www%.", "")] == true
end

-- Role addresses reach a desk, not a person. They are perfectly deliverable,
-- which is exactly why the flag is worth carrying separately from the status.
function M.is_role(email)
  local localpart = trim(email):lower():match("^([^@]+)@")
  if not localpart then return false end
  return M.ROLE[(localpart:gsub("[%.%-_]", ""))] == true or M.ROLE[localpart] == true
end

-- Optimal string alignment rather than plain Levenshtein: the typos worth
-- catching are overwhelmingly adjacent transpositions (gmial, hotmial), which
-- Levenshtein scores as two edits and would therefore miss entirely.
local function within_one_edit(a, b)
  if a == b then return false end
  if math.abs(#a - #b) > 1 then return false end
  local prev2, prev = nil, {}
  for j = 0, #b do prev[j] = j end
  for i = 1, #a do
    local cur = { [0] = i }
    local best = i
    for j = 1, #b do
      local cost = (a:sub(i, i) == b:sub(j, j)) and 0 or 1
      local v = math.min(prev[j] + 1, cur[j - 1] + 1, prev[j - 1] + cost)
      if i > 1 and j > 1 and a:sub(i, i) == b:sub(j - 1, j - 1)
        and a:sub(i - 1, i - 1) == b:sub(j, j) then
        v = math.min(v, prev2[j - 2] + 1)
      end
      cur[j] = v
      if v < best then best = v end
    end
    if best > 1 then return false end
    prev2, prev = prev, cur
  end
  return prev[#b] == 1
end

function M.suggest(email)
  local localpart, domain = trim(email):lower():match("^([^@]+)@([^@]+)$")
  if not localpart then return nil end
  if M.DISPOSABLE[domain] then return nil end
  for _, known in ipairs(M.POPULAR) do
    if domain == known then return nil end
  end
  for _, known in ipairs(M.POPULAR) do
    if within_one_edit(domain, known) then return localpart .. "@" .. known end
  end
  return nil
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

  -- What the envelope dialogue proved, in NEP-0007's vocabulary. An accepted
  -- RCPT is the strongest evidence available without sending, and it still
  -- stops at PROBABLE: servers accept at RCPT and bounce later. VERIFIED is
  -- reserved for a delivery that actually happened.
  local SMTP_VERDICTS = {
    catch_all = "CATCH_ALL",
    no_mailbox = "INVALID",
    not_allowed = "INVALID",
    recipient_moved = "INVALID",
  }

  --- Full waterfall: syntax, MX, then a live RCPT probe.
  --- `opts.from` is required and must be an address on a domain you control —
  --- receiving servers judge the envelope sender, and a bogus one earns the
  --- probing IP a reputation hit that outlives the lookup.
  function c:probe(email, opts)
    opts = opts or {}
    if not opts.from then error("email_verify: probe requires opts.from (envelope sender)") end

    local verdict = self:verify(email)
    local domain = email:match("@(.+)$")
    verdict.disposable = domain ~= nil and M.is_disposable(domain) or false
    verdict.role = M.is_role(email)
    verdict.suggestion = M.suggest(email)
    if verdict.status == "INVALID" then return verdict end

    local hosts = {}
    for _, entry in ipairs(verdict.mx) do hosts[#hosts + 1] = entry.host end
    local ok, r = pcall(smtp_probe.check, {
      email = email, mx = hosts, from = opts.from, helo = opts.helo,
      port = opts.port, catch_all = opts.catch_all,
      connect_timeout_ms = opts.connect_timeout_ms,
      op_timeout_ms = opts.op_timeout_ms,
      greylist_delay_ms = opts.greylist_delay_ms,
    })
    if not ok then
      verdict.method = "smtp:error"
      verdict.detail = tostring(r)
      return verdict
    end

    verdict.status = r.deliverable and "PROBABLE" or (SMTP_VERDICTS[r.reason] or "UNKNOWN")
    verdict.method = "smtp:" .. r.reason
    verdict.smtp = r
    verdict.mx_host = r.mx_host
    verdict.full_inbox = r.full_inbox
    verdict.greylisted = r.greylisted
    verdict.detail = r.detail
    return verdict
  end

  return c
end

return M
