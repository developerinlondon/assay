--- @module assay.forge
--- @description Primeforge and Warmforge over the shared forge MCP endpoint — domains, mailboxes, warm-up progress, placement tests, the DNS health report, and Primeforge's registration quote. Neither product has a billing endpoint, so callers price the recurring spend from configuration. Read-only; keys and workspace come from the caller.
--- @category saas
--- @icon flame
--- @keywords primeforge, warmforge, forge, mcp, mailbox, warmup, heat score, placement, blacklist, spf, dkim, dmarc, cold email
--- @quickref M.mcp(product, key, tool, args?, opts?) -> payload | nil, err | One JSON-RPC tools/call; product is "primeforge" or "warmforge"
--- @quickref M.primeforge(opts) -> p | Key via opts.api_key or PRIMEFORGE_API_KEY; opts.workspace_id required
--- @quickref p:domains() -> [domain], meta | nil, err | Name is sld and tld joined; the vendor sends no whole name
--- @quickref p:mailboxes(domain_id?) -> [box], meta | nil, err | The vendor caps this at ten rows and cannot page
--- @quickref p:domain_price(domain) -> {items, meta} | nil, err | Registration quote in whole cents for a year; the only price the forge answers, and it prices domains nobody has bought yet
--- @quickref p:buy_domain(domain, contact) -> {domain, raw} | nil, err | Registers it and charges the stored card; the contact is the registrant the registry requires
--- @quickref p:create_mailboxes(domain_id, boxes) -> {created, raw} | nil, err | username is the LOCAL PART; a full address is refused, not silently doubled
--- @quickref p:app_password(id) -> password | nil, err | The box's Google app password; err.code "not_ready" while the vendor still has none
--- @quickref primeforge costs -> quote only | No billing, subscription or slot tool: p:domain_price quotes a purchase, never the recurring spend
--- @quickref item -> {kind, unit, ref, quantity, unit_price_cents, period, source} | Shared with assay.clayinbox and assay.salesforge; an absent price or period is a fact the vendor withheld
--- @quickref meta -> {truncated, cap, seen} | On every list call; truncated means a cap stopped the walk and rows may be missing
--- @quickref M.warmforge(opts) -> w | Key via opts.api_key or WARMFORGE_API_KEY; opts.workspace_id required
--- @quickref w:mailboxes() -> [box], meta | nil, err | Every page, by totalPages
--- @quickref w:warmup(address) -> {day, total_days, heat, enabled} | nil, err | Position on the curve
--- @quickref w:placement(address) -> {inbox, spam, promotions} | nil | nil, err | Latest placement test; nil when none has run
--- @quickref w:health(address) -> {spf, dkim, dmarc, mx, heat, blacklists} | nil, err | Each check reads valid, invalid or unknown
--- @quickref warmforge costs -> none | Warmforge has no billing endpoint at all: no subscription, slot or price data; callers price warm-up from configuration

local cost = require("assay.vendor_cost")

local M = {}

local ENDPOINT = "https://mcp.salesforge.ai/mcp"

-- One endpoint, two products; the header is the whole of the difference.
local HEADER = { primeforge = "X-Primeforge-Key", warmforge = "X-Warmforge-Key" }

-- Cloudflare answers a client with no browser User-Agent with error 1010.
local BROWSER_UA = "Mozilla/5.0 (X11; Linux x86_64; rv:130.0) Gecko/20100101 Firefox/130.0"

local WARMFORGE_PAGE = 50
local MAX_PAGES = 50

-- Primeforge's list tools answer ten rows and offer no way to ask for an
-- eleventh: probed live, `limit` and `offset` are ignored and offset 10 and
-- offset 20 return the same ten ids. A vendor answering exactly its cap has
-- told the caller nothing about what lies past it, so that reads as truncated.
local PRIMEFORGE_CAP = 10

-- What the registry wants before it will sell a domain. The vendor rejects a
-- partial contact, and finding that out costs a round trip against a card.
local REGISTRANT_FIELDS = {
  "firstName", "lastName", "email", "phone",
  "address", "city", "state", "zip", "country",
}

