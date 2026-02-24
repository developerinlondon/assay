--- @module assay.healthcheck
--- @description HTTP health checking utilities. Status codes, JSON path, body matching, latency, multi-check.
--- @keywords healthcheck, http, health, status, latency, monitoring, json-path, body-match, multi-check, wait, endpoint, probe
--- @quickref M.http(url, opts?) -> {ok, status, latency_ms} | HTTP health check with status assertion
--- @quickref M.json_path(url, path_expr, expected, opts?) -> {ok, actual, expected} | Check JSON path value
--- @quickref M.status_code(url, expected, opts?) -> {ok, status} | Check HTTP status code
--- @quickref M.body_contains(url, pattern, opts?) -> {ok, found} | Check if body contains pattern
--- @quickref M.endpoint(url, opts?) -> {ok, status, latency_ms} | Check endpoint status and latency
--- @quickref M.multi(checks) -> {ok, results, passed, failed, total} | Run multiple checks
--- @quickref M.wait(url, opts?) -> {ok, status, attempts} | Wait for endpoint to become healthy

local M = {}

local function split_path(path_expr)
  local parts = {}
  for part in path_expr:gmatch("[^%.]+") do
    parts[#parts + 1] = part
  end
  return parts
end

local function traverse(tbl, path_expr)
  local parts = split_path(path_expr)
  local current = tbl
  for _, key in ipairs(parts) do
    if type(current) ~= "table" then
      return nil, "path traversal failed at '" .. key .. "': not a table"
    end
    current = current[key]
    if current == nil then
      return nil, "path '" .. path_expr .. "': key '" .. key .. "' not found"
    end
  end
  return current, nil
end

function M.http(url, opts)
  opts = opts or {}
  local expected_status = opts.expected_status or 200
  local method_name = (opts.method or "GET"):upper()

  local before = time()
  local ok, resp = pcall(function()
    if method_name == "POST" then
      return http.post(url, opts.body or "", { headers = opts.headers or {} })
    elseif method_name == "PUT" then
      return http.put(url, opts.body or "", { headers = opts.headers or {} })
    end
    return http.get(url, { headers = opts.headers or {} })
  end)
  local after = time()
  local latency_ms = math.floor((after - before) * 1000)

  if not ok then
    return { ok = false, status = 0, latency_ms = latency_ms, error = tostring(resp) }
  end

  return {
    ok = resp.status == expected_status,
    status = resp.status,
    latency_ms = latency_ms,
    error = resp.status ~= expected_status
      and ("expected status " .. expected_status .. ", got " .. resp.status)
      or nil,
  }
end

function M.json_path(url, path_expr, expected, opts)
  opts = opts or {}

  local ok, resp = pcall(http.get, url, { headers = opts.headers or {} })
  if not ok then
    return { ok = false, actual = nil, expected = expected, error = tostring(resp) }
  end

  if resp.status ~= 200 then
    return {
      ok = false,
      actual = nil,
      expected = expected,
      error = "HTTP " .. resp.status,
    }
  end

  local parse_ok, body = pcall(json.parse, resp.body)
  if not parse_ok then
    return { ok = false, actual = nil, expected = expected, error = "JSON parse error: " .. tostring(body) }
  end

  local value, err = traverse(body, path_expr)
  if err then
    return { ok = false, actual = nil, expected = expected, error = err }
  end

  return {
    ok = value == expected,
    actual = value,
    expected = expected,
    error = value ~= expected
      and ("expected " .. tostring(expected) .. ", got " .. tostring(value))
      or nil,
  }
end

function M.status_code(url, expected, opts)
  opts = opts or {}

  local ok, resp = pcall(http.get, url, { headers = opts.headers or {} })
  if not ok then
    return { ok = false, status = 0, error = tostring(resp) }
  end

  return {
    ok = resp.status == expected,
    status = resp.status,
    error = resp.status ~= expected
      and ("expected status " .. expected .. ", got " .. resp.status)
      or nil,
  }
end

function M.body_contains(url, pattern, opts)
  opts = opts or {}

  local ok, resp = pcall(http.get, url, { headers = opts.headers or {} })
  if not ok then
    return { ok = false, found = false, error = tostring(resp) }
  end

  local found = resp.body:find(pattern, 1, true) ~= nil
  return {
    ok = found,
    found = found,
    error = not found and ("pattern not found in response body") or nil,
  }
end

function M.endpoint(url, opts)
  opts = opts or {}
  local max_latency_ms = opts.max_latency_ms or 5000
  local expected_status = opts.expected_status or 200

  local before = time()
  local ok, resp = pcall(http.get, url, { headers = opts.headers or {} })
  local after = time()
  local latency_ms = math.floor((after - before) * 1000)

  if not ok then
    return { ok = false, status = 0, latency_ms = latency_ms, error = tostring(resp) }
  end

  local status_ok = resp.status == expected_status
  local latency_ok = latency_ms <= max_latency_ms

  return {
    ok = status_ok and latency_ok,
    status = resp.status,
    latency_ms = latency_ms,
    error = (not status_ok and ("expected status " .. expected_status .. ", got " .. resp.status) or nil)
      or (not latency_ok and ("latency " .. latency_ms .. "ms exceeds threshold " .. max_latency_ms .. "ms") or nil),
  }
end

function M.multi(checks)
  local results = {}
  local passed = 0
  local failed = 0

  for _, check in ipairs(checks) do
    local ok, result = pcall(check.check)
    if not ok then
      result = { ok = false, error = tostring(result) }
    end
    local entry = { name = check.name, ok = result.ok }
    for k, v in pairs(result) do
      if k ~= "ok" then
        entry[k] = v
      end
    end
    results[#results + 1] = entry
    if result.ok then
      passed = passed + 1
    else
      failed = failed + 1
    end
  end

  return {
    ok = failed == 0,
    results = results,
    passed = passed,
    failed = failed,
    total = passed + failed,
  }
end

function M.wait(url, opts)
  opts = opts or {}
  local timeout = opts.timeout or 60
  local interval = opts.interval or 2
  local expect_status = opts.expect_status or 200
  local max_attempts = math.ceil(timeout / interval)

  for i = 1, max_attempts do
    local ok, resp = pcall(http.get, url, { headers = opts.headers or {} })
    if ok and resp.status == expect_status then
      log.info("Healthy: " .. url .. " (attempt " .. tostring(i) .. ")")
      return {
        ok = true,
        status = resp.status,
        attempts = i,
      }
    end
    if i < max_attempts then
      sleep(interval)
    end
  end

  error("healthcheck.wait: " .. url .. " not healthy after " .. tostring(timeout) .. "s")
end

return M
