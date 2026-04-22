--- @module assay.alertmanager
--- @description Alertmanager alert and silence management. Query, create, and delete alerts and silences.
--- @keywords alertmanager, alerts, silences, receivers, monitoring, silence, inhibit, grouping, notification, receiver
--- @quickref c.alerts:list(opts?) -> [alert] | List active alerts with filters
--- @quickref c.alerts:post(alerts) -> true | Post new alerts
--- @quickref c.alerts:groups(opts?) -> [group] | List alert groups
--- @quickref c.alerts:is_firing(alertname) -> bool | Check if alert is firing
--- @quickref c.alerts:active_count() -> number | Count active non-silenced alerts
--- @quickref c.silences:list(opts?) -> [silence] | List silences
--- @quickref c.silences:get(id) -> silence | Get silence by ID
--- @quickref c.silences:create(silence) -> {silenceID} | Create a silence
--- @quickref c.silences:delete(id) -> true | Delete silence by ID
--- @quickref c.silences:silence_alert(alertname, duration_hours, opts?) -> silenceID | Silence an alert by name
--- @quickref c.status:get() -> {cluster, config} | Get Alertmanager status
--- @quickref c.receivers:list() -> [receiver] | List receivers

local M = {}

function M.client(url)
  local base_url = url:gsub("/+$", "")

  local function api_get(path_str)
    local resp = http.get(base_url .. path_str, { headers = {} })
    if resp.status ~= 200 then
      error("alertmanager: GET " .. path_str .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  local function api_post(path_str, payload, content_type)
    local resp = http.post(base_url .. path_str, payload, {
      headers = { ["Content-Type"] = content_type or "application/json" },
    })
    if resp.status ~= 200 then
      error("alertmanager: POST " .. path_str .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    return resp
  end

  local function api_delete(path_str)
    local resp = http.delete(base_url .. path_str, { headers = {} })
    if resp.status ~= 200 then
      error("alertmanager: DELETE " .. path_str .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    return true
  end

  local c = {}

  -- ===== Alerts =====

  c.alerts = {}

  function c.alerts:list(opts)
    local api_path = "/api/v2/alerts"
    local params = {}
    if opts then
      if opts.active ~= nil then params[#params + 1] = "active=" .. tostring(opts.active) end
      if opts.silenced ~= nil then params[#params + 1] = "silenced=" .. tostring(opts.silenced) end
      if opts.inhibited ~= nil then params[#params + 1] = "inhibited=" .. tostring(opts.inhibited) end
      if opts.unprocessed ~= nil then params[#params + 1] = "unprocessed=" .. tostring(opts.unprocessed) end
      if opts.filter then
        for _, f in ipairs(opts.filter) do
          params[#params + 1] = "filter=" .. f
        end
      end
      if opts.receiver then params[#params + 1] = "receiver=" .. opts.receiver end
    end
    if #params > 0 then
      api_path = api_path .. "?" .. table.concat(params, "&")
    end
    return api_get(api_path)
  end

  function c.alerts:post(alerts)
    api_post("/api/v2/alerts", alerts)
    return true
  end

  function c.alerts:groups(opts)
    local api_path = "/api/v2/alerts/groups"
    local params = {}
    if opts then
      if opts.active ~= nil then params[#params + 1] = "active=" .. tostring(opts.active) end
      if opts.silenced ~= nil then params[#params + 1] = "silenced=" .. tostring(opts.silenced) end
      if opts.inhibited ~= nil then params[#params + 1] = "inhibited=" .. tostring(opts.inhibited) end
      if opts.filter then
        for _, f in ipairs(opts.filter) do
          params[#params + 1] = "filter=" .. f
        end
      end
      if opts.receiver then params[#params + 1] = "receiver=" .. opts.receiver end
    end
    if #params > 0 then
      api_path = api_path .. "?" .. table.concat(params, "&")
    end
    return api_get(api_path)
  end

  function c.alerts:is_firing(alertname)
    local all = c.alerts:list({
      active = true,
      silenced = false,
      inhibited = false,
      filter = { 'alertname="' .. alertname .. '"' },
    })
    return #all > 0
  end

  function c.alerts:active_count()
    local all = c.alerts:list({
      active = true,
      silenced = false,
      inhibited = false,
    })
    return #all
  end

  -- ===== Silences =====

  c.silences = {}

  function c.silences:list(opts)
    local api_path = "/api/v2/silences"
    local params = {}
    if opts then
      if opts.filter then
        for _, f in ipairs(opts.filter) do
          params[#params + 1] = "filter=" .. f
        end
      end
    end
    if #params > 0 then
      api_path = api_path .. "?" .. table.concat(params, "&")
    end
    return api_get(api_path)
  end

  function c.silences:get(id)
    return api_get("/api/v2/silence/" .. id)
  end

  function c.silences:create(silence)
    local resp = api_post("/api/v2/silences", silence)
    return json.parse(resp.body)
  end

  function c.silences:delete(id)
    return api_delete("/api/v2/silence/" .. id)
  end

  function c.silences:silence_alert(alertname, duration_hours, opts)
    opts = opts or {}
    local now = time()
    local ends_at = now + (duration_hours * 3600)

    local silence = {
      matchers = {
        { name = "alertname", value = alertname, isRegex = false, isEqual = true },
      },
      startsAt = tostring(now),
      endsAt = tostring(ends_at),
      createdBy = opts.created_by or "assay",
      comment = opts.comment or "Silenced by assay",
    }

    local result = c.silences:create(silence)
    return result.silenceID
  end

  -- ===== Status =====

  c.status = {}

  function c.status:get()
    return api_get("/api/v2/status")
  end

  -- ===== Receivers =====

  c.receivers = {}

  function c.receivers:list()
    return api_get("/api/v2/receivers")
  end

  return c
end

return M
