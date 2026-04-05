--- @module assay.gcal
--- @description Google Calendar REST API client with OAuth2 token refresh. Events CRUD, calendar list.
--- @keywords google, calendar, gcal, events, oauth2, schedule, meeting, create, update, delete
--- @quickref c:events(opts?) -> [event] | List calendar events
--- @quickref c:event_get(event_id) -> event | Get event by ID
--- @quickref c:event_create(event) -> event | Create a calendar event
--- @quickref c:event_update(event_id, event) -> event | Update an event
--- @quickref c:event_delete(event_id) -> true | Delete an event
--- @quickref c:calendars() -> [calendar] | List all calendars

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

  local c = {
    _oauth2 = auth,
    _api_base = opts.api_base or GCAL_API,
  }

  local function headers(self)
    return self._oauth2:headers()
  end

  local function refresh_auth(self)
    self._oauth2:refresh()
    self._oauth2:save()
  end

  local function api_get(self, path_str)
    local resp = http.get(self._api_base .. path_str, { headers = headers(self) })
    if resp.status == 401 then
      refresh_auth(self)
      resp = http.get(self._api_base .. path_str, { headers = headers(self) })
    end
    if resp.status == 404 then return nil end
    if resp.status ~= 200 then
      error("gcal: GET " .. path_str .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  local function api_post(self, path_str, payload)
    local resp = http.post(self._api_base .. path_str, payload, { headers = headers(self) })
    if resp.status == 401 then
      refresh_auth(self)
      resp = http.post(self._api_base .. path_str, payload, { headers = headers(self) })
    end
    if resp.status ~= 200 and resp.status ~= 201 then
      error("gcal: POST " .. path_str .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  local function api_put(self, path_str, payload)
    local resp = http.put(self._api_base .. path_str, payload, { headers = headers(self) })
    if resp.status == 401 then
      refresh_auth(self)
      resp = http.put(self._api_base .. path_str, payload, { headers = headers(self) })
    end
    if resp.status ~= 200 then
      error("gcal: PUT " .. path_str .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  local function api_delete(self, path_str)
    local resp = http.delete(self._api_base .. path_str, { headers = headers(self) })
    if resp.status == 401 then
      refresh_auth(self)
      resp = http.delete(self._api_base .. path_str, { headers = headers(self) })
    end
    if resp.status ~= 200 and resp.status ~= 204 then
      error("gcal: DELETE " .. path_str .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    return true
  end

  function c:events(events_opts)
    events_opts = events_opts or {}
    local params = {}
    if events_opts.timeMin then params[#params + 1] = "timeMin=" .. events_opts.timeMin end
    if events_opts.timeMax then params[#params + 1] = "timeMax=" .. events_opts.timeMax end
    if events_opts.maxResults then params[#params + 1] = "maxResults=" .. events_opts.maxResults end
    if events_opts.q then params[#params + 1] = "q=" .. events_opts.q end
    local qs = ""
    if #params > 0 then qs = "?" .. table.concat(params, "&") end
    local result = api_get(self, "/calendar/v3/calendars/primary/events" .. qs)
    if result and result.items then
      return result.items
    end
    return {}
  end

  function c:event_get(event_id)
    return api_get(self, "/calendar/v3/calendars/primary/events/" .. event_id)
  end

  function c:event_create(event)
    return api_post(self, "/calendar/v3/calendars/primary/events", event)
  end

  function c:event_update(event_id, event)
    return api_put(self, "/calendar/v3/calendars/primary/events/" .. event_id, event)
  end

  function c:event_delete(event_id)
    return api_delete(self, "/calendar/v3/calendars/primary/events/" .. event_id)
  end

  function c:calendars()
    local result = api_get(self, "/calendar/v3/users/me/calendarList")
    if result and result.items then
      return result.items
    end
    return {}
  end

  return c
end

return M
