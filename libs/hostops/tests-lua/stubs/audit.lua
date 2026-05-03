--! Stub `audit` service — in-memory ring buffer.

local M = {}

local entries = {}

function M.append(entry)
  entry = entry or {}
  entry.ts = entry.ts or os.time()
  table.insert(entries, 1, entry)
  while #entries > 200 do table.remove(entries) end
end

function M.recent(n)
  n = n or 50
  local out = {}
  for i = 1, math.min(n, #entries) do out[i] = entries[i] end
  return out
end

function M.export()
  local lines = {}
  for _, e in ipairs(entries) do
    lines[#lines+1] = json.encode(e)
  end
  return table.concat(lines, "\n")
end

return M
