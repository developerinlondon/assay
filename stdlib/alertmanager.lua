local M = {}

function M.alerts(url, opts)
  local api_url = url:gsub("/+$", "") .. "/api/v2/alerts"
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
    api_url = api_url .. "?" .. table.concat(params, "&")
  end

  local resp = http.get(api_url, { headers = {} })

  if resp.status ~= 200 then
    error("alertmanager.alerts: HTTP " .. resp.status .. ": " .. resp.body)
  end

  return json.parse(resp.body)
end

function M.post_alerts(url, alerts)
  local api_url = url:gsub("/+$", "") .. "/api/v2/alerts"
  local resp = http.post(api_url, alerts, {
    headers = { ["Content-Type"] = "application/json" },
  })

  if resp.status ~= 200 then
    error("alertmanager.post_alerts: HTTP " .. resp.status .. ": " .. resp.body)
  end

  return true
end

function M.alert_groups(url, opts)
  local api_url = url:gsub("/+$", "") .. "/api/v2/alerts/groups"
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
    api_url = api_url .. "?" .. table.concat(params, "&")
  end

  local resp = http.get(api_url, { headers = {} })

  if resp.status ~= 200 then
    error("alertmanager.alert_groups: HTTP " .. resp.status .. ": " .. resp.body)
  end

  return json.parse(resp.body)
end

function M.silences(url, opts)
  local api_url = url:gsub("/+$", "") .. "/api/v2/silences"
  local params = {}

  if opts then
    if opts.filter then
      for _, f in ipairs(opts.filter) do
        params[#params + 1] = "filter=" .. f
      end
    end
  end

  if #params > 0 then
    api_url = api_url .. "?" .. table.concat(params, "&")
  end

  local resp = http.get(api_url, { headers = {} })

  if resp.status ~= 200 then
    error("alertmanager.silences: HTTP " .. resp.status .. ": " .. resp.body)
  end

  return json.parse(resp.body)
end

function M.silence(url, id)
  local api_url = url:gsub("/+$", "") .. "/api/v2/silence/" .. id
  local resp = http.get(api_url, { headers = {} })

  if resp.status ~= 200 then
    error("alertmanager.silence: HTTP " .. resp.status .. ": " .. resp.body)
  end

  return json.parse(resp.body)
end

function M.create_silence(url, silence)
  local api_url = url:gsub("/+$", "") .. "/api/v2/silences"
  local resp = http.post(api_url, silence, {
    headers = { ["Content-Type"] = "application/json" },
  })

  if resp.status ~= 200 then
    error("alertmanager.create_silence: HTTP " .. resp.status .. ": " .. resp.body)
  end

  return json.parse(resp.body)
end

function M.delete_silence(url, id)
  local api_url = url:gsub("/+$", "") .. "/api/v2/silence/" .. id
  local resp = http.delete(api_url, { headers = {} })

  if resp.status ~= 200 then
    error("alertmanager.delete_silence: HTTP " .. resp.status .. ": " .. resp.body)
  end

  return true
end

function M.status(url)
  local api_url = url:gsub("/+$", "") .. "/api/v2/status"
  local resp = http.get(api_url, { headers = {} })

  if resp.status ~= 200 then
    error("alertmanager.status: HTTP " .. resp.status .. ": " .. resp.body)
  end

  return json.parse(resp.body)
end

function M.receivers(url)
  local api_url = url:gsub("/+$", "") .. "/api/v2/receivers"
  local resp = http.get(api_url, { headers = {} })

  if resp.status ~= 200 then
    error("alertmanager.receivers: HTTP " .. resp.status .. ": " .. resp.body)
  end

  return json.parse(resp.body)
end

function M.is_firing(url, alertname)
  local all = M.alerts(url, {
    active = true,
    silenced = false,
    inhibited = false,
    filter = { 'alertname="' .. alertname .. '"' },
  })

  return #all > 0
end

function M.silence_alert(url, alertname, duration_hours, opts)
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

  local result = M.create_silence(url, silence)
  return result.silenceID
end

function M.active_count(url)
  local all = M.alerts(url, {
    active = true,
    silenced = false,
    inhibited = false,
  })

  return #all
end

return M
