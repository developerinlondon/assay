-- Tests for assay.pkg catalog loader and validator.
-- Run via tests/stdlib_pkg.rs harness.
--
local pkg = require("assay.pkg")
local fixtures = "tests/fixtures/pkg_catalog"

-- 1. Load valid fixtures: should return three entries, three errors (from invalid_*).
local out = pkg.catalog.load({ fixtures })
assert.eq(type(out), "table", "load should return a table")
assert.eq(type(out.entries), "table", "out.entries must be a table")
assert.eq(type(out.errors), "table", "out.errors must be a table")

-- The three valid fixtures must load.
assert.not_nil(out.entries["test-apt-only"], "test-apt-only should load")
assert.not_nil(out.entries["test-binary-only"], "test-binary-only should load")
assert.not_nil(out.entries["test-both"], "test-both should load")

-- The three invalid fixtures must produce errors.
local err_ids = {}
for _, e in ipairs(out.errors) do err_ids[e.package_id or e.path] = e end
assert.gt(#out.errors, 2, "expected at least 3 errors, got " .. #out.errors)

-- 2. _origin tagging: built-in directory (the only one we passed) should be tagged "built-in".
assert.eq(out.entries["test-apt-only"]._origin, "built-in",
  "first-layer entries should be tagged built-in")

-- 3. Method shape: catalog.get returns the same entry as table lookup.
local both = pkg.catalog.get(out.entries, "test-both")
assert.eq(both.methods[1], "apt", "methods order preserved")
assert.eq(both.methods[2], "binary", "methods order preserved")
assert.eq(both.apt.package_name, "test-both", "apt block accessible")
assert.eq(both.binary.install_path, "/usr/local/bin/test-both",
  "binary block accessible")

-- 4. catalog.list returns sorted-by-id array.
local listed = pkg.catalog.list(out.entries)
assert.eq(#listed, 3, "list should return 3 entries")
assert.eq(listed[1].id <= listed[2].id and listed[2].id <= listed[3].id, true,
  "list is sorted by id")

print("catalog_validation.lua OK")
