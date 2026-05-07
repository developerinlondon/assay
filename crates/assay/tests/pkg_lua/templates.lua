-- Tests for assay.pkg template loader and cross-catalog validation.
-- Run via tests/stdlib_pkg.rs harness.
local pkg = require("assay.pkg")

-- Need a catalog to validate template package references against.
-- Paths relative to crate root (cargo test cwd).
local catalog_dir = "tests/fixtures/pkg_catalog"
local templates_dir = "tests/fixtures/pkg_templates"

local cat = pkg.catalog.load({ catalog_dir })
local tpl = pkg.templates.load({ templates_dir }, cat.entries)

assert.eq(type(tpl), "table", "templates.load returns table")
assert.not_nil(tpl.entries, "templates.load returns entries")
assert.not_nil(tpl.errors, "templates.load returns errors")

-- valid_default.toml references test-apt-only and test-binary-only — both exist in catalog.
local def = tpl.entries["default"]
assert.not_nil(def, "default template should load")
assert.eq(def.packages[1], "test-apt-only", "packages preserved")

-- invalid_bad_pkg.toml references "does-not-exist" — should be rejected.
local bad = tpl.entries["bad"]
assert.eq(bad, nil, "template referencing missing catalog id should be rejected")
local found_err = false
for _, e in ipairs(tpl.errors) do
  if e.template_id == "bad" and e.message:find("does-not-exist", 1, true) then
    found_err = true; break
  end
end
assert.eq(found_err, true, "expected error mentioning the missing catalog id")

-- Sorted listing — only the valid default template should appear.
local listed = pkg.templates.list(tpl.entries)
assert.eq(#listed, 1, "only the default template should list")

-- _origin tagging on first-layer entries.
assert.eq(def._origin, "built-in",
  "first-layer template entries should be tagged built-in, got: " .. tostring(def._origin))

print("templates.lua OK")
