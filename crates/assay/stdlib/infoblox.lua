--- @module assay.infoblox
--- @description Infoblox WAPI. DNS records, networks, DHCP ranges, grid status. Basic auth.
--- @category cloud
--- @keywords infoblox, wapi, ipam, dns, dhcp, network, range, grid, records, a-record, ref
--- @quickref c.records:get(type, opts?) -> [record]|nil | Get records of a type (GET, read)
--- @quickref c.network:get(opts?) -> [network] | Get networks (GET, read)
--- @quickref c.range:get(opts?) -> [range] | Get DHCP ranges (GET, read)
--- @quickref c.grid:status() -> [grid] | Get grid status (GET, read)
--- @quickref c.records:create(type, body) -> ref | Create a record (POST, mutates)
--- @quickref c.records:update(ref, body) -> ref | Update a record by ref (PUT, mutates)
--- @quickref c.records:delete(ref) -> ref | Delete a record by ref (DELETE, mutates)

local M = {}

local function encode(value)
  local out = tostring(value):gsub("([^A-Za-z0-9%-_.~])", function(ch)
    return string.format("%%%02X", string.byte(ch))
  end)
  return out
end

local function build_query(params)
  local parts = {}
  for k, v in pairs(params) do
    if v ~= nil then
      parts[#parts + 1] = encode(k) .. "=" .. encode(v)
    end
  end
  table.sort(parts)
  if #parts == 0 then return "" end
  return "?" .. table.concat(parts, "&")
end

function M.client(url, opts)
  opts = opts or {}
  local base_url = url:gsub("/+$", "")
  local user = opts.user
  local password = opts.password
  local wapi = "/wapi/" .. (opts.wapi_version or "v2.12")

  local function headers()
    local h = { ["Content-Type"] = "application/json" }
    if user then
      h["Authorization"] = "Basic " .. base64.encode(user .. ":" .. (password or ""))
    end
    return h
  end

  local function api_get(path_str)
    local resp = http.get(base_url .. path_str, { headers = headers() })
    if resp.status == 404 then return nil end
    if resp.status ~= 200 then
      error("infoblox: GET " .. path_str .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  local function api_post(path_str, payload)
    local resp = http.post(base_url .. path_str, payload, { headers = headers() })
    if resp.status ~= 200 and resp.status ~= 201 then
      error("infoblox: POST " .. path_str .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  local function api_put(path_str, payload)
    local resp = http.put(base_url .. path_str, payload, { headers = headers() })
    if resp.status ~= 200 then
      error("infoblox: PUT " .. path_str .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  local function api_delete(path_str)
    local resp = http.delete(base_url .. path_str, { headers = headers() })
    if resp.status ~= 200 then
      error("infoblox: DELETE " .. path_str .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  -- ===== Client =====

  local c = {}

  -- ===== Records =====

  c.records = {}

  function c.records:get(record_type, get_opts)
    return api_get(wapi .. "/" .. record_type .. build_query(get_opts or {}))
  end

  function c.records:create(record_type, body)
    return api_post(wapi .. "/" .. record_type, body)
  end

  function c.records:update(ref, body)
    return api_put(wapi .. "/" .. ref, body)
  end

  function c.records:delete(ref)
    return api_delete(wapi .. "/" .. ref)
  end

  -- ===== Networks =====

  c.network = {}

  function c.network:get(get_opts)
    return api_get(wapi .. "/network" .. build_query(get_opts or {}))
  end

  -- ===== Ranges =====

  c.range = {}

  function c.range:get(get_opts)
    return api_get(wapi .. "/range" .. build_query(get_opts or {}))
  end

  -- ===== Grid =====

  c.grid = {}

  function c.grid:status()
    return api_get(wapi .. "/grid")
  end

  return c
end

return M
