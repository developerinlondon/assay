--- @module assay.prometheus
--- @description Prometheus monitoring queries. PromQL instant/range queries, alerts, targets, rules, series.
--- @keywords prometheus, promql, metrics, alerts, targets, rules, monitoring, instant-query, range-query, scrape, metadata, reload, observability, metric
--- @quickref M.query(url, promql) -> number|[{metric, value}] | Instant PromQL query
--- @quickref M.query_range(url, promql, start, end, step) -> [result] | Range PromQL query
--- @quickref M.alerts(url) -> [alert] | List active alerts
--- @quickref M.targets(url) -> {activeTargets, droppedTargets} | List scrape targets
--- @quickref M.rules(url, opts?) -> [group] | List alerting/recording rules
--- @quickref M.label_values(url, label_name) -> [string] | List values for a label
--- @quickref M.series(url, match_selectors) -> [series] | Query series metadata
--- @quickref M.config_reload(url) -> bool | Trigger configuration reload
--- @quickref M.targets_metadata(url, opts?) -> [metadata] | Get targets metadata

local M = {}

function M.query(url, promql)
  local query_url = url:gsub("/+$", "") .. "/api/v1/query"
  local resp = http.get(query_url, { headers = {} })

  if resp.status ~= 200 then
    error("prometheus.query: HTTP " .. resp.status .. ": " .. resp.body)
  end

  local data = json.parse(resp.body)

  if data.status ~= "success" then
    error("prometheus.query: " .. (data.error or "unknown error"))
  end

  if not data.data or not data.data.result then
    error("prometheus.query: unexpected response format")
  end

  local results = data.data.result

  if #results == 1 then
    local val_str = results[1].value[2]
    local num = tonumber(val_str)
    if num then
      return num
    end
    return val_str
  end

  local out = {}
  for i, result in ipairs(results) do
    out[i] = {
      metric = result.metric,
      value = tonumber(result.value[2]) or result.value[2],
    }
  end
  return out
end

function M.query_range(url, promql, start_time, end_time, step)
  local query_url = url:gsub("/+$", "") .. "/api/v1/query_range"

  local params = "query=" .. promql
  if start_time then params = params .. "&start=" .. start_time end
  if end_time then params = params .. "&end=" .. end_time end
  if step then params = params .. "&step=" .. step end

  local resp = http.get(query_url .. "?" .. params, { headers = {} })

  if resp.status ~= 200 then
    error("prometheus.query_range: HTTP " .. resp.status .. ": " .. resp.body)
  end

  local data = json.parse(resp.body)

  if data.status ~= "success" then
    error("prometheus.query_range: " .. (data.error or "unknown error"))
  end

  return data.data.result
end

function M.alerts(url)
  local api_url = url:gsub("/+$", "") .. "/api/v1/alerts"
  local resp = http.get(api_url, { headers = {} })

  if resp.status ~= 200 then
    error("prometheus.alerts: HTTP " .. resp.status .. ": " .. resp.body)
  end

  local data = json.parse(resp.body)

  if data.status ~= "success" then
    error("prometheus.alerts: " .. (data.error or "unknown error"))
  end

  return data.data.alerts
end

function M.targets(url)
  local api_url = url:gsub("/+$", "") .. "/api/v1/targets"
  local resp = http.get(api_url, { headers = {} })

  if resp.status ~= 200 then
    error("prometheus.targets: HTTP " .. resp.status .. ": " .. resp.body)
  end

  local data = json.parse(resp.body)

  if data.status ~= "success" then
    error("prometheus.targets: " .. (data.error or "unknown error"))
  end

  return data.data
end

function M.rules(url, opts)
  local api_url = url:gsub("/+$", "") .. "/api/v1/rules"

  if opts and opts.type then
    api_url = api_url .. "?type=" .. opts.type
  end

  local resp = http.get(api_url, { headers = {} })

  if resp.status ~= 200 then
    error("prometheus.rules: HTTP " .. resp.status .. ": " .. resp.body)
  end

  local data = json.parse(resp.body)

  if data.status ~= "success" then
    error("prometheus.rules: " .. (data.error or "unknown error"))
  end

  return data.data.groups
end

function M.label_values(url, label_name)
  local api_url = url:gsub("/+$", "") .. "/api/v1/label/" .. label_name .. "/values"
  local resp = http.get(api_url, { headers = {} })

  if resp.status ~= 200 then
    error("prometheus.label_values: HTTP " .. resp.status .. ": " .. resp.body)
  end

  local data = json.parse(resp.body)

  if data.status ~= "success" then
    error("prometheus.label_values: " .. (data.error or "unknown error"))
  end

  return data.data
end

function M.series(url, match_selectors)
  local api_url = url:gsub("/+$", "") .. "/api/v1/series"

  local params = ""
  for i, sel in ipairs(match_selectors) do
    if i > 1 then params = params .. "&" end
    params = params .. "match[]=" .. sel
  end

  local resp = http.get(api_url .. "?" .. params, { headers = {} })

  if resp.status ~= 200 then
    error("prometheus.series: HTTP " .. resp.status .. ": " .. resp.body)
  end

  local data = json.parse(resp.body)

  if data.status ~= "success" then
    error("prometheus.series: " .. (data.error or "unknown error"))
  end

  return data.data
end

function M.config_reload(url)
  local api_url = url:gsub("/+$", "") .. "/-/reload"
  local resp = http.post(api_url, "", { headers = {} })

  return resp.status == 200
end

function M.targets_metadata(url, opts)
  local api_url = url:gsub("/+$", "") .. "/api/v1/targets/metadata"

  local params = {}
  if opts then
    if opts.match_target then
      params[#params + 1] = "match_target=" .. opts.match_target
    end
    if opts.metric then
      params[#params + 1] = "metric=" .. opts.metric
    end
    if opts.limit then
      params[#params + 1] = "limit=" .. opts.limit
    end
  end

  if #params > 0 then
    api_url = api_url .. "?" .. table.concat(params, "&")
  end

  local resp = http.get(api_url, { headers = {} })

  if resp.status ~= 200 then
    error("prometheus.targets_metadata: HTTP " .. resp.status .. ": " .. resp.body)
  end

  local data = json.parse(resp.body)

  if data.status ~= "success" then
    error("prometheus.targets_metadata: " .. (data.error or "unknown error"))
  end

  return data.data
end

return M
