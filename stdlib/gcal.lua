--- @module assay.gcal
--- @description Google Calendar REST API client with OAuth2 token refresh. Events CRUD, calendar list.
--- @keywords google, calendar, gcal, events, oauth2, schedule, meeting, create, update, delete
--- @quickref c.events:list(opts?) -> [event] | List calendar events
--- @quickref c.events:get(event_id) -> event | Get event by ID
--- @quickref c.events:create(event) -> event | Create a calendar event
--- @quickref c.events:update(event_id, event) -> event | Update an event
--- @quickref c.events:delete(event_id) -> true | Delete an event
--- @quickref c.calendars:list() -> [calendar] | List all calendars

local M = {}
local oauth2 = require("assay.oauth2")

local GCAL_API = "https://www.googleapis.com"
local TOKEN_URL = "https://oauth2.googleapis.com/token"

function M.client(opts)
  opts = opts or {}

  local credentials_file = opts.credentials_file
  local token_file = opts.token_file
  local token_url = opts.token_url or TOKEN_URL

  if not credentials_file then
    error("gcal: credentials_file is required")
  end
  if not token_file then
    error("gcal: token_file is required")
  end

  local auth = oauth2.from_file(credentials_file, token_file, {
    token_url = token_url,
  })

  local api_base = opts.api_base or GCAL_API

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
      refresh_auth()
      resp = http.get(api_base .. path_str, { headers = get_headers() })
    end
    if resp.status == 404 then return nil end
    if resp.status ~= 200 then
      error("gcal: GET " .. path_str .. " HTTP " .. resp.status .. ": " .. resp.body)
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
      error("gcal: POST " .. path_str .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  local function api_put(path_str, payload)
    local resp = http.put(api_base .. path_str, payload, { headers = get_headers() })
    if resp.status == 401 then
      refresh_auth()
      resp = http.put(api_base .. path_str, payload, { headers = get_headers() })
    end
    if resp.status ~= 200 then
      error("gcal: PUT " .. path_str .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  local function api_delete(path_str)
    local resp = http.delete(api_base .. path_str, { headers = get_headers() })
    if resp.status == 401 then
      refresh_auth()
      resp = http.delete(api_base .. path_str, { headers = get_headers() })
    end
    if resp.status ~= 200 and resp.status ~= 204 then
      error("gcal: DELETE " .. path_str .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    return true
  end

  -- ===== Client =====

  local c = {}

  -- ===== Events =====

  c.events = {}

  function c.events:list(events_opts)
    events_opts = events_opts or {}
    local params = {}
    if events_opts.timeMin then params[#params + 1] = "timeMin=" .. events_opts.timeMin end
    if events_opts.timeMax then params[#params + 1] = "timeMax=" .. events_opts.timeMax end
    if events_opts.maxResults then params[#params + 1] = "maxResults=" .. events_opts.maxResults end
    if events_opts.q then params[#params + 1] = "q=" .. events_opts.q end
    local qs = ""
    if #params > 0 then qs = "?" .. table.concat(params, "&") end
    local result = api_get("/calendar/v3/calendars/primary/events" .. qs)
    if result and result.items then
      return result.items
    end
    return {}
  end

  function c.events:get(event_id)
    return api_get("/calendar/v3/calendars/primary/events/" .. event_id)
  end

  function c.events:create(event)
    return api_post("/calendar/v3/calendars/primary/events", event)
  end

  function c.events:update(event_id, event)
    return api_put("/calendar/v3/calendars/primary/events/" .. event_id, event)
  end

  function c.events:delete(event_id)
    return api_delete("/calendar/v3/calendars/primary/events/" .. event_id)
  end

  -- ===== Calendars =====

  c.calendars = {}

  function c.calendars:list()
    local result = api_get("/calendar/v3/users/me/calendarList")
    if result and result.items then
      return result.items
    end
    return {}
  end

  return c
end

return M
