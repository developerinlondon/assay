-- Tests for assay.pkg catalog layering (last-wins override + _origin tagging).
-- Run via tests/stdlib_pkg.rs harness.
local pkg = require("assay.pkg")

-- Local helper because assay registers `assert` as a global table.
local function check(cond, msg)
  if not cond then error(msg, 2) end
end

-- Build temp dirs to simulate "built-in" / "plugin" / "operator" layers.
-- os.getenv is not exposed in assay's sandbox; use /tmp directly.
math.randomseed(os.time())
local tmpbase = "/tmp/assay-pkg-layering-"
                 .. tostring(os.time()) .. "-" .. tostring(math.random(100000, 999999))
fs.mkdir(tmpbase)
local builtin_dir  = tmpbase .. "/builtin"
local empty_plugin = tmpbase .. "/empty_plugin"
local operator_dir = tmpbase .. "/operator"
fs.mkdir(builtin_dir)
fs.mkdir(empty_plugin)
fs.mkdir(operator_dir)

-- Built-in: ships test-foo with display_name="Built-in foo" AND test-baz (which
-- the operator will try to invalidly override later).
fs.write(builtin_dir .. "/test-foo.toml", [[
[package]
id           = "test-foo"
display_name = "Built-in foo"
description  = "From built-in layer"
homepage     = "https://example.com"
methods      = ["binary"]

[package.binary]
release_api   = "https://api.example.com/x"
asset_pattern = "x-{arch}"
sha256_source = "asset"
install_path  = "/usr/local/bin/x"
mode          = "0755"
]])

fs.write(builtin_dir .. "/test-baz.toml", [[
[package]
id           = "test-baz"
display_name = "Built-in baz (will be invalidly overridden)"
description  = "From built-in layer"
homepage     = "https://example.com"
methods      = ["binary"]

[package.binary]
release_api   = "https://api.example.com/baz"
asset_pattern = "baz-{arch}"
sha256_source = "asset"
install_path  = "/usr/local/bin/baz"
mode          = "0755"
]])

-- Operator: overrides test-foo with display_name="Operator foo" + adds test-bar +
-- INVALIDLY overrides test-baz (no methods).
fs.write(operator_dir .. "/foo-override.toml", [[
[package]
id           = "test-foo"
display_name = "Operator foo"
description  = "Overridden by operator"
homepage     = "https://operator.example.com"
methods      = ["binary"]

[package.binary]
release_api   = "https://api.operator.example.com/x"
asset_pattern = "x-{arch}"
sha256_source = "asset"
install_path  = "/usr/local/bin/x-operator"
mode          = "0755"
]])

fs.write(operator_dir .. "/test-bar.toml", [[
[package]
id           = "test-bar"
display_name = "Operator-only bar"
description  = "Operator-defined package"
homepage     = "https://example.com"
methods      = ["binary"]

[package.binary]
release_api   = "https://api.example.com/bar"
asset_pattern = "bar-{arch}"
sha256_source = "asset"
install_path  = "/usr/local/bin/bar"
mode          = "0755"
]])

-- Invalid override of test-baz: methods is empty array.
fs.write(operator_dir .. "/baz-broken.toml", [[
[package]
id           = "test-baz"
display_name = "Broken operator baz"
description  = "Invalid override (empty methods)"
homepage     = "https://example.com"
methods      = []
]])

local out = pkg.catalog.load({ builtin_dir, empty_plugin, operator_dir })

-- 1. test-foo: operator wins, full-entry override, install_path from operator.
local foo = out.entries["test-foo"]
check(foo, "test-foo should exist")
check(foo.display_name == "Operator foo",
      "operator override expected, got: " .. tostring(foo.display_name))
check(foo.binary.install_path == "/usr/local/bin/x-operator",
      "operator install_path expected, got: " .. tostring(foo.binary.install_path))
check(foo._origin:match("^operator:"),
      "_origin should be operator:..., got: " .. tostring(foo._origin))

-- 2. test-bar: only in operator layer.
local bar = out.entries["test-bar"]
check(bar, "test-bar should exist")
check(bar._origin:match("^operator:"),
      "_origin should be operator:..., got: " .. tostring(bar._origin))

-- 3. test-baz: STRICT-OVERRIDE — invalid override clears the valid built-in entry.
check(out.entries["test-baz"] == nil,
      "test-baz should be cleared (invalid override masks built-in)")
-- And the error must be recorded.
local found_baz_error = false
for _, e in ipairs(out.errors) do
  if e.package_id == "test-baz" then found_baz_error = true end
end
check(found_baz_error, "expected error recorded for test-baz invalid override")

-- 4. Re-load idempotency (rule 3 from spec, in the equivalent-content sense):
-- same source files → same set of entries with same display_name/origin.
local out2 = pkg.catalog.load({ builtin_dir, empty_plugin, operator_dir })
local foo2 = out2.entries["test-foo"]
check(foo2.display_name == foo.display_name, "re-load drift on display_name")
check(foo2._origin == foo._origin, "re-load drift on _origin")
check(out2.entries["test-baz"] == nil, "re-load: test-baz still cleared")

print("catalog_layering.lua OK")
