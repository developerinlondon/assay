--! Stub `brand` service — fixture brand pack.

local M = {}

function M.snapshot()
  return {
    name        = "Test Brand",
    subtitle    = "smoke fixture",
    title       = "Test · hostops",
    accent_hex  = "#3366ee",
    favicon_url = nil,
  }
end

return M
