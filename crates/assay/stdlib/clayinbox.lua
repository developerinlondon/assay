--- @module assay.clayinbox
--- @description Clayinbox mailbox provisioning (app.clayinbox.ai) — the domains a workspace holds, the Google mailboxes on them, and what those boxes bill, listed to the last page. Read-only; the API key comes from the caller.
--- @category saas
--- @icon inbox
--- @keywords clayinbox, mailbox, cold email, domain, dns, spf, dkim, dmarc, deliverability, provisioning
--- @quickref M.client(opts?) -> c | Key via opts.api_key or CLAYINBOX_API_KEY; opts.base_url overrides the endpoint
--- @quickref c:mailboxes() -> [box], meta | nil, err | Every mailbox, paged; {address, domain, status, provider, raw}
--- @quickref c:domains() -> [domain], meta | nil, err | Every domain with the vendor's DNS flags
--- @quickref c:costs() -> {items, meta} | nil, err | What the active fleet bills, from the price on each mailbox row; whole cents
--- @quickref c:available(domain) -> {domain, available, price_cents} | nil, err | Registration quote; a domain you already own is not available and needs no order
--- @quickref c:wallet() -> {available_cents, raw} | nil, err | Credits an order is charged against; an order past the balance is refused by the vendor
--- @quickref c:order(domain, boxes) -> {order_id, raw} | nil, err | Order mailboxes on a domain you already own (import). Usernames are FULL addresses
--- @quickref c:app_password(id) -> password | nil, err | The Google app password, minutes after the box goes active; err.code "not_ready" until then
--- @quickref item -> {kind, unit, ref, quantity, unit_price_cents, period, source} | Shared with assay.forge and assay.salesforge; an absent price or period is a fact the vendor withheld
--- @quickref costs meta -> {priced, currency_known, unpriced, inactive, status_unknown, next_billing_date, wallet_available_cents, wallet_error, truncated, cap, seen} | unpriced, inactive and status_unknown are the three reasons a row went unbilled; currency_known is false because Clayinbox states no currency anywhere
--- @quickref meta -> {truncated, cap, seen} | On every list call; truncated means a cap stopped the walk and rows may be missing

local cost = require("assay.vendor_cost")

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

-- The vendor writes the billing cycle in words. Anything outside this map is a
-- cycle this module cannot price, and a row carrying one is counted rather than
-- guessed at.
local PERIOD = { MONTHLY = "month", YEARLY = "year", ANNUAL = "year", ANNUALLY = "year" }

-- Only a live mailbox is a mailbox the vendor charges for. The status is read
-- exactly as `M.map_box` reads it, so a row billed here is a row a caller can
-- also find in the listing.
local BILLABLE = { active = true }

