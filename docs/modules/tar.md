---
category: Filesystem & Archives
---

## tar

Tar archive creation, extraction, and listing. Supports plain tar and gzip-compressed archives. No
`require()` needed — available as a global builtin.

### Functions

- `tar.create(output, files, opts?)` → true — Create a tar archive. `files` is a table of
  `{path = content}`. `opts.gzip` (default `true`) controls gzip compression. Output path
  determines format (`.tar.gz` enables gzip).

- `tar.extract(archive, dest)` → true — Extract a tar or tar.gz archive to `dest`. Auto-detects
  gzip compression from the filename extension.

- `tar.list(archive)` → `[string]` — List all file paths in a tar or tar.gz archive.

Example:

```lua
-- Create a tar.gz archive
tar.create("bundle.tar.gz", {
  ["app/main.lua"] = [[print("hello")]],
  ["config.toml"] = '[server]\nport = 8080\n',
}, {gzip = true})

-- Extract
tar.extract("bundle.tar.gz", "/tmp/extracted")

-- List contents
local paths = tar.list("bundle.tar.gz")
-- → {"app/main.lua", "config.toml"}
```
