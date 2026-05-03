local M = {}

local function parse_int(s)
  if not s or s == "" then return nil end
  return tonumber(s)
end

function M.stream(req)
  local q    = req.params or {}
  local opts = { lines = 0 }

  if q.machine and q.machine ~= "" then opts.machine  = q.machine  end
  if q.unit    and q.unit    ~= "" then opts.unit     = q.unit     end
  local pri = parse_int(q.priority)
  if pri then opts.priority = pri end

  return {
    sse = function(send)
      systemd.journal_follow(opts, function(entry)
        send({ event = "log", data = json.encode(entry) })
      end)
    end,
    headers = {
      ["Content-Type"]      = "text/event-stream",
      ["Cache-Control"]     = "no-cache",
      ["X-Accel-Buffering"] = "no",
    },
  }
end

return M