-- Statuses that plainly mean the vendor has stopped charging.
--
-- A row outside both lists — no status at all, or a word this module has never
-- seen — is not evidence of anything. Counted as inactive it would say the
-- vendor cancelled a box nobody cancelled; billed, it would charge for one that
-- may already be gone. It is counted under its own name instead, so the bill
-- under-reports by an amount the caller can see rather than by a reason that
-- was made up.
local NOT_BILLED = {
  cancelled = true,
  canceled = true,
  suspended = true,
  deleted = true,
  inactive = true,
  expired = true,
  terminated = true,
}

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

  -- The second return is the walk's own account of itself. `truncated` means
  -- the page cap stopped the walk rather than the vendor running out of rows,
  -- so the list is short and the caller is told rather than left to assume it
  -- has everything.
  local function all(path, key, map)
    local out = {}
    local seen = 0
    local truncated = true
    for page = 1, MAX_PAGES do
      local data, err = get(path .. "?limit=" .. PAGE .. "&page=" .. page)
      if not data then return nil, err end
      local rows = type(data[key]) == "table" and data[key] or {}
      local on_page = 0
      for _, raw in ipairs(rows) do
        on_page = on_page + 1
        local row = map(raw)
        if row then out[#out + 1] = row end
      end
      seen = seen + on_page
      local total = data.total_count
      if on_page == 0 or on_page < PAGE then truncated = false break end
      if type(total) == "number" and page * PAGE >= total then truncated = false break end
    end
    return out, { truncated = truncated, cap = MAX_PAGES * PAGE, seen = seen }
  end

  --- A write. GET is not enough for these: the vendor takes an order as a POST
  --- body, and Cloudflare refuses a client with no browser User-Agent outright.
  local function send(method, path, body)
    local ok, resp = pcall(http[method:lower()], base_url .. path, body and json.encode(body) or "", {
      headers = {
        ["x-api-key"] = api_key,
        Accept = "application/json",
        ["Content-Type"] = "application/json",
        ["User-Agent"] = BROWSER_UA,
      },
    })
    if not ok then
      return fail("transport", nil, method .. " " .. path .. ": " .. tostring(resp))
    end
    if resp.status < 200 or resp.status >= 300 then return refused(method .. " " .. path, resp) end
    local parsed_ok, parsed = pcall(json.parse, resp.body or "")
    if not parsed_ok or type(parsed) ~= "table" then
      return fail("unreadable", resp.status, method .. " " .. path .. " answered no object")
    end
    return parsed
  end

  local c = {}

  function c:domains() return all("/domain", "domains", M.map_domain) end

  function c:mailboxes() return all("/mailbox", "mailboxes", M.map_box) end

  --- What registering this domain would cost, and whether it is even for sale.
  ---
  --- A domain the workspace already owns reads as NOT available, which is the
  --- ordinary answer on the import path and not an error: the caller is
  --- ordering boxes on a domain it holds, and only needs the quote when it does
  --- not hold one.
  function c:available(domain)
    local name = lower(domain)
    if name == "" then return fail("config", nil, "available needs a domain") end
    local data, err = get("/domain/available?domain=" .. name)
    if not data then return nil, err end
    return {
      domain = name,
      available = data.available == true,
      price_cents = cost.to_cents(data.price),
      raw = data,
    }
  end

  --- The credit balance an order is charged against.
  ---
  --- Read before ordering rather than after being refused: the vendor answers
  --- an over-balance order with a generic failure, and an operator who was
  --- shown the balance first knows to top up instead of retrying.
  function c:wallet()
    local data, err = get("/wallet")
    if not data then return nil, err end
    -- The same field `costs` reads its balance from, so the number an operator
    -- is shown before an order is the number the bill is drawn against.
    return { available_cents = cost.to_cents(data.available), raw = data }
  end

  --- Order mailboxes on a domain this workspace already owns.
  ---
  --- `import` is what makes it a BYO order: without it the vendor tries to sell
  --- the domain too. The username the vendor wants here is the FULL address,
  --- unlike Primeforge's local part — mixing the two up silently produces
  --- `user@domain@domain`, so this refuses a bare local part rather than
  --- sending one.
  function c:order(domain, boxes)
    local name = lower(domain)
    if name == "" then return fail("config", nil, "order needs a domain") end
    if type(boxes) ~= "table" or #boxes == 0 then
      return fail("config", nil, "order needs at least one mailbox")
    end
    local wanted = {}
    for i, box in ipairs(boxes) do
      local address = lower(box.username or box.address)
      if not address:find("@", 1, true) then
        return fail("config", nil, "order takes full addresses; box " .. i .. " is " .. address)
      end
      if address:match("@([^@]+)$") ~= name then
        return fail("config", nil, address .. " is not on " .. name)
      end
      if trim(box.password) == "" then
        return fail("config", nil, "no password for " .. address)
      end
      wanted[i] = {
        username = address,
        first_name = trim(box.first_name),
        last_name = trim(box.last_name),
        password = box.password,
      }
    end
    local answer, err = send("POST", "/order", {
      import = true,
      data = { { domain_name = name, mailboxes = wanted } },
    })
    if not answer then return nil, err end
    local data = type(answer.data) == "table" and answer.data or answer
    return { order_id = data.order_id or data.id, raw = data }
  end

  --- The Google app password for a box, once the vendor has one.
  ---
  --- It appears some minutes AFTER the mailbox goes active, and until then the
  --- endpoint answers 200 with an empty record. Read as a password that is an
  --- empty string handed to an SMTP connect, which the vendor then refuses for
  --- a reason that has nothing to do with the real one. So an empty record is
  --- its own answer — `not_ready` — and never a password.
  function c:app_password(id)
    local box = trim(id)
    if box == "" then return fail("config", nil, "app_password needs a mailbox id") end
    local data, err = get("/mailbox/" .. box .. "/app-password")
    if not data then return nil, err end
    local password = type(data.app_password) == "string" and data.app_password
      or (type(data) == "string" and data or nil)
    if type(password) ~= "string" or trim(password) == "" then
      return fail("not_ready", nil, "no app password for " .. box .. " yet")
    end
    return password
  end

  -- What the live fleet bills.
  --
  -- The vendor publishes no invoice, order or price endpoint — every one of
  -- those paths 404s — and puts the price on the mailbox row itself, so the
  -- bill is the rows added up. Rows sharing a price and a cycle collapse into
  -- one item, which is what makes `quantity` mean anything; the cycle is part
  -- of the grouping key, so a yearly box never lands in a monthly line at the
  -- same number.
  --
  -- The walk is over raw rows rather than mapped ones: a row `map_box` drops
  -- for an unreadable address is still a row the vendor charges for, and
  -- dropping it here would understate the bill.
  --
  -- A row is set aside for exactly one reason, and the counters say which:
  -- `inactive` for a box the vendor said it has stopped charging for,
  -- `status_unknown` for one whose status it did not say at all, and `unpriced`
  -- for a live box whose price or cycle this module cannot read. Status is
  -- checked first, so a cancelled box with an unreadable price counts once.
  function c:costs()
    local rows, meta = all("/mailbox", "mailboxes", function(raw) return raw end)
    if not rows then return nil, meta end

    local groups, order = {}, {}
    local unpriced, inactive, status_unknown, next_billing = 0, 0, 0, nil
    for _, raw in ipairs(rows) do
      local status = raw.status ~= nil and lower(raw.status) or ""
      local cents = cost.to_cents(raw.cost)
      local period = PERIOD[trim(raw.billing_cycle):upper()]
      if not BILLABLE[status] then
        if NOT_BILLED[status] then
          inactive = inactive + 1
        else
          status_unknown = status_unknown + 1
        end
      elseif cents and period then
        local key = cents .. "/" .. period
        if not groups[key] then
          groups[key] = cost.item({
            kind = "box",
            unit = "mailbox",
            quantity = 0,
            unit_price_cents = cents,
            period = period,
          })
          order[#order + 1] = key
        end
        groups[key].quantity = groups[key].quantity + 1
        -- The dates are the vendor's own ISO-8601 UTC strings, which sort in
        -- the order they happen, so the earliest is the smallest.
        local due = trim(raw.next_billing_date)
        if due ~= "" and (next_billing == nil or due < next_billing) then next_billing = due end
      else
        unpriced = unpriced + 1
      end
    end

    local items = {}
    for _, key in ipairs(order) do items[#items + 1] = groups[key] end

    -- The wallet is the prepaid balance these charges draw down, not a charge
    -- itself. It is fetched second and separately: a wallet the vendor refuses
    -- leaves the bill intact, and the whole typed error is kept rather than
    -- its code alone, because a 401 and a 500 need different answers from the
    -- caller and a nil balance says neither on its own.
    local wallet, wallet_err = get("/wallet")
    return {
      items = items,
      meta = {
        priced = true,
        -- Not on a mailbox row, not on a domain row, not on the wallet.
        currency_known = false,
        unpriced = unpriced,
        inactive = inactive,
        status_unknown = status_unknown,
        next_billing_date = next_billing,
        truncated = meta.truncated,
        cap = meta.cap,
        seen = meta.seen,
        wallet_available_cents = wallet and cost.to_cents(wallet.available) or nil,
        wallet_error = wallet_err,
      },
    }
  end

  return c
end

return M
