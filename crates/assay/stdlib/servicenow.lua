--- @module assay.servicenow
--- @description ServiceNow Table API and a small CMDB helper. List, get, create, update records; query CMDB classes.
--- @category saas
--- @keywords servicenow, snow, table-api, cmdb, itsm, incident, ci, records, now, sys_id
--- @quickref c.table:list(table, opts?) -> [record] | List rows (GET, read)
--- @quickref c.table:get(table, sys_id) -> record|nil | Get a row by sys_id (GET, read)
--- @quickref c.table:create(table, body) -> record | Create a row (POST, mutates)
--- @quickref c.table:update(table, sys_id, body) -> record | Update a row (PATCH, mutates)
--- @quickref c.cmdb:query(class, body) -> [record] | Query a CMDB class (POST by contract, read)

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
      parts[#parts + 1] = k .. "=" .. encode(v)
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
  local token = opts.token

  local function headers()
    local h = { ["Content-Type"] = "application/json", ["Accept"] = "application/json" }
    if token then
      h["Authorization"] = "Bearer " .. token
    elseif user then
      h["Authorization"] = "Basic " .. base64.encode(user .. ":" .. (password or ""))
    end
    return h
  end

  local function api_get(path_str)
    local resp = http.get(base_url .. path_str, { headers = headers() })
    if resp.status == 404 then return nil end
    if resp.status ~= 200 then
      error("servicenow: GET " .. path_str .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  local function api_post(path_str, payload)
    local resp = http.post(base_url .. path_str, payload, { headers = headers() })
    if resp.status ~= 200 and resp.status ~= 201 then
      error("servicenow: POST " .. path_str .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  local function api_patch(path_str, payload)
    local resp = http.patch(base_url .. path_str, payload, { headers = headers() })
    if resp.status ~= 200 then
      error("servicenow: PATCH " .. path_str .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  -- ===== Client =====

  local c = {}

  -- ===== Table API =====

  c.table = {}

  function c.table:list(table_name, list_opts)
    list_opts = list_opts or {}
    local fields = list_opts.fields
    if type(fields) == "table" then fields = table.concat(fields, ",") end
    local data = api_get("/api/now/table/" .. table_name .. build_query({
      sysparm_query = list_opts.query,
      sysparm_limit = list_opts.limit,
      sysparm_offset = list_opts.offset,
      sysparm_fields = fields,
      sysparm_display_value = list_opts.display_value,
    }))
    if not data then return {} end
    return data.result or {}
  end

  function c.table:get(table_name, sys_id)
    local data = api_get("/api/now/table/" .. table_name .. "/" .. sys_id)
    if not data then return nil end
    return data.result
  end

  function c.table:create(table_name, body)
    local data = api_post("/api/now/table/" .. table_name, body)
    return data.result
  end

  function c.table:update(table_name, sys_id, body)
    local data = api_patch("/api/now/table/" .. table_name .. "/" .. sys_id, body)
    return data.result
  end

  -- ===== CMDB =====

  c.cmdb = {}

  function c.cmdb:query(class, body)
    local data = api_post("/api/now/cmdb/instance/" .. class, body)
    if not data then return {} end
    return data.result or {}
  end

  return c
end

return M
