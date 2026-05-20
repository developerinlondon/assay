--! sysops.codec - tiny byte/string helpers shared across the auth
--! gateway modules (oidc, session).
--!
--! Why: assay-lua's base64.encode binding requires UTF-8 input, so raw
--! SHA-256 / HMAC bytes can't go through it. Pure-lua base64url is the
--! workaround. Hex<->bytes belongs here too since several call sites
--! need to hand off hash output.

local M = {}

local B64U_CHARS = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_"

--- Encode an arbitrary byte string as URL-safe base64 (no padding).
--- @param bytes string raw bytes (NOT UTF-8 constrained)
function M.b64url(bytes)
  local out = {}
  local i = 1
  local len = #bytes
  while i <= len do
    local b1 = string.byte(bytes, i)
    local b2 = string.byte(bytes, i + 1) or 0
    local b3 = string.byte(bytes, i + 2) or 0
    local n = b1 * 65536 + b2 * 256 + b3
    out[#out + 1] = B64U_CHARS:sub(((n // 262144) % 64) + 1, ((n // 262144) % 64) + 1)
    out[#out + 1] = B64U_CHARS:sub(((n // 4096) % 64) + 1, ((n // 4096) % 64) + 1)
    if i + 1 <= len then
      out[#out + 1] = B64U_CHARS:sub(((n // 64) % 64) + 1, ((n // 64) % 64) + 1)
    end
    if i + 2 <= len then
      out[#out + 1] = B64U_CHARS:sub((n % 64) + 1, (n % 64) + 1)
    end
    i = i + 3
  end
  return table.concat(out)
end

-- Reverse lookup table for b64url decode.
local B64U_REV = {}
for i = 1, #B64U_CHARS do
  B64U_REV[B64U_CHARS:sub(i, i)] = i - 1
end

--- Decode URL-safe base64 (no-padding) back to a byte string.
function M.b64url_decode(s)
  -- Pad to multiple of 4 implicit; we just track bit position.
  local out = {}
  local buf, bits = 0, 0
  for i = 1, #s do
    local ch = s:sub(i, i)
    local v = B64U_REV[ch]
    if not v then return nil, "invalid b64url char at index " .. i end
    buf = buf * 64 + v
    bits = bits + 6
    if bits >= 8 then
      bits = bits - 8
      local byte = (buf // (2 ^ bits)) % 256
      out[#out + 1] = string.char(byte)
      buf = buf % (2 ^ bits)
    end
  end
  return table.concat(out)
end

--- Convert hex string ("deadbeef") to raw bytes.
function M.hex_to_bytes(hex)
  return (hex:gsub("..", function(b) return string.char(tonumber(b, 16)) end))
end

--- "Unwrap or throw" — useful because assay-lua replaces the lua
--- builtin `assert` global with a table (assert.eq, assert.not_nil, …)
--- and the (value, err) → value pattern loses its standard helper.
function M.must(v, err)
  if not v then
    error(err or "assertion failed", 2)
  end
  return v
end

--- Constant-time equality on byte strings (best-effort in pure lua).
function M.consteq(a, b)
  if type(a) ~= "string" or type(b) ~= "string" or #a ~= #b then
    return false
  end
  local diff = 0
  for i = 1, #a do
    diff = diff | (string.byte(a, i) ~ string.byte(b, i))
  end
  return diff == 0
end

return M
