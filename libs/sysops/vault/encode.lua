--! sysops.vault.encode - URL encoding helpers shared across vault submodules.

local M = {}

function M.segment(value)
  value = tostring(value or "")
  return (value:gsub("([^%w%-%._~])", function(ch)
    return string.format("%%%02X", string.byte(ch))
  end))
end

function M.path(value)
  local parts = {}
  for part in tostring(value or ""):gmatch("[^/]+") do
    table.insert(parts, M.segment(part))
  end
  return table.concat(parts, "/")
end

return M
