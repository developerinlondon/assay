--- @module assay.salesforge
--- @description Salesforge sequencer — the public REST API for workspaces, mailboxes, sequences, contacts, do-not-contact and replies, plus the web app's own API for the warm-up state and the plan the public one does not carry. Credentials come from the caller.
--- @category saas
--- @icon send
--- @keywords salesforge, sequencer, cold email, sequence, contact, enrol, dnc, mailbox, warmup, reply
--- @quickref M.client(opts) -> c | Key via opts.api_key or SALESFORGE_API_KEY; opts.workspace_id required
--- @quickref M.client{email, password} -> c | Or SALESFORGE_EMAIL and SALESFORGE_PASSWORD; only the internal API uses them
--- @quickref c:workspaces() -> [workspace], meta | nil, err | Every workspace the key can see
--- @quickref c:mailboxes() -> [box], meta | nil, err | Connected mailboxes; the public API carries no warm-up state
--- @quickref c:sequences() -> [sequence], meta | nil, err | Every sequence in the workspace
--- @quickref c:sequence(id) -> sequence | nil, err | One sequence, with its mailbox rotation
--- @quickref c:create_contact(fields) -> contact | nil, err | firstName is required by the vendor
--- @quickref c:enrol(sequence_id, contact_ids) -> true | nil, err | Assign contacts to a sequence
--- @quickref c:dnc(addresses) -> true | nil, err | Stop writing to these addresses
--- @quickref c:reply(mailbox_id, email_id, body) -> true | nil, err | Reply on an existing thread
--- @quickref c:set_rotation(sequence_id, mailbox_ids) -> true | nil, err | Replace which mailboxes a sequence sends from; an empty list clears it
--- @quickref c:set_sequence_status(sequence_id, status) -> true | nil, err | "paused" or "active"; c:sequence(id) reads it back
--- @quickref c:sign_in() -> true | nil, err | Firebase password sign-in for the internal API; memoised, token never returned
--- @quickref c:mailboxes_internal() -> [box], meta | nil, err | Warm-up state: warmupActivated, daysUntilWarm, heat
--- @quickref c:mailbox_internal(id) -> box | nil, err | One box with its warm-up state, read from the web app's API
--- @quickref c:mailbox_id(id_or_address) -> id | nil, err | The vendor's id behind an address; an id passes through
--- @quickref c:connect_smtp(address, password, opts) -> box | nil, err | Connect a mailbox over SMTP/IMAP; opts.first/last/smtp/imap/daily_limit
--- @quickref c:set_warmup(id_or_address, on) -> box | nil, err | Switch warm-up on or off and read the vendor's answer back
--- @quickref c:costs() -> {items, meta} | nil, err | The plan, its monthly limits and the credits left; the vendor names no price at all, and meta.priced says so
--- @quickref item -> {kind, unit, ref, quantity, unit_price_cents, period, source} | Shared with assay.clayinbox and assay.forge; an absent price or period is a fact the vendor withheld
--- @quickref meta -> {truncated, cap, seen} | On every list call; truncated means a cap stopped the walk and rows may be missing

local cost = require("assay.vendor_cost")

local M = {}

local PUBLIC_BASE = "https://api.salesforge.ai/public/v2"
local INTERNAL_BASE = "https://api.salesforge.ai"
local IDENTITY_URL = "https://identitytoolkit.googleapis.com/v1/accounts:signInWithPassword"

-- Salesforge's Firebase project key. It ships in every copy of the web app's
-- front-end bundle and identifies the project, not an account.
local FIREBASE_WEB_API_KEY = "AIzaSyCSvPu4xQeXnowWbgt2uRFGwAuMhkbJo-o"

-- Cloudflare answers a client with no browser User-Agent with error 1010.
local BROWSER_UA = "Mozilla/5.0 (X11; Linux x86_64; rv:130.0) Gecko/20100101 Firefox/130.0"

local SEQUENCE_STATUS = { paused = true, active = true }

-- Where a mailbox is reached when the caller does not say. Every box the fleet
-- vendors hand out is Google Workspace and Salesforge stores whatever host it
-- is given, so this is a default for the common case rather than a fact about
-- an address. A box anywhere else names its own host.
local DEFAULT_SMTP = { host = "smtp.gmail.com", port = 587 }
local DEFAULT_IMAP = { host = "imap.gmail.com", port = 993 }

