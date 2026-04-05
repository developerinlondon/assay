#!/usr/bin/assay

local gmail = require("assay.gmail")
local triage = require("assay.email_triage")
local openclaw = require("assay.openclaw")

local gc = gmail.client({
  credentials_file = env.get("GMAIL_CREDENTIALS_FILE"),
  token_file = env.get("GMAIL_TOKEN_FILE"),
})

local oc = openclaw.client()
local emails = gc:search(env.get("GMAIL_QUERY") or "is:unread newer_than:2d", { max = 20 })
local buckets = triage.categorize(emails)

local lines = {
  "Email triage results",
  "needs_action=" .. #buckets.needs_action,
  "needs_reply=" .. #buckets.needs_reply,
  "fyi=" .. #buckets.fyi,
}

if #buckets.needs_action > 0 then
  lines[#lines + 1] = "top_action=" .. (buckets.needs_action[1].subject or buckets.needs_action[1].snippet or "(no subject)")
end

oc:notify(env.get("OPENCLAW_NOTIFY_TARGET") or "ops", table.concat(lines, "\n"))
log.info(table.concat(lines, " | "))