-- A Primeforge mailbox row carries the box's own password and its Google app
-- password. `raw` is for the vendor's metadata; a credential riding along on it
-- would reach every log that prints a row.
local SECRET_KEYS = { password = true, appPassword = true, app_password = true }

-- A health check the report does not mention has not been run. Reading that as
-- a failure tells an operator a record they published is missing, so an absent
-- or unreadable check is "unknown" and never "invalid".
local PASSED = { valid = true, ok = true, passed = true, success = true }

-- A Primeforge domain's `expiresAt` lands one year after its `createdAt`, so
-- the registration the quote prices is a year of it.
local DOMAIN_PERIOD = "year"

local ERR = { __tostring = function(e) return "forge: " .. e.message end }

local function fail(code, status, message)
  return nil, setmetatable({ code = code, status = status, message = message }, ERR)
end

local function trim(s) return (tostring(s or ""):gsub("^%s+", ""):gsub("%s+$", "")) end
local function lower(s) return trim(s):lower() end
local function num(v) return type(v) == "number" and v == v and v or 0 end

local function redact(raw)
  local out = {}
  for k, v in pairs(raw) do out[k] = SECRET_KEYS[k] and "[redacted]" or v end
  return out
end

--- The frame this call asked for.
---
--- A Streamable-HTTP MCP endpoint answers either with a JSON body or with an
--- SSE stream, and a stream can carry frames nobody asked for — a server
--- notification may follow the reply. So the frame is chosen by its JSON-RPC id
--- rather than by position: taking the last one reads a notification as an
--- answer. Both encodings are tried whatever the content type says, because a
--- stream served under the wrong one is still a stream.
local function frame_for_id(body, id)
  local ok, parsed = pcall(json.parse, body or "")
  if ok and type(parsed) == "table" and parsed.id == id then return parsed end
  for line in (tostring(body or "") .. "\n"):gmatch("([^\n]*)\n") do
    local payload = line:match("^data:%s*(.+)$")
    if payload then
      local frame_ok, frame = pcall(json.parse, payload)
      if frame_ok and type(frame) == "table" and frame.id == id then return frame end
    end
  end
  return nil
end

--- One `tools/call` against the forge MCP endpoint.
---
--- The tool's own payload arrives as a JSON string nested in the reply's text
--- part, so the envelope is parsed twice. A text part that is not JSON is
--- returned as the string it is rather than discarded.
function M.mcp(product, key, tool, args, opts)
  opts = opts or {}
  local header = HEADER[product]
  if not header then
    return fail("product", nil, "unknown forge product " .. tostring(product))
  end
  if not key or trim(key) == "" then
    return fail("config", nil, product .. ": api key required")
  end
  local target = (opts.base_url or ENDPOINT)
  local id = 1
  local ok, resp = pcall(http.post, target, json.encode({
    jsonrpc = "2.0",
    id = id,
    method = "tools/call",
    params = { name = tool, arguments = args or {} },
  }), {
    headers = {
      [header] = key,
      ["Content-Type"] = "application/json",
      Accept = "application/json, text/event-stream",
      ["User-Agent"] = BROWSER_UA,
    },
  })
  if not ok then return fail("transport", nil, tool .. ": " .. tostring(resp)) end
  if resp.status == 401 or resp.status == 403 then
    return fail("auth", resp.status, tool .. " rejected the key (HTTP " .. resp.status .. ")")
  end
  if resp.status == 429 then
    return fail("rate_limit", 429, tool .. " rate limited (HTTP 429)")
  end
  if resp.status >= 500 then
    return fail("server", resp.status, tool .. " HTTP " .. resp.status)
  end
  if resp.status ~= 200 then
    return fail("http", resp.status, tool .. " HTTP " .. resp.status)
  end
  local reply = frame_for_id(resp.body, id)
  if not reply then
    return fail("unreadable", resp.status, tool .. ": no reply frame carried this call's id")
  end
  if type(reply.error) == "table" then
    return fail("tool", resp.status, tool .. ": " .. tostring(reply.error.message or "forge MCP error"))
  end
  local content = reply.result and reply.result.content
  local text = type(content) == "table" and type(content[1]) == "table" and content[1].text
  if type(text) ~= "string" then
    return fail("unreadable", resp.status, tool .. ": the reply carried no content")
  end
  local parsed_ok, parsed = pcall(json.parse, text)
  if parsed_ok then return parsed end
  return text
