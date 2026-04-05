#!/usr/bin/assay

local gmail = require("assay.gmail")
local openclaw = require("assay.openclaw")

local gc = gmail.client({
  credentials_file = env.get("GMAIL_CREDENTIALS_FILE"),
  token_file = env.get("GMAIL_TOKEN_FILE"),
})

local oc = openclaw.client()
local messages = gc:search(env.get("GMAIL_QUERY") or "is:unread newer_than:1d", { max = 10 })

local digest = oc:llm_task("Summarize these unread emails into a short digest with priorities.", {
  artifacts = messages,
  temperature = 0.1,
})

log.info("Unread messages: " .. #messages)
if digest.response then
  log.info(digest.response)
elseif digest.summary then
  log.info(digest.summary)
else
  log.info(json.encode(digest))
end
