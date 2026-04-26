## assay.version

Cross-scheme version comparison. Pure Lua, no I/O. Schemes: `semver` (default), `debian`, `rpm`,
`numeric`.

- `version.compare(a, b, scheme?)` → `-1|0|1` — Compare two version strings.
- `version.max(list, scheme?)` → `string` — Return the largest version in `list`.

Scheme rules:

- **semver** — strips leading `v`/`V`; splits on `.` then `-` (pre-release) and `+` (build,
  ignored). Numeric segments compare numerically; numeric pre-release identifiers sort before
  alphanumeric ones; a version with a pre-release is less than the same version without one.
- **debian** — `[epoch:]upstream[-revision]`; epoch defaults to `0`. Strings split into runs of
  digits (compared numerically) and non-digits (where `~` sorts before the empty string, then
  letters, then everything else).
- **rpm** — like debian but no epoch handling and no tilde rule (`~` is just another character).
- **numeric** — pure dotted integers; missing segments default to `0`, so `1.2 == 1.2.0` and
  `1.10 > 1.9`.

Unknown scheme → error.

```lua
local version = require("assay.version")

assert.eq(version.compare("1.2.3", "1.2.4"), -1)
assert.eq(version.compare("v0.13.1", "0.13.2", "semver"), -1)
assert.eq(version.compare("1:1.84.3-noble1", "1.84.2", "debian"), 1)
assert.eq(version.compare("1.0~rc1", "1.0", "debian"), -1)
assert.eq(version.compare("1.10", "1.9", "numeric"), 1)
assert.eq(version.max({ "1.2", "1.10", "1.9" }, "semver"), "1.10")
```
