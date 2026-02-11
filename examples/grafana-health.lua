local grafana = require("assay.grafana")
local hc = require("assay.healthcheck")

local grafana_url = env.get("GRAFANA_URL") or "http://kube-prometheus-stack-grafana.monitoring:80"

local client = grafana.client(grafana_url)
local health = client:health()

assert.eq(health.database, "ok", "Grafana database not healthy")
log.info("Grafana healthy: database=" .. health.database .. " version=" .. health.version)

local result = hc.endpoint(grafana_url .. "/api/health")
assert.eq(result.ok, true, "Grafana endpoint check failed")
log.info("Grafana response time: " .. result.latency_ms .. "ms")
