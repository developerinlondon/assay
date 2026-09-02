-- Domain health for cold-email sending domains: can mail leave, and will it
-- arrive? MX says a domain can receive; SPF, DKIM and DMARC say receivers will
-- believe what it sends; the blacklists say whether anyone has taken against it.
--
--   assay run examples/domain-health.lua -- officeofnayeem.com askmarkanastasi.com
--
-- Selectors and lists come from the environment, since which selector a
-- provider publishes under is a property of the provider, not of the domain:
--   DKIM_SELECTORS=google,default,selector1  (comma-separated, in order)
--   DNSBLS=fresh.spameatingmonkey.net        (comma-separated)
--
-- Raises if any domain fails a check, so it can gate a send. A blacklist hit
-- is one of those failures, so a healthy-looking report that ends in a listing
-- still exits non-zero. That is the point, not a broken run.

local function split_list(value, fallback)
  local out = {}
  for item in (value or fallback):gmatch("[^,%s]+") do
    out[#out + 1] = item
  end
  return out
end

local DKIM_SELECTORS = split_list(env.get("DKIM_SELECTORS"), "google,default,selector1,k1,mail")
local DNSBLS = split_list(env.get("DNSBLS"), "fresh.spameatingmonkey.net")

-- A lookup that answers "the resolver broke" as such. Callers must not read
-- that as "the domain has no record", which is the opposite conclusion.
local function lookup(name, rtype)
  local ok, result = pcall(dns.lookup, name, rtype)
  if not ok then
    return nil, tostring(result)
  end
  return result
end

local function first_txt_matching(name, prefix)
  local records, err = lookup(name, "TXT")
  if not records then
    return nil, err
  end
  for _, record in ipairs(records) do
    if record:sub(1, #prefix):lower() == prefix:lower() then
      return record
    end
  end
  return nil
end

local function check_mx(domain, report)
  local records, err = lookup(domain, "MX")
  if not records then
    report.dns_ok = false
    report.mx = "resolver error: " .. err
    return
  end
  if #records == 0 then
    report.ok = false
    report.mx = "missing"
    return
  end
  local hosts = {}
  for _, record in ipairs(records) do
    hosts[#hosts + 1] = record.preference .. " " .. record.exchange
  end
  report.mx = table.concat(hosts, ", ")
end

local function check_spf(domain, report)
  local record, err = first_txt_matching(domain, "v=spf1")
  if err then
    report.dns_ok = false
    report.spf = "resolver error: " .. err
  elseif not record then
    report.ok = false
    report.spf = "missing"
  else
    report.spf = record
  end
end

-- A domain publishes DKIM under a selector the provider chooses, and nothing
-- in DNS lists the selectors in use — so the known ones are tried in turn.
local function check_dkim(domain, report)
  for _, selector in ipairs(DKIM_SELECTORS) do
    local record, err = first_txt_matching(selector .. "._domainkey." .. domain, "v=DKIM1")
    if err then
      report.dns_ok = false
      report.dkim = "resolver error: " .. err
      return
    end
    if record then
      report.dkim = selector
      return
    end
  end
  report.ok = false
  report.dkim = "missing (tried " .. table.concat(DKIM_SELECTORS, ", ") .. ")"
end

local function check_dmarc(domain, report)
  local record, err = first_txt_matching("_dmarc." .. domain, "v=DMARC1")
  if err then
    report.dns_ok = false
    report.dmarc = "resolver error: " .. err
  elseif not record then
    report.ok = false
    report.dmarc = "missing"
  else
    report.dmarc = record
  end
end

local function check_blacklists(domain, report)
  local hits = {}
  for _, list in ipairs(DNSBLS) do
    local ok, result = pcall(dns.dnsbl, domain, list)
    if not ok then
      report.dns_ok = false
      report.blacklists = "resolver error: " .. tostring(result)
      return
    end
    if result.listed then
      report.ok = false
      hits[#hits + 1] = list .. " (" .. table.concat(result.codes, ", ") .. ")"
    end
  end
  report.blacklists = #hits > 0 and table.concat(hits, "; ") or "clean"
end

local function check(domain)
  local report = { domain = domain, ok = true, dns_ok = true }
  check_mx(domain, report)
  check_spf(domain, report)
  check_dkim(domain, report)
  check_dmarc(domain, report)
  check_blacklists(domain, report)
  -- A domain we could not finish asking about has not passed.
  report.ok = report.ok and report.dns_ok
  return report
end

local domains = {}
for i = 1, #(arg or {}) do
  domains[i] = arg[i]
end
if #domains == 0 then
  error("usage: assay run examples/domain-health.lua -- <domain> [domain...]")
end

local failed = 0
for _, domain in ipairs(domains) do
  local report = check(domain)
  log.info(domain .. ": dns " .. (report.dns_ok and "ok" or "FAILED"))
  log.info("  mx:          " .. report.mx)
  log.info("  spf:         " .. report.spf)
  log.info("  dkim:        " .. report.dkim)
  log.info("  dmarc:       " .. report.dmarc)
  log.info("  blacklists:  " .. report.blacklists)
  if not report.ok then
    failed = failed + 1
    log.warn(domain .. ": FAILED")
  end
end

-- Raising is how a script fails a run under `assay run`; it is what makes this
-- usable as a gate in front of a send.
if failed > 0 then
  error(failed .. " of " .. #domains .. " domain(s) failed")
end
log.info("all " .. #domains .. " domain(s) healthy")
