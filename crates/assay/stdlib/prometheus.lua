--- @module assay.prometheus
--- @description Prometheus monitoring queries. PromQL instant/range queries, alerts, targets, rules, series.
--- @keywords prometheus, promql, metrics, alerts, targets, rules, monitoring, instant-query, range-query, scrape, metadata, reload, observability, metric
--- @quickref c.queries:instant(promql) -> number|[{metric, value}] | Instant PromQL query
--- @quickref c.queries:range(promql, start, end, step) -> [result] | Range PromQL query
--- @quickref c.alerts:list() -> [alert] | List active alerts
--- @quickref c.targets:list() -> {activeTargets, droppedTargets} | List scrape targets
--- @quickref c.targets:metadata(opts?) -> [metadata] | Get targets metadata
--- @quickref c.rules:list(opts?) -> [group] | List alerting/recording rules
--- @quickref c.labels:values(label_name) -> [string] | List values for a label
--- @quickref c.series:list(match_selectors) -> [series] | Query series metadata
--- @quickref c.config:reload() -> bool | Trigger configuration reload

local M = {}

function M.client(url)
  local base_url = url:gsub("/+$", "")

  local function api_get(path_str)
    local resp = http.get(base_url .. path_str, { headers = {} })
    if resp.status ~= 200 then
      error("prometheus: GET " .. path_str .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    local data = json.parse(resp.body)
    if data.status ~= "success" then
      error("prometheus: " .. (data.error or "unknown error"))
    end
    return data
  end

  local c = {}

  -- ===== Queries =====

  c.queries = {}

  function c.queries:instant(promql)
    local data = api_get("/api/v1/query")
    if not data.data or not data.data.result then
      error("prometheus: unexpected response format")
    end
    local results = data.data.result
    if #results == 1 then
      local val_str = results[1].value[2]
      local num = tonumber(val_str)
      if num then return num end
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

  function c.queries:range(promql, start_time, end_time, step)
    local api_path = "/api/v1/query_range"
    local params = "query=" .. promql
    if start_time then params = params .. "&start=" .. start_time end
    if end_time then params = params .. "&end=" .. end_time end
    if step then params = params .. "&step=" .. step end
    local data = api_get(api_path .. "?" .. params)
    return data.data.result
  end

  -- ===== Alerts =====

  c.alerts = {}

  function c.alerts:list()
    local data = api_get("/api/v1/alerts")
    return data.data.alerts
  end

  -- ===== Targets =====

  c.targets = {}

  function c.targets:list()
    local data = api_get("/api/v1/targets")
    return data.data
  end

  function c.targets:metadata(opts)
    local api_path = "/api/v1/targets/metadata"
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
      api_path = api_path .. "?" .. table.concat(params, "&")
    end
    local data = api_get(api_path)
    return data.data
  end

  -- ===== Rules =====

  c.rules = {}

  function c.rules:list(opts)
    local api_path = "/api/v1/rules"
    if opts and opts.type then
      api_path = api_path .. "?type=" .. opts.type
    end
    local data = api_get(api_path)
    return data.data.groups
  end

  -- ===== Labels =====

  c.labels = {}

  function c.labels:values(label_name)
    local data = api_get("/api/v1/label/" .. label_name .. "/values")
    return data.data
  end

  -- ===== Series =====

  c.series = {}

  function c.series:list(match_selectors)
    local params = ""
    for i, sel in ipairs(match_selectors) do
      if i > 1 then params = params .. "&" end
      params = params .. "match[]=" .. sel
    end
    local data = api_get("/api/v1/series?" .. params)
    return data.data
  end

  -- ===== Config =====

  c.config = {}

  function c.config:reload()
    local resp = http.post(base_url .. "/-/reload", "", { headers = {} })
    return resp.status == 200
  end

  return c
end

return M
