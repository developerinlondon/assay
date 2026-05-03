local audit = require("services.audit")

local M = {}

function M.export(req)
  local entries = audit.recent(200)
  local lines = {}
  for _, e in ipairs(entries) do
    local ok, encoded = pcall(json.encode, e)
    if ok then lines[#lines + 1] = encoded end
  end
  local body = table.concat(lines, "\n")
  if #lines > 0 then body = body .. "\n" end

  return {
    status = 200,
    body   = body,
    headers = {
      ["Content-Type"]        = "application/x-ndjson",
      ["Content-Disposition"] = 'attachment; filename="knowhere-audit.ndjson"',
    },
  }
end

return M
