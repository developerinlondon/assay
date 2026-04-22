local path = "/tmp/assay_e2e_test.txt"
fs.write(path, "e2e test content")
local content = fs.read(path)
assert.eq(content, "e2e test content")
