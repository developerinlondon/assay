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

return M