end

--- The rows a forge list tool returns, whichever key this one puts them under.
function M.rows(payload)
  if type(payload) ~= "table" then return {} end
  local rows = payload.results or payload.mailboxes or payload.domains
  if type(rows) == "table" then return rows end
  return payload[1] ~= nil and payload or {}
end

--- A Primeforge domain row.
---
--- The vendor answers the name in two halves — `sld` and `tld` — and never as
--- one string. A reader that looks only for a whole name finds no domains at
--- all, which is how live mailboxes came to sit under no domain.
function M.map_domain(raw)
  local sld, tld = lower(raw.sld), lower(raw.tld)
  local domain = (sld ~= "" and tld ~= "") and (sld .. "." .. tld) or lower(raw.domain)
  if domain == "" then return nil end
  return {
    domain = domain,
    provider = "primeforge",
    provider_ref = raw.id,
    status = raw.status ~= nil and lower(raw.status) or "unknown",
    raw = redact(raw),
  }
end

--- A mailbox row from either product. Warmforge keys a box by its address and
--- Primeforge by an id, so the reference falls back to the address.
function M.map_box(raw, product)
  local address = lower(raw.address) ~= "" and lower(raw.address) or lower(raw.username)
  if address == "" or not address:find("@", 1, true) then return nil end
  return {
    address = address,
    domain = address:match("@([^@]+)$"),
    status = raw.status ~= nil and lower(raw.status) or "unknown",
    provider = product,
    provider_ref = raw.id or (product == "warmforge" and address or nil),
    raw = redact(raw),
  }
end

--- A heat score outside 0..100, or not a number at all, is not a reading.
function M.heat(raw)
  if type(raw) ~= "number" or raw ~= raw then return nil end
  local n = math.floor(raw + 0.5)
  if n < 0 or n > 100 then return nil end
  return n
end

--- A placement test as fractions of one.
---
--- Vendors report either seed counts or percentages and the two are
--- indistinguishable at a glance — `{inbox = 80}` is both — so anything summing
--- above one is read as counts and divided through. Nothing at all is a test
--- nobody has run, which is not a placement of zero.
function M.placement(inbox, spam, promotions)
  local function n(v) return (type(v) == "number" and v == v and v >= 0) and v or 0 end
  local parts = { inbox = n(inbox), spam = n(spam), promotions = n(promotions) }
  local total = parts.inbox + parts.spam + parts.promotions
  if total <= 0 then return nil end
  local scale = total > 1.000001 and total or 1
  return {
    inbox = parts.inbox / scale,
    spam = parts.spam / scale,
    promotions = parts.promotions / scale,
  }
end