local PAGE = 100
local INTERNAL_PAGE = 50
local MAX_PAGES = 20

-- The plan's monthly ceilings and the credit pools they refill, under the names
-- the vendor gives them. Both are entitlements rather than charges, so they
-- ride in `meta` instead of being dressed up as priced items.
local PLAN_LIMITS = {
  { field = "emailsPerMonthLimit", name = "emails_per_month" },
  { field = "activatedLeadsPerMonthLimit", name = "activated_leads_per_month" },
  { field = "validationsPerMonthLimit", name = "validations_per_month" },
  { field = "personalizationsPerMonthLimit", name = "personalizations_per_month" },
  { field = "socialActionsPerMonthLimit", name = "social_actions_per_month" },
  { field = "linkedInProfilesLimit", name = "linkedin_profiles" },
}

-- The words the vendor uses for a billing cycle, wherever it happens to put
-- one. A plan carrying none is a plan whose cycle the vendor never stated, and
-- the item then carries no period at all rather than a guessed month.
local PLAN_PERIOD = {
  MONTH = "month",
  MONTHLY = "month",
  YEAR = "year",
  YEARLY = "year",
  ANNUAL = "year",
  ANNUALLY = "year",
}

local PLAN_CREDITS = {
  { field = "emailCreditsLeft", name = "emails" },
  { field = "leadCreditsLeft", name = "leads" },
  { field = "emailValidationCreditsLeft", name = "validations" },
  { field = "personalizationCreditsLeft", name = "personalizations" },
  { field = "socialActionCreditsLeft", name = "social_actions" },
}

local ERR = { __tostring = function(e) return "salesforge: " .. e.message end }

local function fail(code, status, message)
  return nil, setmetatable({ code = code, status = status, message = message }, ERR)
end

local function trim(s) return (tostring(s or ""):gsub("^%s+", ""):gsub("%s+$", "")) end
local function lower(s) return trim(s):lower() end

--- A public mailbox row. The public API lists what the workspace is connected
--- to and carries no warm-up state at all — that lives on the internal API.
function M.map_box(raw)
  local address = lower(raw.address)
  if address == "" or not address:find("@", 1, true) then return nil end
  return {
    address = address,
    domain = address:match("@([^@]+)$"),
    status = raw.status ~= nil and lower(raw.status) or "unknown",
    provider = "salesforge",
    provider_ref = raw.id,
    daily_limit = raw.dailyEmailLimit,
    mailbox_provider = raw.mailboxProvider,
    -- Listed by the sequencer at all means the sequencer holds it.
    connected = true,
    raw = raw,
  }
end

--- An internal mailbox row, which is the only place the warm-up state appears.
---
--- `warmup` is nil when the switch is off: a connected box nobody is warming is
--- at day nothing of nothing, and a zero-of-fourteen would read as a curve that
--- had started. `reputationScore` was absent from every live row on 2026-09-04,
--- so an absent heat is a reading nobody has yet rather than a heat of zero.
function M.map_internal_box(raw)
  local address = lower(raw.address)
  if address == "" or not address:find("@", 1, true) then return nil end
  local warmup
  if raw.warmupActivated == true then
    local left = raw.daysUntilWarm
    if type(left) == "number" and left == left then
      warmup = { days_until_warm = math.max(0, math.floor(left + 0.5)) }
    else
      warmup = {}
    end
    warmup.heat = M.heat(raw.reputationScore)
    warmup.activated = true
  end
  return {
    address = address,
    domain = address:match("@([^@]+)$"),
    status = raw.status ~= nil and lower(raw.status) or "unknown",
    provider = "salesforge",
    provider_ref = raw.id,
    warmup = warmup,
    raw = raw,
  }
end

-- What a mailbox is reached at. The vendor's documented mailbox object carries
-- none of these back, but the create call SENDS them, and a row that echoed
-- what it was given would carry a plaintext password out through `raw`.
local TRANSPORT_KEYS = { smtp = true, imap = true, password = true, appPassword = true }

