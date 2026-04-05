#!/usr/bin/assay

local gcal = require("assay.gcal")

local c = gcal.client({
  credentials_file = env.get("GCAL_CREDENTIALS_FILE"),
  token_file = env.get("GCAL_TOKEN_FILE"),
})

local now = time()
local start_of_day = now:sub(1, 10) .. "T00:00:00Z"
local end_of_day = now:sub(1, 10) .. "T23:59:59Z"
local events = c:events({
  timeMin = start_of_day,
  timeMax = end_of_day,
  maxResults = 20,
})

log.info("Today's agenda")
for _, event in ipairs(events) do
  local start_at = event.start and (event.start.dateTime or event.start.date) or "unknown"
  log.info("- " .. start_at .. " | " .. (event.summary or "(no title)"))
end
