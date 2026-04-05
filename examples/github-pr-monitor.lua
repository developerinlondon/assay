#!/usr/bin/assay
-- Monitor open PRs and notify via OpenClaw when new ones appear
-- Requires: GITHUB_TOKEN, OPENCLAW_URL, OPENCLAW_TOKEN
-- Usage: assay examples/github-pr-monitor.lua

local github = require("assay.github")
local openclaw = require("assay.openclaw")

local repo = env.get("GITHUB_REPO") or "developerinlondon/assay"
local gh = github.client()
local oc = openclaw.client()

-- Fetch open PRs
local prs = gh:pr_list(repo, { state = "open", per_page = 10 })
if not prs then
  log.warn("No PRs found or repo not accessible")
  return
end

-- Compare with previous state to detect new PRs
local pr_ids = {}
for _, pr in ipairs(prs) do
  pr_ids[#pr_ids + 1] = pr.number
end

local diff = oc:diff("pr-monitor-" .. repo:gsub("/", "-"), pr_ids)

if diff.changed then
  local msg = repo .. ": " .. #prs .. " open PRs"
  for _, pr in ipairs(prs) do
    msg = msg .. "\n  #" .. pr.number .. " " .. pr.title .. " (" .. pr.user.login .. ")"
  end
  oc:notify("dev-team", msg)
  log.info("Notified: " .. #prs .. " open PRs")
else
  log.info("No change: " .. #prs .. " open PRs")
end