--- A vendor row with anything that could carry a credential removed.
---
--- `raw` exists so a caller can read metadata this module does not map, and it
--- is handed back whole. On the connect path that whole is a response to a body
--- containing the mailbox password, so the transport blocks come off it here
--- rather than being trusted not to appear.
function M.without_transport(raw)
  if type(raw) ~= "table" then return raw end
  local out = {}
  for k, v in pairs(raw) do
    if not TRANSPORT_KEYS[k] then out[k] = v end
  end
  return out
end

--- A heat score outside 0..100, or not a number at all, is not a reading.
function M.heat(raw)
  if type(raw) ~= "number" or raw ~= raw then return nil end
  local n = math.floor(raw + 0.5)
  if n < 0 or n > 100 then return nil end
  return n
end

--- Build a client. The key, the workspace and the account are the caller's to
--- supply — this module reads no secret store. The `*_url` options exist so a
--- test can stand a server in front of each of the three endpoints.
function M.client(opts)
  opts = opts or {}
  local api_key = opts.api_key or env.get("SALESFORGE_API_KEY")
  if not api_key or trim(api_key) == "" then
    error("salesforge: api key required (opts.api_key or SALESFORGE_API_KEY)")
  end
  local workspace = opts.workspace_id
  if not workspace or trim(workspace) == "" then
    error("salesforge: workspace_id required")
  end
  workspace = trim(workspace)
  local email = opts.email or env.get("SALESFORGE_EMAIL")
  local password = opts.password or env.get("SALESFORGE_PASSWORD")
  local base_url = (opts.base_url or PUBLIC_BASE):gsub("/+$", "")
  local internal_base = (opts.internal_base_url or INTERNAL_BASE):gsub("/+$", "")
  local identity_url = opts.identity_url or IDENTITY_URL

  local token

  local function refused(where, status)
    if status == 401 or status == 403 then
      return fail("auth", status, where .. " rejected the credentials (HTTP " .. status .. ")")
    end
    if status == 429 then
      return fail("rate_limit", 429, where .. " rate limited (HTTP 429)")
    end
    -- The public API is Growth-plan-only; a 402 is that plan gate rather than a
    -- malformed request, and reads as itself.
    if status == 402 then
      return fail("plan", 402, where .. " needs a Growth plan (HTTP 402)")
    end
    if status >= 500 then
      return fail("server", status, where .. " HTTP " .. status)
    end
    return fail("http", status, where .. " HTTP " .. status)
  end

  local function send(method, target, headers, body)
    local ok, resp
    if method == "GET" then
      ok, resp = pcall(http.get, target, { headers = headers })
    else
      ok, resp = pcall(http[method:lower()], target, body and json.encode(body) or "", { headers = headers })
    end
    if not ok then return nil, tostring(resp) end
    return resp
  end

  -- The key goes in `Authorization` bare. It is an apiKey scheme, not a bearer
  -- one, and a "Bearer " prefix is refused — which reads as an invalid key
  -- rather than as a malformed request, and is a slow thing to debug.
  local function public_headers()
    return {
      Authorization = api_key,
      Accept = "application/json",
      ["Content-Type"] = "application/json",
      ["User-Agent"] = BROWSER_UA,
    }
  end

  -- The internal API takes the Firebase id token as a bearer, which is the
  -- opposite of the public one's bare apiKey. Both internal callers build the
  -- same header set, and only after `sign_in` has filled `token`.
  local function internal_headers()
    return {
      Authorization = "Bearer " .. token,
      Accept = "application/json",
      ["Content-Type"] = "application/json",
      ["User-Agent"] = BROWSER_UA,
    }
  end

  --- One call, parsed only when there is a body: several of these endpoints
  --- answer 204 with nothing at all, and a blanket parse turns every success
  --- into a read error.
  local function request(method, path, body, headers)
    local where = method .. " " .. path
    local target = (path:sub(1, 4) == "http" and path or base_url .. path)
    local resp, transport = send(method, target, headers or public_headers(), body)
    if not resp then return fail("transport", nil, where .. ": " .. transport) end
    if resp.status < 200 or resp.status >= 300 then return refused(where, resp.status) end
    local text = trim(resp.body)
    if text == "" then return true end
    local ok, parsed = pcall(json.parse, text)
    if not ok then
      return fail("unreadable", resp.status, where .. " answered with a body that is not JSON")
    end
    return parsed
  end

  --- Every row across pages, with the walk's own account of itself.
  ---
  --- An empty list arrives as a JSON object rather than an array — a workspace
  --- with no sequences answers `{"data": {}, "total": 0}` — so a `data` that is
  --- not a list is an empty page and never a read error.
  ---
  --- `meta.truncated` means the page cap stopped the walk rather than the
  --- vendor running out of rows. A caller that ignores it reads a capped list
  --- as the whole workspace.
  local function all(path, map)
    local out = {}
    local seen = 0
    local truncated = true
    for page = 0, MAX_PAGES - 1 do
      local sep = path:find("?", 1, true) and "&" or "?"
      local body, err = request("GET", path .. sep .. "limit=" .. PAGE .. "&offset=" .. (page * PAGE))
      if not body then return nil, err end
      if type(body) ~= "table" then truncated = false break end
      local rows = type(body.data) == "table" and body.data or {}
      local on_page = 0
      for _, raw in ipairs(rows) do
        on_page = on_page + 1
        local row = map and map(raw) or raw
        if row then out[#out + 1] = row end
      end
      seen = seen + on_page
      local total = body.total
      if on_page == 0 or on_page < PAGE then truncated = false break end
      if type(total) == "number" and (page + 1) * PAGE >= total then truncated = false break end
    end
    return out, { truncated = truncated, cap = MAX_PAGES * PAGE, seen = seen }
  end

  local c = {}

  function c:workspaces() return all("/workspaces") end

  function c:mailboxes() return all("/workspaces/" .. workspace .. "/mailboxes", M.map_box) end

  function c:sequences() return all("/workspaces/" .. workspace .. "/sequences") end

  function c:sequence(id)
    if not id or trim(id) == "" then return fail("config", nil, "sequence id required") end
    return request("GET", "/workspaces/" .. workspace .. "/sequences/" .. trim(id))
  end

  function c:create_contact(fields)
    if type(fields) ~= "table" or trim(fields.firstName) == "" then
      return fail("config", nil, "create_contact needs at least firstName")
    end
    return request("POST", "/workspaces/" .. workspace .. "/contacts", fields)
  end

  -- An empty list would encode as a JSON object rather than an empty array and
  -- reach the vendor as a malformed body, so it is refused here instead.
  function c:enrol(sequence_id, contact_ids)
    if not sequence_id or trim(sequence_id) == "" then
      return fail("config", nil, "enrol needs a sequence id")
    end
    if type(contact_ids) ~= "table" or #contact_ids == 0 then
      return fail("config", nil, "enrol needs at least one contact id")
    end
    return request("PUT", "/workspaces/" .. workspace .. "/sequences/" .. trim(sequence_id) .. "/contacts",
      { contactIds = contact_ids })
  end

  function c:dnc(addresses)
    if type(addresses) ~= "table" or #addresses == 0 then
      return fail("config", nil, "dnc needs at least one address")
    end
    return request("POST", "/workspaces/" .. workspace .. "/dnc/bulk", { dncs = addresses })
  end

  -- The rotation is replaced wholesale rather than added to, so a caller taking
  -- one domain out sends back the ids it means to keep. An empty list is a real
  -- instruction — a sequence whose every mailbox was pulled has none, and that
  -- is the truthful state rather than a reason to leave a stale box sending.
  --
  -- The ids are copied onto a table carrying `__jsontype = "array"` because an
  -- empty Lua table would otherwise encode as `{}`, and the vendor wants `[]`.
  -- The copy is what keeps the marker off the caller's own table.
  function c:set_rotation(sequence_id, mailbox_ids)
    if not sequence_id or trim(sequence_id) == "" then
      return fail("config", nil, "set_rotation needs a sequence id")
    end
    if type(mailbox_ids) ~= "table" then
      return fail("config", nil, "set_rotation needs a list of mailbox ids")
    end
    local ids = setmetatable({}, { __jsontype = "array" })
    for i, id in ipairs(mailbox_ids) do ids[i] = id end
    return request("PUT",
      "/workspaces/" .. workspace .. "/sequences/" .. trim(sequence_id) .. "/mailboxes",
      { mailboxIds = ids })
  end

  -- The vendor takes two statuses and answers 400 for anything else. Checking
  -- here makes a typo a config error the caller can read rather than a rejected
  -- request it has to interpret.
  function c:set_sequence_status(sequence_id, status)
    if not sequence_id or trim(sequence_id) == "" then
      return fail("config", nil, "set_sequence_status needs a sequence id")
    end
    local wanted = lower(status)
    if not SEQUENCE_STATUS[wanted] then
      return fail("config", nil,
        "sequence status must be \"paused\" or \"active\", not " .. tostring(status))
    end
    return request("PUT",
      "/workspaces/" .. workspace .. "/sequences/" .. trim(sequence_id) .. "/status",
      { status = wanted })
  end

  function c:reply(mailbox_id, email_id, body)
    if not mailbox_id or trim(mailbox_id) == "" or not email_id or trim(email_id) == "" then
      return fail("config", nil, "reply needs a mailbox id and an email id")
    end
    return request("POST",
      "/workspaces/" .. workspace .. "/mailboxes/" .. trim(mailbox_id)
      .. "/emails/" .. trim(email_id) .. "/reply",
      { content = tostring(body or ""), includeHistory = true })
  end

  --- Firebase password sign-in for the web app's own API.
  ---
  --- The token is held on the client and never returned or logged: a caller
  --- that needs the internal API calls the internal method, and one that does
  --- not never sees the credential. A failure here leaves the public API
  --- working, because the two surfaces authenticate differently.
  function c:sign_in()
    if token then return true end
    if not email or trim(email) == "" or not password or trim(password) == "" then
      return fail("sign_in", nil, "internal API needs email and password (opts or SALESFORGE_EMAIL/SALESFORGE_PASSWORD)")
    end
    local target = identity_url .. (identity_url:find("?", 1, true) and "&" or "?")
      .. "key=" .. FIREBASE_WEB_API_KEY
    local resp, transport = send("POST", target, {
      ["Content-Type"] = "application/json",
      Accept = "application/json",
      ["User-Agent"] = BROWSER_UA,
    }, { email = email, password = password, returnSecureToken = true })
    if not resp then return fail("sign_in", nil, "sign-in transport failure: " .. transport) end
    local ok, parsed = pcall(json.parse, resp.body or "")
    local id_token = ok and type(parsed) == "table" and parsed.idToken
    if type(id_token) ~= "string" or id_token == "" then
      return fail("sign_in", resp.status, "sign-in failed (HTTP " .. tostring(resp.status) .. ")")
    end
    token = id_token
    return true
  end

  --- Warm-up state, which only the web app's own API answers. It pages by a
  --- `pagination.next` link rather than by an offset, and `totalPages` is the
  --- stop condition.
  function c:mailboxes_internal()
    local signed, err = self:sign_in()
    if not signed then return nil, err end
    local headers = internal_headers()
    local out = {}
    local seen = 0
    local truncated = true
    for page = 1, MAX_PAGES do
      local target = internal_base .. "/workspaces/" .. workspace
        .. "/mailboxes?page=" .. page .. "&size=" .. INTERNAL_PAGE
      local body, call_err = request("GET", target, nil, headers)
      if not body then return nil, call_err end
      if type(body) ~= "table" then truncated = false break end
      local rows = type(body.data) == "table" and body.data or {}
      local on_page = 0
      for _, raw in ipairs(rows) do
        on_page = on_page + 1
        local row = M.map_internal_box(raw)
        if row then out[#out + 1] = row end
      end
      seen = seen + on_page
      local pagination = type(body.pagination) == "table" and body.pagination or {}
      if on_page == 0 then truncated = false break end
      if type(pagination.totalPages) == "number" and page >= pagination.totalPages then
        truncated = false
        break
      end
      if trim(pagination.next) == "" then truncated = false break end
    end
    return out, { truncated = truncated, cap = MAX_PAGES * INTERNAL_PAGE, seen = seen }
  end

  --- One mailbox as the web app's own API holds it, warm-up state and all.
  function c:mailbox_internal(id)
    if not id or trim(id) == "" then return fail("config", nil, "mailbox id required") end
    local signed, err = self:sign_in()
    if not signed then return nil, err end
    local body, call_err = request("GET",
      internal_base .. "/workspaces/" .. workspace .. "/mailboxes/" .. trim(id),
      nil, internal_headers())
    if not body then return nil, call_err end
    local raw = type(body) == "table" and (type(body.data) == "table" and body.data or body) or nil
    local row = raw and M.map_internal_box(raw) or nil
    if not row then
      return fail("unreadable", nil, "GET mailbox " .. trim(id) .. " answered no readable mailbox")
    end
    return row
  end

  --- The vendor's id behind an address; an id passes straight through.
  ---
  --- An operator holds addresses and these endpoints take ids. Resolved on the
  --- internal listing rather than the public one because both APIs answer the
  --- same box under the same id and only one of them can also say what its
  --- warm-up is doing. Anything without an `@` is already an id: the vendor's
  --- prefix has changed before and matching on one would break silently.
  function c:mailbox_id(id_or_address)
    local given = trim(id_or_address)
    if given == "" then return fail("config", nil, "mailbox id or address required") end
    if not given:find("@", 1, true) then return given end
    local wanted = lower(given)
    local rows, err = self:mailboxes_internal()
    if not rows then return nil, err end
    for _, row in ipairs(rows) do
      if row.address == wanted then return row.provider_ref end
    end
    return fail("not_found", nil, "no mailbox " .. wanted .. " in this workspace")
  end

  --- Connect a mailbox to the sequencer over SMTP and IMAP.
  ---
  --- The vendor verifies the credentials asynchronously: what comes back is
  --- `pending`, and it becomes `active` — or `failed` — seconds later. So this
  --- returns what the vendor said at the moment it said it, and `connected` is
  --- true only where the vendor already said `active`. A caller that needs the
  --- verdict reads the box again rather than being told a verification that has
  --- not happened yet succeeded.
  ---
  --- Warm-up is NOT switched on here, even though the vendor documents that a
  --- connected box warms automatically. A box created through this API arrives
  --- with `warmupActivated` false — that is what `set_warmup` is for, and the
  --- two are separate calls because they are separate APIs and either can fail
  --- while the other stands.
  function c:connect_smtp(address, password, opts)
    opts = opts or {}
    local addr = lower(address)
    if addr == "" or not addr:find("@", 1, true) then
      return fail("config", nil, "connect_smtp needs an email address")
    end
    if not password or trim(password) == "" then
      return fail("config", nil, "connect_smtp needs the mailbox password")
    end
    local smtp = opts.smtp or DEFAULT_SMTP
    local imap = opts.imap or DEFAULT_IMAP
    -- The vendor requires a first name and rejects a body without one. The
    -- local part is a poor name and a better one than a refused request.
    local first = trim(opts.first)
    local body = {
      firstName = first ~= "" and first or addr:match("^([^@]+)"),
      lastName = trim(opts.last),
      address = addr,
      smtp = {
        host = smtp.host, port = smtp.port,
        username = smtp.username or addr, password = password,
      },
      imap = {
        host = imap.host, port = imap.port,
        username = imap.username or addr, password = password,
      },
    }
    if type(opts.daily_limit) == "number" then body.dailyEmailLimit = opts.daily_limit end
    local created, err = request("POST", "/workspaces/" .. workspace .. "/mailboxes", body)
    if not created then return nil, err end
    local row = type(created) == "table" and M.map_box(created) or nil
    if not row then
      -- The vendor answers 2xx with its own refusal in the body when the
      -- credentials do not verify. Read as a created mailbox that is a box
      -- nothing can send from, reported as connected.
      local said = type(created) == "table" and type(created.message) == "string"
        and created.message or "no address in the answer"
      return fail("refused", nil, "connect refused: " .. said)
    end
    -- `connected` on a listed box means the workspace holds it. This one has
    -- not been verified yet, so it says what the vendor said and nothing more.
    row.connected = row.status == "active"
    -- The password this call just sent must not come back out of it. The
    -- vendor does not echo the transport blocks today; a row that carried them
    -- would put a plaintext credential into every caller that prints `raw`.
    row.raw = M.without_transport(created)
    return row
  end

  --- Switch warm-up on or off, and read the vendor's answer back.
  ---
  --- The read-back is the point. A box created through the public API arrives
  --- with warm-up off despite the vendor's own docs, and a PUT that answers 200
  --- while the flag stays false is exactly the failure this exists to catch —
  --- seventeen boxes sat cold for two hours behind one. So the switch is set,
  --- the box is read AGAIN, and what comes back is what the vendor now holds
  --- rather than what it was asked for.
  function c:set_warmup(id_or_address, on)
    if type(on) ~= "boolean" then
      return fail("config", nil, "set_warmup needs true or false")
    end
    local id, err = self:mailbox_id(id_or_address)
    if not id then return nil, err end
    local signed, sign_err = self:sign_in()
    if not signed then return nil, sign_err end
    local put, put_err = request("PUT",
      internal_base .. "/workspaces/" .. workspace .. "/mailboxes/" .. id,
      { warmupActivated = on }, internal_headers())
    if not put then return nil, put_err end
    return self:mailbox_internal(id)
  end

  --- The billing cycle the vendor states, in whichever field it states it.
  ---
  --- Salesforge writes it on the plan on some accounts and on the account on
  --- others, and on a trial it writes it nowhere. Nothing is nothing: a plan
  --- with no stated cycle gets no period, because "month" here would be this
  --- module's guess presented as the vendor's answer.
  local function plan_period(account, plan)
    local sources = { plan.interval, plan.billingPeriod, account.billingCycle }
    for i = 1, 3 do
      local mapped = PLAN_PERIOD[trim(sources[i]):upper()]
      if mapped then return mapped end
    end
    return nil
  end

  --- The plan the account is on, and what it entitles.
  ---
  --- Only the web app's own `/me` carries any of this. The public API answers a
  --- workspace with a name, an id and nothing else, and every plan, billing,
  --- usage and limits path under it is a flat 404. The internal
  --- `/workspaces/{id}/subscription` route does exist — it answers "growth
  --- subscription not found" rather than the generic "Not Found" — but it holds
  --- nothing for an account that has never bought one.
  ---
  --- The vendor names no money anywhere on any of it: no amount, no currency,
  --- no price on the plan it says you are on. So the plan item carries no
  --- `unit_price_cents` and `meta.priced` is false outright. A caller that read
  --- the absent price as free would put the sequencer's cost at nothing.
  function c:costs()
    local signed, err = self:sign_in()
    if not signed then return nil, err end
    local body, call_err = request("GET", internal_base .. "/me", nil, internal_headers())
    if call_err then return nil, call_err end
    -- A 204, an empty body, a JSON scalar and an array all reach here as
    -- something that is not an account. Indexed, they crash; read as an empty
    -- account they would report a workspace entitled to nothing, which is a
    -- plan downgrade that never happened. An account whose `activePlan` is
    -- missing is a different thing — the account is real and names its plan by
    -- id — so the line is drawn at the account, not at the plan.
    local user = type(body) == "table" and type(body.user) == "table" and body.user or nil
    local account = user and type(user.account) == "table" and user.account or nil
    if not account then
      return fail("unreadable", nil, "GET /me answered without an account object")
    end
    local plan = type(account.activePlan) == "table" and account.activePlan or {}

    local limits, credits = {}, {}
    for _, entry in ipairs(PLAN_LIMITS) do
      if type(plan[entry.field]) == "number" then limits[entry.name] = plan[entry.field] end
    end
    for _, entry in ipairs(PLAN_CREDITS) do
      if type(account[entry.field]) == "number" then credits[entry.name] = account[entry.field] end
    end

    return {
      items = { cost.item({
        kind = "plan",
        unit = "plan",
        ref = plan.name or account.activePlanId,
        quantity = 1,
        period = plan_period(account, plan),
      }) },
      meta = {
        -- No amount and no currency on any field of any of it.
        priced = false,
        currency_known = false,
        plan = {
          id = account.activePlanId,
          name = plan.name,
          status = account.subscriptionStatus,
          started_at = account.planStartedAt,
          trial_expires_at = account.freeTrialExpiresAt,
        },
        -- The ceilings are stated per month by the vendor's own field names,
        -- whatever cycle the plan bills on.
        limits = limits,
        credits_left = credits,
      },
    }
  end

  return c
end

return M
