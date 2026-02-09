-- Integration test: Lua script that passes
local resp = http.get("https://httpbin.org/get")
assert.eq(resp.status, 200, "Expected 200 from httpbin")

local data = json.parse(resp.body)
assert.not_nil(data.url, "Expected url field in response")

log.info("Script test passed: url=" .. data.url)
