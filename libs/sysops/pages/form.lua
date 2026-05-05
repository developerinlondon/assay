--- pages/form.lua
--
-- Tiny form-urlencoded body decoder for POST handlers. assay's HTTP
-- server passes the raw POST body via `req.body` (a string) and only
-- decodes the query string into `req.params`; there is no `req.form`.
-- This helper accepts a request table and returns a {key=value} table
-- decoded from the body when Content-Type is application/x-www-form-
-- urlencoded (or when the body simply looks like one).

local M = {}

local function url_decode(s)
  if not s then return "" end
  s = s:gsub("+", " ")
  s = s:gsub("%%(%x%x)", function(h) return string.char(tonumber(h, 16)) end)
  return s
end

function M.parse(req)
  local out = {}
  local body = req and req.body
  if type(body) ~= "string" or body == "" then return out end
  for pair in body:gmatch("[^&]+") do
    local k, v = pair:match("^([^=]+)=?(.*)$")
    if k and k ~= "" then
      out[url_decode(k)] = url_decode(v or "")
    end
  end
  return out
end

return M
