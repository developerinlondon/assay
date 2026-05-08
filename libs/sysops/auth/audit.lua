--! sysops.auth.audit - audit log SDK for assay-engine auth admin.
--!
--! Wraps: GET /api/v1/engine/auth/admin/audit

local M = {}

local function encode_segment(value)
  value = tostring(value or "")
  return (value:gsub("([^%w%-%._~])", function(ch)
    return string.format("%%%02X", string.byte(ch))
  end))
end

local function ok2xx(status)
  return type(status) == "number" and status >= 200 and status < 300
end

local function result(resp)
  if not resp or not ok2xx(resp.status) then
    return nil, { status = (resp and resp.status) or 0, body = resp and resp.body }
  end
  return resp.body, nil
end

local BASE = "/api/v1/engine/auth/admin/audit"

local function build_query(opts)
  if not opts or next(opts) == nil then return "" end
  local parts = {}
  for _, k in ipairs({ "limit", "offset", "since", "kind" }) do
    if opts[k] ~= nil then
      table.insert(parts, encode_segment(k) .. "=" .. encode_segment(tostring(opts[k])))
    end
  end
  if #parts == 0 then return "" end
  return "?" .. table.concat(parts, "&")
end

function M.new(engine)
  local self = {}

  function self.list(opts)
    local resp = engine.get(BASE .. build_query(opts))
    return result(resp)
  end

  return self
end

return M
