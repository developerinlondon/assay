-- Grafana health check: verify API responds and database is healthy
local grafana_url = env.get("GRAFANA_URL") or "http://kube-prometheus-stack-grafana.monitoring:80"

local resp = http.get(grafana_url .. "/api/health")
assert.eq(resp.status, 200, "Grafana API not responding")

local health = json.parse(resp.body)
assert.eq(health.database, "ok", "Grafana database not healthy")

log.info("Grafana healthy: database=" .. health.database)
