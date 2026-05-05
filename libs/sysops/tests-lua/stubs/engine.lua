--! Stub `engine` HTTP wrapper. Records calls; returns a 200/empty by
--! default. Override per-test via M.respond(method, path, response).

local M = {}

local responses = {}
M.calls = {}

function M.respond(method, path, response)
  responses[method:upper() .. " " .. path] = response
end

local function do_call(method, path, body)
  table.insert(M.calls, { method = method, path = path, body = body })
  local key = method:upper() .. " " .. path
  return responses[key] or { status = 200, body = "{}" }
end

function M.get(path)        return do_call("GET",    path) end
function M.post(path, body) return do_call("POST",   path, body) end
function M.put(path, body)  return do_call("PUT",    path, body) end
function M.delete(path)     return do_call("DELETE", path) end
function M.api_call(method, path, body) return do_call(method, path, body) end

return M
