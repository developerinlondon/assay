-- Tests for assay.nspawn name validation.
-- All entry points (provision/destroy/start/stop/get/exists/wait_ready/
-- config.read/config.write) call validate_name; we exercise a representative
-- path to confirm.
local nspawn = require("assay.nspawn")

local function expect_error(fn, ...)
  local ok, _ = pcall(fn, ...)
  assert.eq(ok, false, "expected error from " .. tostring(fn))
end

-- ── valid names: alphanumerics + . _ - ────────────────────────────────────
-- We use config.read which only validates name + tries fs.exists; doesn't
-- mutate anything.
nspawn.config.read("apex")          -- valid, returns nil because file absent
nspawn.config.read("agent-x")
nspawn.config.read("node_01")
nspawn.config.read("host.local")
nspawn.config.read("ABC123")

-- ── reject empty / nil ────────────────────────────────────────────────────
expect_error(nspawn.config.read, "")
expect_error(nspawn.config.read, nil)
expect_error(nspawn.config.read, 42)

-- ── reject path traversal ─────────────────────────────────────────────────
expect_error(nspawn.config.read, "../../../etc/passwd")
expect_error(nspawn.config.read, "name/with/slash")
expect_error(nspawn.config.read, "x/y")

-- ── reject shell metacharacters ───────────────────────────────────────────
expect_error(nspawn.config.read, "name;rm")
expect_error(nspawn.config.read, "$(whoami)")
expect_error(nspawn.config.read, "x y")           -- space
expect_error(nspawn.config.read, "x'y")
expect_error(nspawn.config.read, "x`y")

-- ── reject leading dash (would parse as systemctl flag) ───────────────────
expect_error(nspawn.config.read, "-rf")
expect_error(nspawn.config.read, "--help")

print("name_validation.lua OK")
