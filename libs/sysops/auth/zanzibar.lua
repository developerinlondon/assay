--! sysops.auth.zanzibar - Zanzibar relation-tuple SDK for assay-engine auth admin.
--!
--! Wraps: GET    /api/v1/engine/auth/admin/zanzibar/namespaces
--!        GET    /api/v1/engine/auth/admin/zanzibar/tuples  (engine may 404/405; callers
--!               should surface a "listing not shipped" banner on those statuses)
--!        POST   /api/v1/engine/auth/admin/zanzibar/tuples  (write)
--!        DELETE /api/v1/engine/auth/admin/zanzibar/tuples  (delete)
--!        POST   /api/v1/engine/auth/admin/zanzibar/check
--!        POST   /api/v1/engine/auth/admin/zanzibar/expand

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

local BASE = "/api/v1/engine/auth/admin/zanzibar"

local function build_query(opts)
  if not opts or next(opts) == nil then return "" end
  local parts = {}
  for k, v in pairs(opts) do
    table.insert(parts, encode_segment(tostring(k)) .. "=" .. encode_segment(tostring(v)))
  end
  table.sort(parts)
  if #parts == 0 then return "" end
  return "?" .. table.concat(parts, "&")
end

function M.new(engine)
  local self = {}

  function self.namespaces()
    local resp = engine.get(BASE .. "/namespaces")
    return result(resp)
  end

  -- Engine does not yet expose GET /admin/zanzibar/tuples.
  -- Returns (nil, err) with the raw status so page handlers can
  -- render the "listing not shipped" banner on 404 or 405.
  function self.tuples(filter)
    local resp = engine.get(BASE .. "/tuples" .. build_query(filter))
    return result(resp)
  end

  function self.write_tuple(t)
    local resp = engine.post(BASE .. "/tuples", t)
    return result(resp)
  end

  function self.delete_tuple(t)
    local resp = engine.delete(BASE .. "/tuples")
    -- delete_tuple sends the tuple as a query string since DELETE bodies
    -- are not universally supported; fall back to POST body if engine changes.
    -- For now pass t as query params.
    if t then
      local parts = {}
      for k, v in pairs(t) do
        table.insert(
          parts,
          encode_segment(tostring(k)) .. "=" .. encode_segment(tostring(v))
        )
      end
      table.sort(parts)
      if #parts > 0 then
        resp = engine.delete(BASE .. "/tuples?" .. table.concat(parts, "&"))
      end
    end
    return result(resp)
  end

  function self.check(subject, relation, object)
    local resp = engine.post(BASE .. "/check", {
      subject  = subject,
      relation = relation,
      object   = object,
    })
    return result(resp)
  end

  function self.expand(object, relation)
    local resp = engine.post(BASE .. "/expand", {
      object   = object,
      relation = relation,
    })
    return result(resp)
  end

  return self
end

return M
