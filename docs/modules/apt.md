---
category: Builtins
---

## apt

apt-get / dpkg-query wrapper builtin. No `require()` needed — `apt` is a global table. Linux
(Debian/Ubuntu) only. Introduced in v0.16.0.

> **Namespace note:** This documents the `apt.*` builtin global (apt-get/dpkg-query wrapper).
> For the `assay.apt` Lua stdlib (apt-repo `Packages`-index reader), see
> [`apt_index.md`](apt_index.md).

Mutating operations (`install`, `remove`, `update`, `add_source`) require root. All functions are
async at the Rust level; mlua drives them as Lua coroutines so callers write straight-line code.

### Query / inspect

- `apt.query(name)` → `{installed=bool, version=string|nil}` — Check whether a single package is
  installed. Shells out to `dpkg-query`; returns `{installed=false, version=nil}` if the package
  is unknown.

- `apt.list_installed()` → `{[name] = {installed=bool, version=string}}` — Full dpkg-query
  snapshot of all known packages, keyed by package name.

- `apt.list_upgradable()` → `[{name, current, candidate, suite}]` — Array of packages with a
  candidate upgrade available (parsed from `apt list --upgradable -a`).

### Source management

- `apt.add_source(opts)` → `{changed=bool, list_path=string, key_path=string}` — Idempotently
  write an apt source `.list` file and its GPG keyring. Safe to call on every reconcile; only
  writes if content differs.
  - `opts.id` (string, required): identifier matching `[a-z0-9-]+`; used as the file stem
  - `opts.source_list` (string, required): single-line `.list` content
  - `opts.key_path` (string, required): local path to the GPG `.gpg` keyring to install
  - `opts._sources_dir` / `opts._keyrings_dir` (string, optional): override defaults
    `/etc/apt/sources.list.d` / `/usr/share/keyrings` (for testing)

### Mutating apt-get operations

All three functions return `{status=integer, stdout=string, stderr=string, timed_out=boolean}`,
matching the `shell.exec` result shape. `timed_out` is always `false` (timeout not yet
implemented for apt — apt-get's own behaviour applies).

- `apt.update()` — Run `apt-get update`.
- `apt.install(opts)` — Run `apt-get install -y --no-install-recommends`.
  - `opts.names` (string[], required): package names to install
  - `opts.only_upgrade` (bool, optional): pass `--only-upgrade` (skip if not already installed)
- `apt.remove(opts)` — Run `apt-get remove -y`.
  - `opts.names` (string[], required): package names to remove

### Test-only helpers

These are exported for unit tests and not intended for production use:

- `apt._parse_dpkg_lines(text)` → `{[name] = {installed, version}}` — Parse raw `dpkg-query
  -W -f='${Package}\t${Version}\t${Status}\n'` output.
- `apt._parse_upgradable_lines(text)` → `[{name, current, candidate, suite}]` — Parse raw
  `apt list --upgradable -a` output.

### Example

```lua
-- Check before installing
local q = apt.query("curl")
if not q.installed then
  apt.add_source({
    id          = "mycorp",
    source_list = "deb [signed-by=/usr/share/keyrings/mycorp.gpg] https://pkgs.mycorp.com/debian stable main",
    key_path    = "/opt/mycorp/mycorp.gpg",
  })
  apt.update()
  local r = apt.install({ names = { "curl", "jq" } })
  if r.status ~= 0 then
    error("apt install failed:\n" .. r.stderr)
  end
end

-- Check for pending upgrades
for _, pkg in ipairs(apt.list_upgradable()) do
  print(pkg.name, pkg.current, "->", pkg.candidate)
end
```
