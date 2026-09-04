--- @module assay.salesforge
--- @description Salesforge sequencer — the public REST API for workspaces, mailboxes, sequences, contacts, do-not-contact and replies, plus the web app's own API for the warm-up state the public one does not carry. Credentials come from the caller.
--- @category saas
--- @icon send
--- @keywords salesforge, sequencer, cold email, sequence, contact, enrol, dnc, mailbox, warmup, reply
--- @quickref M.client(opts) -> c | Key via opts.api_key or SALESFORGE_API_KEY; opts.workspace_id required
--- @quickref c:workspaces() -> [workspace] | nil, err | Every workspace the key can see
--- @quickref c:mailboxes() -> [box] | nil, err | Connected mailboxes; the public API carries no warm-up state
--- @quickref c:sequences() -> [sequence] | nil, err | Every sequence in the workspace
--- @quickref c:sequence(id) -> sequence | nil, err | One sequence, with its mailbox rotation
--- @quickref c:create_contact(fields) -> contact | nil, err | firstName is required by the vendor
--- @quickref c:enrol(sequence_id, contact_ids) -> true | nil, err | Assign contacts to a sequence
--- @quickref c:dnc(addresses) -> true | nil, err | Stop writing to these addresses
--- @quickref c:reply(mailbox_id, email_id, body) -> true | nil, err | Reply on an existing thread
--- @quickref c:sign_in() -> true | nil, err | Firebase password sign-in for the internal API; memoised, token never returned
--- @quickref c:mailboxes_internal() -> [box] | nil, err | Warm-up state: warmupActivated, daysUntilWarm, heat

local M = {}

local PUBLIC_BASE = "https://api.salesforge.ai/public/v2"
local INTERNAL_BASE = "https://api.salesforge.ai"
local IDENTITY_URL = "https://identitytoolkit.googleapis.com/v1/accounts:signInWithPassword"

-- Salesforge's Firebase project key. It ships in every copy of the web app's
-- front-end bundle and identifies the project, not an account.
local FIREBASE_WEB_API_KEY = "AIzaSyCSvPu4xQeXnowWbgt2uRFGwAuMhkbJo-o"

-- Cloudflare answers a client with no browser User-Agent with error 1010.
local BROWSER_UA = "Mozilla/5.0 (X11; Linux x86_64; rv:130.0) Gecko/20100101 Firefox/130.0"

local PAGE = 100
local INTERNAL_PAGE = 50
local MAX_PAGES = 20

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

  --- Every row across pages.
  ---
  --- An empty list arrives as a JSON object rather than an array — a workspace
  --- with no sequences answers `{"data": {}, "total": 0}` — so a `data` that is
  --- not a list is an empty page and never a read error.
  local function all(path, map)
    local out = {}
    for page = 0, MAX_PAGES - 1 do
      local sep = path:find("?", 1, true) and "&" or "?"
      local body, err = request("GET", path .. sep .. "limit=" .. PAGE .. "&offset=" .. (page * PAGE))
      if not body then return nil, err end
      if type(body) ~= "table" then break end
      local rows = type(body.data) == "table" and body.data or {}
      local seen = 0
      for _, raw in ipairs(rows) do
        seen = seen + 1
        local row = map and map(raw) or raw
        if row then out[#out + 1] = row end
      end
      local total = body.total
      if seen == 0 or seen < PAGE then break end
      if type(total) == "number" and (page + 1) * PAGE >= total then break end
    end
    return out
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
    local headers = {
      Authorization = "Bearer " .. token,
      Accept = "application/json",
      ["Content-Type"] = "application/json",
      ["User-Agent"] = BROWSER_UA,
    }
    local out = {}
    for page = 1, MAX_PAGES do
      local target = internal_base .. "/workspaces/" .. workspace
        .. "/mailboxes?page=" .. page .. "&size=" .. INTERNAL_PAGE
      local body, call_err = request("GET", target, nil, headers)
      if not body then return nil, call_err end
      if type(body) ~= "table" then break end
      local rows = type(body.data) == "table" and body.data or {}
      local seen = 0
      for _, raw in ipairs(rows) do
        seen = seen + 1
        local row = M.map_internal_box(raw)
        if row then out[#out + 1] = row end
      end
      local pagination = type(body.pagination) == "table" and body.pagination or {}
      if seen == 0 then break end
      if type(pagination.totalPages) == "number" and page >= pagination.totalPages then break end
      if trim(pagination.next) == "" then break end
    end
    return out
  end

  return c
end

return M
