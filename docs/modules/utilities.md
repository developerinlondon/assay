## log

Structured logging. No `require()` needed.

- `log.info(msg)` — Log info message
- `log.warn(msg)` — Log warning message
- `log.error(msg)` — Log error message

## env

Environment variable access. No `require()` needed.

- `env.get(key)` → string|nil — Get environment variable value

## sleep

Sleep utility. No `require()` needed.

- `sleep(secs)` → nil — Sleep for N seconds (supports fractional: `sleep(0.5)`)

## time

Timestamp utility. No `require()` needed.

- `time()` → number — Unix timestamp in seconds (with fractional milliseconds)

## string

Lua's built-in `string` library (see [the Lua 5.4 reference](https://www.lua.org/manual/5.4/manual.html#6.4))
is available as always — `string.format`, `string.gsub`, `string.match`, `string.gmatch`, etc.
Assay extends it with one awk-style helper:

- `string.split(s, sep?)` → `{parts}` — Split a string into an array of parts.
  When `sep` is `nil` (or empty), splits on any run of whitespace and skips
  leading/trailing empty fields (matches awk's default FS and Python's
  `str.split()` with no arg). When `sep` is provided, splits on the literal
  string (NOT a Lua pattern — use `string.gmatch` if you need pattern
  semantics). Pairs well with `fs.lines` for awk-style field processing:

```lua
-- awk '{ print $2 }' users.tsv
for line in fs.lines("users.tsv") do
  print(string.split(line)[2])
end

-- awk -F, '{ sum[$1] += $2 }' stats.csv
local sum = {}
for line in fs.lines("stats.csv") do
  local f = string.split(line, ",")
  sum[f[1]] = (sum[f[1]] or 0) + tonumber(f[2])
end
```
