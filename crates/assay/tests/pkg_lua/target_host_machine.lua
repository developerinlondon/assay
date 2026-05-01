-- Tests for assay.pkg target.host / target.machine abstractions.
-- Run via tests/stdlib_pkg.rs harness.
local pkg = require("assay.pkg")

-- Local helper because assay registers `assert` as a table.
local function check(cond, msg)
  if not cond then error(msg, 2) end
end

-- Host target
local host = pkg.target.host()
check(host.kind == "host", "host kind")
check(host.id == "host", "host id")
local r = host:exec("echo hi", {})
check(type(r) == "table" and r.status == 0, "host exec succeeds")
check(r.stdout:match("hi"), "host stdout captured: " .. tostring(r.stdout))

-- Machine target — calling machine_exec against a non-existent machine should
-- return non-zero status (via the wrapper) without throwing.
local m = pkg.target.machine("does-not-exist-xyz")
check(m.kind == "machine", "machine kind")
check(m.id == "does-not-exist-xyz", "machine id matches name")
local mr = m:exec("/bin/true", {})
check(type(mr) == "table", "machine exec returns table")
check(mr.status ~= 0, "non-existent machine should yield non-zero status")

-- Argument validation: empty/host/non-string names rejected.
local ok = pcall(function() pkg.target.machine("") end)
check(not ok, "empty machine name should error")
ok = pcall(function() pkg.target.machine("host") end)
check(not ok, "machine name 'host' is reserved")
ok = pcall(function() pkg.target.machine(nil) end)
check(not ok, "nil machine name should error")

print("target_host_machine.lua OK")
