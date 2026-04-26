--- @module assay.url
--- @description Pure-Lua URL helpers. RFC 3986 percent-encoding plus form-encoded body builder.
--- @keywords url, urlencode, percent-encode, form, query, encode, decode, www-form-urlencoded
--- @quickref url.encode(s) -> string | RFC 3986 percent-encode a string (space -> %20)
--- @quickref url.encode_form(t) -> string | Build application/x-www-form-urlencoded body
--- @quickref url.decode(s) -> string | Inverse of encode (also turns '+' into space)

local M = {}

local function is_unreserved(b)
  return (b >= 0x30 and b <= 0x39) -- 0-9
      or (b >= 0x41 and b <= 0x5A) -- A-Z
      or (b >= 0x61 and b <= 0x7A) -- a-z
      or b == 0x2D -- -
      or b == 0x5F -- _
      or b == 0x2E -- .
      or b == 0x7E -- ~
end

function M.encode(s)
  if s == nil then return "" end
  s = tostring(s)
  local out = {}
  for i = 1, #s do
    local b = string.byte(s, i)
    if is_unreserved(b) then
      out[#out + 1] = string.char(b)
    else
      out[#out + 1] = string.format("%%%02X", b)
    end
  end
  return table.concat(out)
end

local function stringify(v)
  if type(v) == "boolean" then
    return v and "true" or "false"
  end
  return tostring(v)
end

function M.encode_form(t)
  if t == nil then return "" end
  if type(t) ~= "table" then
    error("url.encode_form: expected table, got " .. type(t))
  end
  local keys = {}
  for k, _ in pairs(t) do
    keys[#keys + 1] = k
  end
  table.sort(keys, function(a, b) return tostring(a) < tostring(b) end)
  local parts = {}
  for _, k in ipairs(keys) do
    parts[#parts + 1] = M.encode(stringify(k)) .. "=" .. M.encode(stringify(t[k]))
  end
  return table.concat(parts, "&")
end

function M.decode(s)
  if s == nil then return "" end
  s = tostring(s)
  s = s:gsub("+", " ")
  s = s:gsub("%%(%x%x)", function(hex)
    return string.char(tonumber(hex, 16))
  end)
  return s
end

return M
