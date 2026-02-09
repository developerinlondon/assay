-- Prometheus scrape targets check: verify expected targets are being scraped
local prom_url = env.get("PROMETHEUS_URL") or "http://kube-prometheus-stack-prometheus.monitoring:9090"

-- Check total number of up targets
local up_count = prometheus.query(prom_url, "count(up)")
assert.gt(up_count, 0, "No scrape targets found")
log.info("Total scrape targets: " .. tostring(up_count))

-- Check that all targets are healthy (up == 1)
local down_count = prometheus.query(prom_url, "count(up == 0)")
if type(down_count) == "number" and down_count > 0 then
    log.warn("Found " .. tostring(down_count) .. " down targets")
end

-- Verify specific critical targets exist
local node_exporter = prometheus.query(prom_url, 'count(up{job="node-exporter"})')
assert.gt(node_exporter, 0, "node-exporter not being scraped")
log.info("node-exporter targets: " .. tostring(node_exporter))