--- Warmforge's health report, which is a read of public DNS under another name.
--- Each check reads "valid", "invalid" or "unknown"; a check the report omits
--- is unknown, because the vendor leaves out what it did not run.
function M.health(raw)
  local report = type(raw.healthReport) == "table" and raw.healthReport or {}
  local function tri(key)
    local entry = report[key]
    if type(entry) ~= "table" then return "unknown" end
    local status = lower(entry.status)
    if status == "" then return "unknown" end
    return PASSED[status] and "valid" or "invalid"
  end
  local blacklists = type(report.blacklists) == "table" and report.blacklists or {}
  local listed = {}
  for _, check in ipairs(type(blacklists.checks) == "table" and blacklists.checks or {}) do
    if check.detected == true then listed[#listed + 1] = check.id or check.name end
  end
  return {
    spf = tri("spf"),
    dkim = tri("dkim"),
    dmarc = tri("dmarc"),
    mx = tri("mx"),
    heat = M.heat(report.heatScore),
    blacklists = { detected = #listed, lists = listed },
    checked_at = report.lastCheckedAt,
  }
end

--- Where a mailbox is on its warm-up curve.
---
--- The vendor reports the two halves — days done and days left — so the length
--- of the curve is their sum rather than a constant this module would have to
--- keep in step with the vendor's.
function M.warmup(raw)
  local done = num(raw.warmupDaysCompleted)
  local report = type(raw.healthReport) == "table" and raw.healthReport or {}
  return {
    day = done,
    total_days = done + num(raw.warmupDaysLeft),
    heat = M.heat(report.heatScore),
    enabled = raw.warmupEnabled == true,
  }
end

local function config(product, opts, env_key)
  opts = opts or {}
  local key = opts.api_key or env.get(env_key)
  if not key or trim(key) == "" then
    error(product .. ": api key required (opts.api_key or " .. env_key .. ")")
  end
  local workspace = opts.workspace_id
  if not workspace or trim(workspace) == "" then
    error(product .. ": workspace_id required")
  end
  return key, trim(workspace), opts
end

--- Primeforge: the product that sells the domain and the box.
function M.primeforge(opts)
  local key, workspace, o = config("primeforge", opts, "PRIMEFORGE_API_KEY")
  local p = {}

  local function call(tool, args)
    return M.mcp("primeforge", key, tool, args, o)
  end

  function p:domains()
    local payload, err = call("primeforge_list_domains", { workspaceId = workspace })
    if not payload then return nil, err end
    local rows = M.rows(payload)
    local out = {}
    for _, raw in ipairs(rows) do
      local row = M.map_domain(raw)
      if row then out[#out + 1] = row end
    end
    return out, { truncated = #rows == PRIMEFORGE_CAP, cap = PRIMEFORGE_CAP, seen = #rows }
  end

  -- `primeforge_list_mailboxes` accepts `workspaceId` and nothing else, so the
  -- domain filter is applied to what the vendor gives rather than sent. That
  -- makes an empty result ambiguous — the domain may have no boxes, or its
  -- boxes may sit outside the ten-row window — which is exactly what `meta`
  -- exists to disambiguate. A caller that ignores it can read a truncated
  -- window as a domain with nothing on it.
  function p:mailboxes(domain_id)
    local payload, err = call("primeforge_list_mailboxes", { workspaceId = workspace })
    if not payload then return nil, err end
    local rows = M.rows(payload)
    local out = {}
    for _, raw in ipairs(rows) do
      local row = M.map_box(raw, "primeforge")
      if row and (domain_id == nil or raw.domainId == domain_id) then out[#out + 1] = row end
    end
    return out, { truncated = #rows == PRIMEFORGE_CAP, cap = PRIMEFORGE_CAP, seen = #rows }
  end

  -- What Primeforge would charge to register a domain.
  --
  -- This is the only price either product answers. There is no billing,
  -- subscription or slot tool on the endpoint, and a domain already bought
  -- carries no price on its row — so this quotes what is still for sale and
  -- says nothing about what the workspace is being charged today.
  --
  -- The search answers the name asked for plus prefixed alternatives, and each
  -- carries its own price. A row is set aside for exactly one reason and the
  -- counters say which: `unavailable` for a name nobody can buy, since a price
  -- for that is not a cost anyone can incur, and `unpriced` for a name that is
  -- for sale but that the vendor quoted with nothing readable. Collapsing the
  -- two would report a domain as sold when it is only unquoted.
  function p:domain_price(domain)
    local fqdn = lower(domain)
    if fqdn == "" then return fail("config", nil, "domain_price needs a domain") end
    local payload, err = call("primeforge_search_domains", { domain = fqdn })
    if not payload then return nil, err end
    local rows = (type(payload) == "table" and type(payload.domains) == "table")
      and payload.domains or {}
    local items, unavailable, unpriced = {}, 0, 0
    for _, raw in ipairs(rows) do
      local name = lower(raw.name)
      if name ~= "" then
        local cents = cost.to_cents(raw.price)
        if raw.available ~= true then
          unavailable = unavailable + 1
        elseif cents then
          items[#items + 1] = cost.item({
            kind = "domain",
            unit = "domain",
            ref = name,
            quantity = 1,
            unit_price_cents = cents,
            period = DOMAIN_PERIOD,
          })
        else
          unpriced = unpriced + 1
        end
      end
    end
    return {
      items = items,
      meta = {
        priced = true,
        -- The quote is a bare number with no currency beside it.
        currency_known = false,
        seen = #rows,
        unavailable = unavailable,
        unpriced = unpriced,
      },
    }
  end

  --- Register a domain. This SPENDS: it charges the card Primeforge holds.
  ---
  --- The registry requires a full registrant contact and the vendor rejects a
  --- partial one, so the fields are checked here rather than after the charge
  --- attempt. Nothing about the price is decided here — `domain_price` is the
  --- quote, and a caller that has not shown it to somebody is buying blind.
  function p:buy_domain(domain, contact)
    local fqdn = lower(domain)
    if fqdn == "" or not fqdn:find(".", 1, true) then
      return fail("config", nil, "buy_domain needs a domain")
    end
    if type(contact) ~= "table" then
      return fail("config", nil, "buy_domain needs a registrant contact")
    end
    for _, field in ipairs(REGISTRANT_FIELDS) do
      if trim(contact[field]) == "" then
        return fail("config", nil, "buy_domain needs the registrant's " .. field)
      end
    end
    local payload, err = call("primeforge_buy_domains", {
      workspaceId = workspace,
      domains = { fqdn },
      contact = contact,
    })
    if not payload then return nil, err end
    return { domain = fqdn, raw = payload }
  end

  --- Create mailboxes on a domain this workspace holds. This SPENDS: a box
  --- occupies a slot, and slots are billed whether or not a box sits in one.
  ---
  --- `username` is the LOCAL PART. The vendor accepts a full address silently
  --- and stores it doubled — `ada@brand.test@brand.test` — which is a mailbox
  --- nothing can send from and which cannot be renamed afterwards, only
  --- deleted, and a deleted address tombstones. So a full address is refused
  --- here instead.
  function p:create_mailboxes(domain_id, boxes)
    local domain = trim(domain_id)
    if domain == "" then return fail("config", nil, "create_mailboxes needs a domain id") end
    if type(boxes) ~= "table" or #boxes == 0 then
      return fail("config", nil, "create_mailboxes needs at least one mailbox")
    end
    local wanted = {}
    for i, box in ipairs(boxes) do
      local username = lower(box.username)
      if username == "" then
        return fail("config", nil, "no username for mailbox " .. i)
      end
      if username:find("@", 1, true) then
        return fail("config", nil, "username is the local part, not an address: " .. username)
      end
      local first = trim(box.first_name)
      local last = trim(box.last_name)
      if first == "" then return fail("config", nil, "no first name for " .. username) end
      wanted[i] = {
        firstName = first,
        lastName = last,
        username = username,
        signature = trim(box.signature) ~= "" and box.signature or trim(first .. " " .. last),
        profilePictureUrl = box.profile_picture_url or "",
      }
    end
    local payload, err = call("primeforge_create_mailboxes_for_domain", {
      workspaceId = workspace,
      domainId = domain,
      mailboxes = wanted,
    })
    if not payload then return nil, err end
    return { created = #wanted, raw = payload }
  end

  --- The box's Google app password, which is how it is wired to a sequencer.
  ---
  --- Absent until the vendor has finished provisioning. An empty one handed to
  --- an SMTP connect is refused for a reason that has nothing to do with the
  --- real one, so "not yet" is its own answer here and never a password.
  function p:app_password(id)
    local box = trim(id)
    if box == "" then return fail("config", nil, "app_password needs a mailbox id") end
    local payload, err = call("primeforge_get_mailbox", { id = box })
    if not payload then return nil, err end
    local password = type(payload) == "table" and payload.appPassword or nil
    if type(password) ~= "string" or trim(password) == "" then
      return fail("not_ready", nil, "no app password for " .. box .. " yet")
    end
    return password
  end

  return p
end

--- Warmforge: the product that warms a box someone else provisioned.
function M.warmforge(opts)
  local key, workspace, o = config("warmforge", opts, "WARMFORGE_API_KEY")
  local w = {}
  local cached, cached_meta

  local function call(tool, args)
    return M.mcp("warmforge", key, tool, args, o)
  end

  -- `page` and `page_size` are not optional on this tool: omitting them fails.
  -- `totalPages` is the stop condition, and the page cap bounds a vendor whose
  -- count never runs out.
  local function raw_rows()
    if cached then return cached, cached_meta end
    local out = {}
    local truncated = true
    for page = 1, MAX_PAGES do
      local payload, err = call("warmforge_list_mailboxes", {
        workspaceId = workspace,
        page = page,
        page_size = WARMFORGE_PAGE,
      })
      if not payload then return nil, err end
      local rows = M.rows(payload)
      for _, raw in ipairs(rows) do out[#out + 1] = raw end
      -- `totalPages` is the vendor's own count and wins outright: a short page
      -- before the last one would otherwise stop the walk early. Only a vendor
      -- that reports no count at all falls back to the short-page rule.
      local pages = type(payload) == "table" and payload.totalPages
      if #rows == 0 then truncated = false break end
      if type(pages) == "number" then
        if page >= pages then truncated = false break end
      elseif #rows < WARMFORGE_PAGE then
        truncated = false
        break
      end
    end
    cached = out
    cached_meta = { truncated = truncated, cap = MAX_PAGES * WARMFORGE_PAGE, seen = #out }
    return out, cached_meta
  end

  -- An address missing from a truncated window is not an address the workspace
  -- lacks, so the error says which of the two happened rather than letting a
  -- short list read as a deleted mailbox.
  local function row_for(address)
    local rows, meta = raw_rows()
    if not rows then return nil, meta end
    local wanted = lower(address)
    for _, raw in ipairs(rows) do
      if lower(raw.address) == wanted then return raw end
    end
    local why = meta.truncated and " (the listing was truncated at " .. meta.seen .. " rows)" or ""
    return fail("not_found", nil,
      "no mailbox " .. wanted .. " in workspace " .. workspace .. why)
  end

  function w:mailboxes()
    local rows, meta = raw_rows()
    if not rows then return nil, meta end
    local out = {}
    for _, raw in ipairs(rows) do
      local row = M.map_box(raw, "warmforge")
      if row then out[#out + 1] = row end
    end
    return out, meta
  end

  function w:warmup(address)
    local raw, err = row_for(address)
    if not raw then return nil, err end
    return M.warmup(raw)
  end

  function w:health(address)
    local raw, err = row_for(address)
    if not raw then return nil, err end
    return M.health(raw)
  end

  -- The tool answers a row per mailbox whether or not a placement test has
  -- ever run for it; a row carrying no folder counts is a test nobody has
  -- taken, so this returns nothing rather than a placement of zero.
  function w:placement(address)
    local raw, err = row_for(address)
    if not raw then return nil, err end
    local payload, call_err = call("warmforge_get_latest_mailbox_placement_results", {
      workspaceId = workspace,
      mailboxIds = { raw.id or lower(address) },
    })
    if not payload then return nil, call_err end
    local first = M.rows(payload)[1]
    if type(first) ~= "table" then return nil end
    local src = type(first.placement) == "table" and first.placement or first
    return M.placement(
      src.inbox or src.inboxCount,
      src.spam or src.spamCount,
      src.promotions or src.promotionsCount or src.categories
    )
  end

  return w
end

return M
