---
category: Builtins
---

## compress

Decompression primitives. Each function takes raw bytes (Lua strings are byte buffers) and returns
the decompressed bytes as a Lua string. Useful for fetching `.gz` / `.xz` / `.zst` payloads over
HTTP and feeding them to a parser without shelling out.

### Functions

- `compress.gunzip(bytes)` → `string` — Decompress gzip-encoded bytes.
- `compress.unxz(bytes)` → `string` — Decompress xz/LZMA2-encoded bytes.
- `compress.unzstd(bytes)` → `string` — Decompress zstd-encoded bytes.

Each raises a runtime error tagged with the function name (e.g.
`compress.gunzip: invalid gzip header`) when the input is not a valid stream.

- `compress.untar(archive_path, dest_path, opts)` → `integer` — Extract a single member from a tar
  archive to disk (v0.15.5+). Writes via temp file + atomic rename.
  - `archive_path` (string): path to the tar archive
  - `dest_path` (string): destination file path; parent dirs are created if needed
  - `opts` (table, required):
    - `member` (string, **required**): path of the member inside the archive to extract
    - `compression` (string, optional): `"gz"` | `"xz"` | `"zst"` | `"none"`. If omitted,
      auto-detected from the archive path extension.
  - Returns bytes written as an integer
  - Raises if `opts.member` is not found in the archive, or on IO failure
  ```lua
  local bytes = compress.untar(
    "/tmp/release.tar.gz",
    "/usr/local/bin/myapp",
    { member = "myapp/bin/myapp" }
  )
  ```

### Example

```lua
local resp = http.get("https://pkgs.example.com/dists/noble/main/binary-amd64/Packages.gz")
local text = compress.gunzip(resp.body)
print(text:sub(1, 200))
```
