-- Requires: Prometheus running in Kubernetes (kube-prometheus-stack)
-- Override URL: PROMETHEUS_URL=http://localhost:9090 assay examples/prometheus-scrape.lua

local prom = require("assay.prometheus")

local prom_url = env.get("PROMETHEUS_URL") or "http://kube-prometheus-stack-prometheus.monitoring:9090"
local c = prom.client(prom_url)

local up_count = c.queries:instant("count(up)")
assert.gt(up_count, 0, "No scrape targets found")
log.info("Total scrape targets: " .. tostring(up_count))

local down_count = c.queries:instant("count(up == 0)")
if type(down_count) == "number" and down_count > 0 then
  log.warn("Found " .. tostring(down_count) .. " down targets")
end

local targets = c.targets:list()
log.info("Active targets: " .. #targets.activeTargets)
log.info("Dropped targets: " .. #targets.droppedTargets)

local node_exporter = c.queries:instant('count(up{job="node-exporter"})')
assert.gt(node_exporter, 0, "node-exporter not being scraped")
log.info("node-exporter targets: " .. tostring(node_exporter))

local alerts = c.alerts:list()
if #alerts > 0 then
  log.warn("Active alerts: " .. #alerts)
  for _, alert in ipairs(alerts) do
    log.warn("  " .. alert.labels.alertname .. " (" .. alert.state .. ")")
  end
else
  log.info("No active alerts")
end
