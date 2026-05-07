-- Tests for assay.pkg deterministic plan generation.
-- Run via tests/stdlib_pkg.rs harness.
local pkg = require("assay.pkg")

-- Synthetic catalog with two packages, both binary-only (apt requires a real host).
local catalog_entries = {
  ["alpha"] = {
    id = "alpha", display_name = "Alpha", methods = {"binary"},
    binary = {
      release_api = "https://api.example.com/alpha",
      asset_pattern = "alpha-{arch}",
      sha256_source = "asset",
      install_path = "/usr/local/bin/alpha",
      mode = "0755",
    },
    _origin = "built-in",
  },
  ["beta"] = {
    id = "beta", display_name = "Beta", methods = {"binary"},
    binary = {
      release_api = "https://api.example.com/beta",
      asset_pattern = "beta-{arch}",
      sha256_source = "asset",
      install_path = "/usr/local/bin/beta",
      mode = "0755",
    },
    _origin = "built-in",
  },
}

-- Case 1: nothing installed, both desired → install both, in id order.
local actual_empty = {}
local desired = { "alpha", "beta" }
local plan = pkg.plan("host", desired, actual_empty, catalog_entries)
assert.eq(#plan, 2, "expected 2 ops, got " .. #plan)
assert.eq(plan[1].op, "install", "alpha install first")
assert.eq(plan[1].id, "alpha", "alpha install first")
assert.eq(plan[2].op, "install", "beta install second")
assert.eq(plan[2].id, "beta", "beta install second")
assert.eq(plan[1].method, "binary", "binary method recorded")

-- Case 2: alpha installed at current, beta missing → only beta install.
local actual_alpha_ok = {
  alpha = { installed = true, version = "1.0.0", available = "1.0.0" },
}
local plan2 = pkg.plan("host", desired, actual_alpha_ok, catalog_entries)
assert.eq(#plan2, 1, "expected 1 op, got " .. #plan2)
assert.eq(plan2[1].id, "beta", "beta install only")
assert.eq(plan2[1].op, "install", "beta install only")

-- Case 3: alpha installed but outdated → upgrade.
local actual_alpha_old = {
  alpha = { installed = true, version = "0.9.0", available = "1.0.0" },
  beta  = { installed = true, version = "2.0.0", available = "2.0.0" },
}
local plan3 = pkg.plan("host", desired, actual_alpha_old, catalog_entries)
assert.eq(#plan3, 1, "expected 1 op")
assert.eq(plan3[1].op, "upgrade", "alpha upgrade")
assert.eq(plan3[1].id, "alpha", "alpha upgrade")
assert.eq(plan3[1].from, "0.9.0", "from/to fields")
assert.eq(plan3[1].to, "1.0.0", "from/to fields")

-- Idempotency rule 1: everything at target → empty plan.
local actual_all_ok = {
  alpha = { installed = true, version = "1.0.0", available = "1.0.0" },
  beta  = { installed = true, version = "2.0.0", available = "2.0.0" },
}
local empty_plan = pkg.plan("host", desired, actual_all_ok, catalog_entries)
assert.eq(#empty_plan, 0, "fully-converged → empty plan, got " .. #empty_plan)

-- Reconcile NEVER removes (spec §Reconciler).
-- A package installed but not in desired set produces no op.
local actual_with_extra = {
  alpha = { installed = true, version = "1.0.0", available = "1.0.0" },
  beta  = { installed = true, version = "2.0.0", available = "2.0.0" },
  gamma = { installed = true, version = "3.0.0", available = "3.0.0" },
}
local plan_no_remove = pkg.plan("host", { "alpha", "beta" }, actual_with_extra, catalog_entries)
assert.eq(#plan_no_remove, 0, "extras must NOT trigger remove ops, got " .. #plan_no_remove)

-- Determinism: re-plan same inputs → identical output (full op shape, not just op+id).
local plan_a = pkg.plan("host", desired, actual_empty, catalog_entries)
local plan_b = pkg.plan("host", desired, actual_empty, catalog_entries)
assert.eq(#plan_a, #plan_b, "plan length must be stable across calls")
for i = 1, #plan_a do
  local a, b = plan_a[i], plan_b[i]
  assert.eq(a.op, b.op, "op[" .. i .. "].op differs")
  assert.eq(a.id, b.id, "op[" .. i .. "].id differs")
  assert.eq(a.method, b.method, "op[" .. i .. "].method differs")
  assert.eq(a.target_version, b.target_version, "op[" .. i .. "].target_version differs")
  assert.eq(a.from, b.from, "op[" .. i .. "].from differs")
  assert.eq(a.to, b.to, "op[" .. i .. "].to differs")
  assert.eq(a.reason, b.reason, "op[" .. i .. "].reason differs")
end

-- actual = nil should be tolerated (treated as empty actual map).
local plan_nil_actual = pkg.plan("host", { "alpha" }, nil, catalog_entries)
assert.eq(#plan_nil_actual, 1, "nil actual should be treated as empty")
assert.eq(plan_nil_actual[1].op, "install", "nil actual: should propose install for alpha")
assert.eq(plan_nil_actual[1].id, "alpha", "nil actual: should propose install for alpha")

-- Sort independence: desired list given in different orders should produce same plan.
local plan_c = pkg.plan("host", { "beta", "alpha" }, actual_empty, catalog_entries)
assert.eq(plan_c[1].id, "alpha", "plan should sort desired internally; operator order should not matter")
assert.eq(plan_c[2].id, "beta", "plan should sort desired internally; operator order should not matter")

-- Skip op for unknown catalog id.
local plan_skip = pkg.plan("host", { "alpha", "unknown-id" }, actual_empty, catalog_entries)
assert.eq(#plan_skip, 2, "unknown id should still produce a skip op")
local found_skip = false
for _, op in ipairs(plan_skip) do
  if op.op == "skip" and op.id == "unknown-id" then found_skip = true end
end
assert.eq(found_skip, true, "expected skip op for unknown-id")

print("plan_idempotency.lua OK")
