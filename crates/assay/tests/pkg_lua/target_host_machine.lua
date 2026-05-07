-- Tests for assay.pkg target.host / target.machine abstractions.
-- Run via tests/stdlib_pkg.rs harness.
local pkg = require("assay.pkg")

-- Host target
local host = pkg.target.host()
assert.eq(host.kind, "host", "host kind")
assert.eq(host.id, "host", "host id")
local r = host:exec("echo hi", {})
assert.eq(type(r), "table", "host exec returns table")
assert.eq(r.status, 0, "host exec succeeds")
assert.not_nil(r.stdout:match("hi"), "host stdout captured: " .. tostring(r.stdout))

-- Machine target — calling machine_exec against a non-existent machine should
-- return non-zero status (via the wrapper) without throwing.
local m = pkg.target.machine("does-not-exist-xyz")
assert.eq(m.kind, "machine", "machine kind")
assert.eq(m.id, "does-not-exist-xyz", "machine id matches name")
local mr = m:exec("/bin/true", {})
assert.eq(type(mr), "table", "machine exec returns table")
assert.ne(mr.status, 0, "non-existent machine should yield non-zero status")

-- Argument validation: empty/host/non-string names rejected.
local ok = pcall(function() pkg.target.machine("") end)
assert.eq(ok, false, "empty machine name should error")
ok = pcall(function() pkg.target.machine("host") end)
assert.eq(ok, false, "machine name 'host' is reserved")
ok = pcall(function() pkg.target.machine(nil) end)
assert.eq(ok, false, "nil machine name should error")

print("target_host_machine.lua OK")
