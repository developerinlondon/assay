local hash = crypto.hash("hello", "sha256")
assert.eq(#hash, 64)
local rand = crypto.random(16)
assert.eq(#rand, 16)
