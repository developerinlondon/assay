# `libs/` — non-embedded Lua libraries

Lua libraries that ship alongside the `assay` binary but are **not** baked into it. Each
`libs/<name>/` is an independently-versioned tree distributed as a tarball (see `release.yml`) and
installed into `<lib-dir>/<name>/` by `assay install`.

This directory is the workspace home for those libraries. The first inhabitant is `hostops` (ported
from `knowhere0426`); see plan `.claude/plans/21-libs-folder-and-install.md`.

## Layout

```
libs/<name>/
├── mount.lua        entry point: M.mount(routes, opts)
├── …                lib-specific subdirectories
├── tests-lua/       smoke + per-page tests run under `assay`
├── README.md
└── VERSION          per-lib semver
```

## Three classes of platform code

| Class      | Where                  | Shipping mechanism                                  |
| ---------- | ---------------------- | --------------------------------------------------- |
| stdlib     | `crates/assay/stdlib/` | embedded in the `assay` binary at compile time      |
| libs       | `libs/<name>/`         | tarball alongside the binary; loaded via `LUA_PATH` |
| extensions | `crates/<name>/`       | separate compiled binary (e.g. `assay-engine`)      |
