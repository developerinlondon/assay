--- @module assay.clayinbox
--- @description Clayinbox mailbox provisioning (app.clayinbox.ai) — the domains a workspace holds and the Google mailboxes on them, listed to the last page. Read-only; the API key comes from the caller.
--- @category saas
--- @icon inbox
--- @keywords clayinbox, mailbox, cold email, domain, dns, spf, dkim, dmarc, deliverability, provisioning
--- @quickref M.client(opts?) -> c | Key via opts.api_key or CLAYINBOX_API_KEY; opts.base_url overrides the endpoint
--- @quickref c:mailboxes() -> [box] | nil, err | Every mailbox, paged; {address, domain, status, provider, raw}
--- @quickref c:domains() -> [domain] | nil, err | Every domain with the vendor's DNS flags

local M = {}

local BASE = "https://app.clayinbox.ai/api/v1"
local PAGE = 100

-- Cloudflare answers a client with no browser User-Agent with error 1010, and
-- serves that block page as HTML under an HTTP 200.
local BROWSER_UA = "Mozilla/5.0 (X11; Linux x86_64; rv:130.0) Gecko/20100101 Firefox/130.0"

-- A vendor that keeps answering with a full page and a total it never reaches
-- would loop forever. 50 pages is 5,000 rows, past any fleet this serves.
local MAX_PAGES = 50

-- A list row carries the mailbox's own password. `raw` exists so a caller can
-- read the fields this module does not map, and a credential riding along on it
-- would reach every log that prints a row.
local SECRET_KEYS = { password = true, app_password = true }

local ERR = { __tostring = function(e) return "clayinbox: " .. e.message end }

local function fail(code, status, message)
  return nil, setmetatable({ code = code, status = status, message = message }, ERR)
end

local function trim(s) return (tostring(s or ""):gsub("^%s+", ""):gsub("%s+$", "")) end
local function lower(s) return trim(s):lower() end

local function redact(raw)
  local out = {}
  for k, v in pairs(raw) do out[k] = SECRET_KEYS[k] and "[redacted]" or v end
  return out
end

local function refused(where, resp)
  if resp.status == 401 or resp.status == 403 then
    return fail("auth", resp.status, where .. " rejected the key (HTTP " .. resp.status .. ")")
  end
  if resp.status == 429 then
    return fail("rate_limit", 429, where .. " rate limited (HTTP 429)")
  end
  if resp.status >= 500 then
    return fail("server", resp.status, where .. " HTTP " .. resp.status)
  end
  return fail("http", resp.status, where .. " HTTP " .. resp.status)
end

--- One domain row: the name, the vendor's id, and its read of the DNS it asked
--- the operator to publish. A flag the vendor omits is a record it has not seen
--- published, which is why an absent one is false rather than unknown.
function M.map_domain(raw)
  local domain = lower(raw.domain)
  if domain == "" then return nil end
  return {
    domain = domain,
    provider = "clayinbox",
    provider_ref = raw.domain_id,
    status = raw.status ~= nil and lower(raw.status) or "unknown",
    dns = {
      spf = raw.spf == true,
      dkim = raw.dkim == true,
      dmarc = raw.dmarc == true,
      mx = raw.mx_records == true,
    },
    raw = redact(raw),
  }
end

--- One mailbox row. `username` is the full address, and the domain arrives
--- nested under `domains` rather than beside it. A row whose address cannot be
--- read is dropped: a half-built row is not a mailbox a caller can act on.
function M.map_box(raw)
  local address = lower(raw.username)
  local nested = type(raw.domains) == "table" and lower(raw.domains.domain) or ""
  local domain = nested ~= "" and nested or address:match("@([^@]+)$")
  if address == "" or not domain or domain == "" then return nil end
  if address:sub(-(#domain + 1)) ~= ("@" .. domain) then return nil end
  return {
    address = address,
    domain = domain,
    status = raw.status ~= nil and lower(raw.status) or "unknown",
    provider = "clayinbox",
    provider_ref = raw.id,
    raw = redact(raw),
  }
end

--- Build a client. The key is the caller's to supply — this module reads no
--- secret store — and `base_url` exists so a test can stand a server in front
--- of it.
function M.client(opts)
  opts = opts or {}
  local api_key = opts.api_key or env.get("CLAYINBOX_API_KEY")
  if not api_key or trim(api_key) == "" then
    error("clayinbox: api key required (opts.api_key or CLAYINBOX_API_KEY)")
  end
  local base_url = (opts.base_url or BASE):gsub("/+$", "")

  local function get(path)
    local ok, resp = pcall(http.get, base_url .. path, {
      headers = {
        ["x-api-key"] = api_key,
        Accept = "application/json",
        ["User-Agent"] = BROWSER_UA,
      },
    })
    if not ok then return fail("transport", nil, "GET " .. path .. ": " .. tostring(resp)) end
    if resp.status ~= 200 then return refused("GET " .. path, resp) end
    local parsed_ok, parsed = pcall(json.parse, resp.body or "")
    -- Cloudflare's block page is HTML under a 200. Read as an empty list it
    -- would report that every domain the workspace holds had vanished.
    if not parsed_ok or type(parsed) ~= "table" or type(parsed.data) ~= "table" then
      return fail("unreadable", resp.status, "GET " .. path .. " answered without a data object")
    end
    return parsed.data
  end

  local function all(path, key, map)
    local out = {}
    for page = 1, MAX_PAGES do
      local data, err = get(path .. "?limit=" .. PAGE .. "&page=" .. page)
      if not data then return nil, err end
      local rows = type(data[key]) == "table" and data[key] or {}
      local seen = 0
      for _, raw in ipairs(rows) do
        seen = seen + 1
        local row = map(raw)
        if row then out[#out + 1] = row end
      end
      local total = data.total_count
      if seen == 0 or seen < PAGE then break end
      if type(total) == "number" and page * PAGE >= total then break end
    end
    return out
  end

  local c = {}

  function c:domains() return all("/domain", "domains", M.map_domain) end

  function c:mailboxes() return all("/mailbox", "mailboxes", M.map_box) end

  return c
end

return M
