--! Stub `secret` service — in-memory keystore.

local M = {}

local store = {}

function M.read(scope, key)
  return (store[scope] and store[scope][key]) or nil
end

function M.write(scope, key, value)
  store[scope] = store[scope] or {}
  store[scope][key] = value
  return true
end

function M.delete(scope, key)
  if store[scope] then store[scope][key] = nil end
  return true
end

function M.available() return true end

return M
