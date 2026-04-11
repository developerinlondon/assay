--- @module assay.gmail
--- @description Gmail REST API client with OAuth2 token refresh. Search, read, reply, send emails, manage labels.
--- @keywords gmail, email, oauth2, google, search, send, reply, labels, message, thread
--- @quickref c.messages:search(query, opts?) -> [message] | Search emails by query
--- @quickref c.messages:get(message_id, opts?) -> message | Get email message by ID
--- @quickref c.messages:reply(message_id, opts) -> message | Reply to an email
--- @quickref c.messages:send(to, subject, body) -> message | Send a new email
--- @quickref c.labels:list() -> [label] | List all labels

local M = {}
local oauth2 = require("assay.oauth2")

local GMAIL_API = "https://gmail.googleapis.com"
local TOKEN_URL = "https://oauth2.googleapis.com/token"

function M.client(opts)
  opts = opts or {}

  local credentials_file = opts.credentials_file
  local token_file = opts.token_file
  local token_url = opts.token_url or TOKEN_URL

  if not credentials_file then
    error("gmail: credentials_file is required")
  end
  if not token_file then
    error("gmail: token_file is required")
  end

  local auth = oauth2.from_file(credentials_file, token_file, {
    token_url = token_url,
  })

  local api_base = opts.api_base or GMAIL_API

  -- Shared HTTP helpers (captured by all sub-object methods as upvalues)

  local function get_headers()
    return auth:headers()
  end

  local function refresh_auth()
    auth:refresh()
    auth:save()
  end

  local function api_get(path_str)
    local resp = http.get(api_base .. path_str, { headers = get_headers() })
    if resp.status == 401 then
      -- Token expired, refresh and retry
      refresh_auth()
      resp = http.get(api_base .. path_str, { headers = get_headers() })
    end
    if resp.status == 404 then return nil end
    if resp.status ~= 200 then
      error("gmail: GET " .. path_str .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  local function api_post(path_str, payload)
    local resp = http.post(api_base .. path_str, payload, { headers = get_headers() })
    if resp.status == 401 then
      refresh_auth()
      resp = http.post(api_base .. path_str, payload, { headers = get_headers() })
    end
    if resp.status ~= 200 and resp.status ~= 201 then
      error("gmail: POST " .. path_str .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  -- ===== Client =====

  local c = {}

  -- ===== Messages =====

  c.messages = {}

  function c.messages:search(query, search_opts)
    search_opts = search_opts or {}
    local max = search_opts.max or 10
    local params = "q=" .. query .. "&maxResults=" .. max
    local list_resp = api_get("/gmail/v1/users/me/messages?" .. params)
    if not list_resp or not list_resp.messages then
      return {}
    end
    local messages = {}
    for _, msg in ipairs(list_resp.messages) do
      local full = api_get("/gmail/v1/users/me/messages/" .. msg.id .. "?format=full")
      if full then
        messages[#messages + 1] = full
      end
    end
    return messages
  end

  function c.messages:get(message_id, get_opts)
    get_opts = get_opts or {}
    local format = get_opts.format or "full"
    return api_get("/gmail/v1/users/me/messages/" .. message_id .. "?format=" .. format)
  end

  function c.messages:reply(message_id, reply_opts)
    reply_opts = reply_opts or {}
    local original = api_get("/gmail/v1/users/me/messages/" .. message_id .. "?format=full")
    if not original then
      error("gmail: message not found: " .. message_id)
    end

    -- Extract headers from original
    local from_header = ""
    local subject = ""
    local message_id_header = ""
    local references = ""
    if original.payload and original.payload.headers then
      for _, h in ipairs(original.payload.headers) do
        if h.name == "From" then from_header = h.value end
        if h.name == "Subject" then subject = h.value end
        if h.name == "Message-Id" or h.name == "Message-ID" then message_id_header = h.value end
        if h.name == "References" then references = h.value end
      end
    end

    if not subject:match("^Re:") then
      subject = "Re: " .. subject
    end

    if references ~= "" then
      references = references .. " " .. message_id_header
    else
      references = message_id_header
    end

    local body = reply_opts.body or ""
    local raw_message = "To: " .. from_header .. "\r\n"
      .. "Subject: " .. subject .. "\r\n"
      .. "In-Reply-To: " .. message_id_header .. "\r\n"
      .. "References: " .. references .. "\r\n"
      .. "Content-Type: text/plain; charset=utf-8\r\n"
      .. "\r\n"
      .. body

    local encoded = base64.encode(raw_message)
    -- URL-safe base64
    encoded = encoded:gsub("+", "-"):gsub("/", "_"):gsub("=+$", "")

    return api_post("/gmail/v1/users/me/messages/send", {
      raw = encoded,
      threadId = original.threadId,
    })
  end

  function c.messages:send(to, subject, body)
    local raw_message = "To: " .. to .. "\r\n"
      .. "Subject: " .. subject .. "\r\n"
      .. "Content-Type: text/plain; charset=utf-8\r\n"
      .. "\r\n"
      .. body

    local encoded = base64.encode(raw_message)
    encoded = encoded:gsub("+", "-"):gsub("/", "_"):gsub("=+$", "")

    return api_post("/gmail/v1/users/me/messages/send", {
      raw = encoded,
    })
  end

  -- ===== Labels =====

  c.labels = {}

  function c.labels:list()
    local result = api_get("/gmail/v1/users/me/labels")
    if result and result.labels then
      return result.labels
    end
    return {}
  end

  return c
end

return M
