-- Tests for assay.pkg version parser and comparator.
-- Run via tests/stdlib_pkg.rs harness.
local pkg = require("assay.pkg")

-- Local helper because assay registers `assert` as a table.
local function check(cond, msg)
  if not cond then error(msg, 2) end
end

-- parse: returns sortable array of integers, dropping non-numeric prefixes/suffixes.
local function check_parse(input, expected)
  local got = pkg.version.parse(input)
  check(#got == #expected,
        ("parse(%q): #got=%d #expected=%d"):format(input, #got, #expected))
  for i, n in ipairs(expected) do
    check(got[i] == n,
          ("parse(%q)[%d]: got=%s want=%s"):format(input, i, tostring(got[i]), tostring(n)))
  end
end

check_parse("1.2.3", {1,2,3})
check_parse("v1.2.3", {1,2,3})
check_parse("0.10.0", {0,10,0})
check_parse("2024.10.1", {2024,10,1})
check_parse("2024.9.0", {2024,9,0})

-- cmp: -1 if a<b, 0 if equal, 1 if a>b.
check(pkg.version.cmp("1.2.3", "1.2.3") == 0,  "equal")
check(pkg.version.cmp("1.2.3", "1.2.4") == -1, "patch <")
check(pkg.version.cmp("1.2.4", "1.2.3") == 1,  "patch >")
check(pkg.version.cmp("0.9.9", "0.10.0") == -1, "two-digit minor not lex")
check(pkg.version.cmp("v1.2.3", "1.2.3") == 0,  "v-prefix equal")
check(pkg.version.cmp("2024.9.0", "2024.10.1") == -1, "calver same year")
check(pkg.version.cmp("2023.12.31", "2024.1.1") == -1, "calver year boundary")

-- Unequal lengths: shorter compared as zero-padded.
check(pkg.version.cmp("1.2", "1.2.0") == 0,  "1.2 == 1.2.0")
check(pkg.version.cmp("1.2", "1.2.1") == -1, "1.2 < 1.2.1")

print("version_compare.lua OK")
