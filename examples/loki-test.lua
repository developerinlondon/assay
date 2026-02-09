-- Loki ingestion test: push a log entry and query it back
local loki_url = env.get("LOKI_URL") or "http://loki-gateway.monitoring:80"

-- Push a test log entry via Loki push API
local timestamp = tostring(math.floor(time() * 1000000000))
local push_body = '{"streams":[{"stream":{"app":"assay-test","job":"verification"},"values":[["' .. timestamp .. '","assay verification test entry"]]}]}'

local push_resp = http.post(loki_url .. "/loki/api/v1/push", push_body, {
    headers = { ["Content-Type"] = "application/json" }
})
assert.eq(push_resp.status, 204, "Loki push failed with status " .. tostring(push_resp.status))
log.info("Loki push succeeded")

-- Wait for ingestion
sleep(5)

-- Query it back
local query_resp = http.get(loki_url .. "/loki/api/v1/query?query=%7Bapp%3D%22assay-test%22%7D&limit=1")
assert.eq(query_resp.status, 200, "Loki query failed with status " .. tostring(query_resp.status))

local data = json.parse(query_resp.body)
assert.not_nil(data.data, "No data in Loki response")
assert.gt(#data.data.result, 0, "No log entries found in Loki")

log.info("Loki ingestion verified: " .. #data.data.result .. " streams found")
