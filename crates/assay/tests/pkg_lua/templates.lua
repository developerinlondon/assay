-- Tests for assay.pkg template loader and cross-catalog validation.
-- Run via tests/stdlib_pkg.rs harness.
local pkg = require("assay.pkg")

-- Local helper because assay registers `assert` as a table.
local function check(cond, msg)
  if not cond then error(msg, 2) end
end

-- Need a catalog to validate template package references against.
-- Paths relative to crate root (cargo test cwd).
local catalog_dir = "tests/fixtures/pkg_catalog"
local templates_dir = "tests/fixtures/pkg_templates"

local cat = pkg.catalog.load({ catalog_dir })
local tpl = pkg.templates.load({ templates_dir }, cat.entries)

check(type(tpl) == "table" and tpl.entries and tpl.errors,
      "templates.load returns {entries, errors}")

-- valid_default.toml references test-apt-only and test-binary-only — both exist in catalog.
local def = tpl.entries["default"]
check(def, "default template should load")
check(def.packages[1] == "test-apt-only", "packages preserved")

-- invalid_bad_pkg.toml references "does-not-exist" — should be rejected.
local bad = tpl.entries["bad"]
check(bad == nil, "template referencing missing catalog id should be rejected")
local found_err = false
for _, e in ipairs(tpl.errors) do
  if e.template_id == "bad" and e.message:find("does-not-exist", 1, true) then
    found_err = true; break
  end
end
check(found_err, "expected error mentioning the missing catalog id")

-- Sorted listing — only the valid default template should appear.
local listed = pkg.templates.list(tpl.entries)
check(#listed == 1, "only the default template should list")

-- _origin tagging on first-layer entries.
check(def._origin == "built-in",
      "first-layer template entries should be tagged built-in, got: " .. tostring(def._origin))

print("templates.lua OK")
