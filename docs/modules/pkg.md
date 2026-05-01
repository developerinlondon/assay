---
category: Infrastructure
tagline: Package manager framework — catalog, templates, targets, plan/reconcile (v0.16.0+)
---

## assay.pkg

Package manager framework — catalog loading, target abstractions, plan generation, and version
comparison. Introduced in v0.16.0.

```lua
local pkg = require("assay.pkg")
```

### Layering model

`catalog.load` and `templates.load` both accept an ordered array of directory paths. Entries from
later layers **overwrite** entries with the same `id` (full-entry override, no field-merge):

| Layer index | `_origin` value |
|-------------|-----------------|
| 1           | `"built-in"` |
| 2           | `"plugin:<dirname>"` |
| 3+          | `"operator:<filename>"` |

**Strict-override:** if a later-layer entry fails validation, any earlier valid entry with the same
`id` is cleared (the caller sees a missing entry + a validation error, never a silent fallback).

`_origin` is a synthetic field — not part of the on-disk TOML schema. Downstream serializers (API
responses, plan writers) must strip it before output.

---

### pkg.catalog

- `pkg.catalog.load(paths)` → `{entries, errors}` — Load catalog TOML files from layered
  directories. `paths` is a string array of directory paths (non-existent dirs silently skipped).
  - `entries`: `{[id] = entry_table}` — map of valid entries, each tagged with `_origin`
  - `errors`: `[{path, package_id, field, message}]` — all validation errors encountered

- `pkg.catalog.get(entries, id)` → `entry | nil` — Look up a single entry by id.

- `pkg.catalog.list(entries)` → `[entry]` — All entries as an array, sorted by `id`.

**Catalog TOML shape:**

```toml
[package]
id           = "curl"
display_name = "cURL"
methods      = ["apt"]        # ordered; first method is preferred at plan time

[package.apt]
source_list  = "deb https://pkgs.example.com/debian stable main"
package_name = "curl"

[package.binary]              # optional; enables binary install fallback
release_api     = "https://api.github.com/repos/example/curl/releases/latest"
asset_pattern   = "curl-{ver}-linux-{arch}.tar.gz"
sha256_source   = "checksums"  # one of: "asset" (sibling .sha256), "checksums" (sha256sums.txt)
install_path    = "/usr/local/bin/curl"
mode            = "0755"
```

---

### pkg.templates

Templates group catalog ids into named sets.

- `pkg.templates.load(paths, catalog_entries)` → `{entries, errors}` — Load template TOML files.
  `catalog_entries` is the `entries` map from `pkg.catalog.load`; any template package id not
  present in the catalog is rejected with a validation error.

- `pkg.templates.get(entries, id)` → `entry | nil`

- `pkg.templates.list(entries)` → `[entry]` — Sorted by `id`.

**Template TOML shape:**

```toml
[template]
id           = "base"
display_name = "Base tooling"
packages     = ["curl", "jq", "git"]
```

---

### pkg.target

- `pkg.target.host()` → `target` — Return the host target singleton.
- `pkg.target.machine(name)` → `target` — Return a target wrapping a systemd-machined nspawn
  container. `name` must be a non-empty string and not the reserved word `"host"`.

**Target:exec(cmd, opts?)**

Run a command on the target. Returns `{status, stdout, stderr, timed_out}` (same shape as
`shell.exec`).

The safe cross-target `opts` subset is:

| Key       | Type            | Description |
|-----------|-----------------|-------------|
| `timeout` | number          | Seconds; `0` means no timeout |
| `env`     | table           | `{[name] = value}` extra environment variables |
| `stdin`   | string \| bytes | Bytes piped to the inner process via systemd-run --pipe / shell.exec stdin |

`shell.exec`-only opts (`cwd`) are silently dropped on machine targets — there's no
working-directory equivalent for a transient nspawn unit. Passing them is out-of-contract.

```lua
local t = pkg.target.host()
local r = t:exec("whoami", { timeout = 5 })
print(r.stdout)   -- "root\n"

local m = pkg.target.machine("mycontainer")
local r2 = m:exec("dpkg-query -W curl", { timeout = 10 })
if r2.timed_out then error("exec timed out") end
```

---

### pkg.version

Simple SemVer-style comparator (integers only; non-numeric trailing components silently dropped).
Strips a leading `"v"` before parsing.

- `pkg.version.parse(s)` → `[integer]` — Parse a version string into an integer array. Returns
  `{0}` for unparseable input rather than `nil`.

- `pkg.version.cmp(a, b)` → `-1 | 0 | 1` — Compare two version strings. Shorter arrays are
  zero-padded to match the longer.

```lua
pkg.version.cmp("1.2.3", "1.2.4")   -- -1
pkg.version.cmp("v2.0", "2.0.0")    -- 0  (leading "v" stripped; zero-padded)
```

---

### pkg.plan

```lua
local ops = pkg.plan(target_id, desired_set, actual, catalog_entries)
```

Pure function — no I/O, no side effects. Builds a deterministic operation array to converge
`actual` toward `desired_set`.

- `target_id` (string): informational only; included for logging (`"host"` or machine name)
- `desired_set` (string[]): catalog ids to ensure are installed; order ignored, sorted internally
- `actual` (table): `{[id] = {installed=bool, version=string?, available=string?}}` — current
  observed state per id
- `catalog_entries` (table): the `entries` map from `pkg.catalog.load`

**Returns** an array of operation tables. Each operation has at minimum `{op, id, method}`:

| `op`        | Additional fields | Meaning |
|-------------|-------------------|---------|
| `"install"` | `target_version`  | Package not installed |
| `"upgrade"` | `from`, `to`      | Installed but `version < available` |
| `"skip"`    | `reason`          | No catalog entry found for id |

`pkg.plan` **never removes** — packages in `actual` but not in `desired_set` are ignored.

```lua
local catalog = pkg.catalog.load({ "/opt/myapp/catalog", "/etc/myapp/packages.d" })
local ops = pkg.plan("host", { "curl", "jq" }, {
  curl = { installed = true,  version = "7.88.0", available = "8.0.0" },
  jq   = { installed = false },
}, catalog.entries)
-- ops = [
--   { op="upgrade", id="curl", method="apt", from="7.88.0", to="8.0.0" },
--   { op="install", id="jq",   method="apt", target_version=nil },
-- ]
```

---

### Caller responsibilities

These pieces stay outside the framework because they're product-specific:
audit-event emission, distributed locking, per-run log rotation, and the
desired-state file. Callers compose the building blocks (`pkg.catalog`,
`pkg.templates`, `pkg.target`, `pkg.version`, `pkg.method.*`, `pkg.release`,
`pkg.plan`, `pkg.query_all`, `pkg.apply`) into their own reconcile loop.
