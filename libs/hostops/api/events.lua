local state = require("services.state")
local M = {}

function M.events(_req)
  return {
    sse = function(send)
      local last = -1
      while true do
        local cur = state.revision()
        if cur ~= last then
          send({ event = "refresh", data = tostring(cur) })
          last = cur
        else
          send({ data = "" })
        end
        sleep(2)
      end
    end,
    headers = {
      ["Content-Type"]      = "text/event-stream",
      ["Cache-Control"]     = "no-cache",
      ["X-Accel-Buffering"] = "no",
    },
  }
end

return M
