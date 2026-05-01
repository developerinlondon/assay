-- Tests for assay.pkg catalog loader and validator.
-- Run via tests/stdlib_pkg.rs harness.
--
-- NOTE: assay registers `assert` as a global table (assert.eq, assert.not_nil, etc.),
-- which shadows Lua's built-in assert function. We use a local helper instead.
local function check(cond, msg)
  if not cond then error(msg or "assertion failed", 2) end
end

local pkg = require("assay.pkg")
local fixtures = "tests/fixtures/pkg_catalog"

-- 1. Load valid fixtures: should return three entries, three errors (from invalid_*).
local out = pkg.catalog.load({ fixtures })
check(type(out) == "table", "load should return a table")
check(type(out.entries) == "table", "out.entries must be a table")
check(type(out.errors) == "table", "out.errors must be a table")

-- The three valid fixtures must load.
check(out.entries["test-apt-only"], "test-apt-only should load")
check(out.entries["test-binary-only"], "test-binary-only should load")
check(out.entries["test-both"], "test-both should load")

-- The three invalid fixtures must produce errors.
local err_ids = {}
for _, e in ipairs(out.errors) do err_ids[e.package_id or e.path] = e end
check(#out.errors >= 3, "expected at least 3 errors, got " .. #out.errors)

-- 2. _origin tagging: built-in directory (the only one we passed) should be tagged "built-in".
check(out.entries["test-apt-only"]._origin == "built-in",
      "first-layer entries should be tagged built-in")

-- 3. Method shape: catalog.get returns the same entry as table lookup.
local both = pkg.catalog.get(out.entries, "test-both")
check(both.methods[1] == "apt" and both.methods[2] == "binary",
      "methods order preserved")
check(both.apt.package_name == "test-both", "apt block accessible")
check(both.binary.install_path == "/usr/local/bin/test-both",
      "binary block accessible")

-- 4. catalog.list returns sorted-by-id array.
local listed = pkg.catalog.list(out.entries)
check(#listed == 3, "list should return 3 entries")
check(listed[1].id <= listed[2].id and listed[2].id <= listed[3].id,
      "list is sorted by id")

print("catalog_validation.lua OK")
