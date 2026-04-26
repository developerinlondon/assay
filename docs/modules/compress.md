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

### Example

```lua
local resp = http.get("https://pkgs.example.com/dists/noble/main/binary-amd64/Packages.gz")
local text = compress.gunzip(resp.body)
print(text:sub(1, 200))
```
