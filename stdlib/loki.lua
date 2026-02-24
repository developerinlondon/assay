--- @module assay.loki
--- @description Loki log aggregation. Push logs, query with LogQL, labels, series, tail.
--- @keywords loki, logs, logql, labels, series, monitoring, push, tail, stream, instant, range, query
--- @quickref M.selector(labels) -> string | Build LogQL stream selector from labels table
--- @quickref c:push(stream_labels, entries) -> true | Push log entries to Loki
--- @quickref c:query(logql, opts?) -> [result] | Instant LogQL query
--- @quickref c:query_range(logql, opts?) -> [result] | Range LogQL query
--- @quickref c:labels(opts?) -> [string] | List label names
--- @quickref c:label_values(label_name, opts?) -> [string] | List values for a label
--- @quickref c:series(match_selectors, opts?) -> [series] | Query series metadata
--- @quickref c:tail(logql, opts?) -> data | Tail log stream
--- @quickref c:ready() -> bool | Check Loki readiness
--- @quickref c:metrics() -> string | Get Loki metrics in Prometheus format

local M = {}

function M.selector(labels)
  local parts = {}
  for k, v in pairs(labels) do
    parts[#parts + 1] = k .. '="' .. v .. '"'
  end
  return "{" .. table.concat(parts, ",") .. "}"
end

function M.client(url)
  local c = {
    url = url:gsub("/+$", ""),
  }

  local function build_params(tbl)
    local parts = {}
    for k, v in pairs(tbl) do
      parts[#parts + 1] = k .. "=" .. v
    end
    if #parts == 0 then return "" end
    return "?" .. table.concat(parts, "&")
  end

  local function api_get(self, path, params)
    local query = ""
    if params then query = build_params(params) end
    local resp = http.get(self.url .. path .. query, { headers = {} })
    if resp.status ~= 200 then
      error("loki: GET " .. path .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  function c:push(stream_labels, entries)
    local values = {}
    for i, entry in ipairs(entries) do
      if type(entry) == "string" then
        local ts = tostring(math.floor(time() * 1e9))
        values[i] = { ts, entry }
      else
        values[i] = { tostring(entry[1]), entry[2] }
      end
    end

    local payload = {
      streams = {
        {
          stream = stream_labels,
          values = values,
        },
      },
    }

    local resp = http.post(
      self.url .. "/loki/api/v1/push",
      json.encode(payload),
      { headers = { ["Content-Type"] = "application/json" } }
    )
    if resp.status ~= 204 then
      error("loki: POST /loki/api/v1/push HTTP " .. resp.status .. ": " .. resp.body)
    end
    return true
  end

  function c:query(logql, opts)
    opts = opts or {}
    local params = { query = logql }
    if opts.limit then params.limit = opts.limit end
    if opts.time then params.time = opts.time end
    if opts.direction then params.direction = opts.direction end
    local data = api_get(self, "/loki/api/v1/query", params)
    return data.data.result
  end

  function c:query_range(logql, opts)
    opts = opts or {}
    local params = { query = logql }
    if opts.start then params.start = opts.start end
    if opts.end_time then params["end"] = opts.end_time end
    if opts.limit then params.limit = opts.limit end
    if opts.step then params.step = opts.step end
    if opts.direction then params.direction = opts.direction end
    local data = api_get(self, "/loki/api/v1/query_range", params)
    return data.data.result
  end

  function c:labels(opts)
    opts = opts or {}
    local params = {}
    if opts.start then params.start = opts.start end
    if opts.end_time then params["end"] = opts.end_time end
    local data = api_get(self, "/loki/api/v1/labels", params)
    return data.data
  end

  function c:label_values(label_name, opts)
    opts = opts or {}
    local params = {}
    if opts.start then params.start = opts.start end
    if opts.end_time then params["end"] = opts.end_time end
    local data = api_get(self, "/loki/api/v1/label/" .. label_name .. "/values", params)
    return data.data
  end

  function c:series(match_selectors, opts)
    opts = opts or {}
    local parts = {}
    for _, sel in ipairs(match_selectors) do
      parts[#parts + 1] = "match[]=" .. sel
    end
    if opts.start then parts[#parts + 1] = "start=" .. opts.start end
    if opts.end_time then parts[#parts + 1] = "end=" .. opts.end_time end
    local query = ""
    if #parts > 0 then query = "?" .. table.concat(parts, "&") end
    local resp = http.get(self.url .. "/loki/api/v1/series" .. query, { headers = {} })
    if resp.status ~= 200 then
      error("loki: GET /loki/api/v1/series HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body).data
  end

  function c:tail(logql, opts)
    opts = opts or {}
    local params = { query = logql }
    if opts.limit then params.limit = opts.limit end
    if opts.start then params.start = opts.start end
    local data = api_get(self, "/loki/api/v1/tail", params)
    return data
  end

  function c:ready()
    local resp = http.get(self.url .. "/ready", { headers = {} })
    return resp.status == 200
  end

  function c:metrics()
    local resp = http.get(self.url .. "/metrics", { headers = {} })
    if resp.status ~= 200 then
      error("loki: GET /metrics HTTP " .. resp.status .. ": " .. resp.body)
    end
    return resp.body
  end

  return c
end

return M
