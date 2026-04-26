---
category: Infrastructure
---

## assay.apt

Debian/Ubuntu apt package index reader. Fetches a `Packages` index from any apt-style HTTP
repository, transparently decompressing `.gz` / `.xz` / `.zst` variants, parses the RFC 822-style
stanzas, and exposes a per-package view with versions sorted newest-first using Debian version
comparison.

### Functions

- `apt.packages(opts)` → `idx` — Fetch and parse a `Packages` index.
  - `opts.base_url` — Repository root, e.g. `https://pkgs.tailscale.com/stable/ubuntu`.
  - `opts.dist` — Distribution suite, e.g. `noble`.
  - `opts.component` — Component, e.g. `main`.
  - `opts.arch` — Architecture, e.g. `amd64`.

The module tries `Packages.gz`, `Packages.xz`, `Packages.zst`, then plain `Packages` in that order
until one returns 200.

### Index methods

- `idx:find(name)` → `pkg | nil` — Look up a package by name.

### Package fields

- `pkg.name` — Package name.
- `pkg.version` — Newest version (Debian-sorted).
- `pkg.versions` — All versions, sorted newest first.
- `pkg.architecture`, `pkg.depends`, `pkg.section`, `pkg.description`, `pkg.filename`, `pkg.sha256`,
  `pkg.size` — Fields from the newest stanza.

### Example

```lua
local apt = require("assay.apt")

local idx = apt.packages({
  base_url  = "https://pkgs.tailscale.com/stable/ubuntu",
  dist      = "noble",
  component = "main",
  arch      = "amd64",
})

local pkg = idx:find("tailscale")
print(pkg.version)        -- e.g. "1.84.3"
print(pkg.versions[1])    -- newest
```
