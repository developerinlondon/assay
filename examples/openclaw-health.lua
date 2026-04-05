#!/usr/bin/assay
-- Health check that reports status to OpenClaw
-- Requires: OPENCLAW_URL and OPENCLAW_TOKEN environment variables
-- Override: OPENCLAW_URL=http://localhost:8080 assay examples/openclaw-health.lua

local openclaw = require("assay.openclaw")
local hc = require("assay.healthcheck")

local c = openclaw.client()

-- Check a service endpoint
local target_url = env.get("HEALTH_TARGET") or "http://localhost:8080/health"
local result = hc.endpoint(target_url, { max_latency_ms = 2000 })

if result.ok then
  c:notify("ops", "Health check passed: " .. target_url .. " (" .. result.latency_ms .. "ms)")
  log.info("Health OK: " .. target_url .. " latency=" .. result.latency_ms .. "ms")
else
  c:send("slack", "#alerts", "Health check FAILED: " .. target_url .. " - " .. (result.error or "unknown"))
  log.error("Health FAILED: " .. target_url)
  error("Health check failed")
end
