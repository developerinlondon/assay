local encoded = base64.encode("hello world")
local decoded = base64.decode(encoded)
assert.eq(decoded, "hello world")
