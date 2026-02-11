-- Requires: Loki running in Kubernetes (loki-stack or grafana/loki)
-- Override URL: LOKI_URL=http://localhost:3100 assay examples/loki-test.lua

local loki = require("assay.loki")

local loki_url = env.get("LOKI_URL") or "http://loki-gateway.monitoring:80"
local client = loki.client(loki_url)

assert.eq(client:ready(), true, "Loki not ready")
log.info("Loki is ready")

client:push(
  { app = "assay-test", job = "verification" },
  { "assay verification test entry" }
)
log.info("Loki push succeeded")

sleep(5)

local results = client:query('{app="assay-test"}', { limit = "1" })
assert.gt(#results, 0, "No log entries found in Loki")
log.info("Loki ingestion verified: " .. #results .. " streams found")
