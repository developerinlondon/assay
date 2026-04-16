## fs

Filesystem operations. No `require()` needed.

### Reading & Writing

- `fs.read(path)` → string — Read entire file as UTF-8 text
- `fs.read_bytes(path)` → string — Read entire file as raw bytes (binary-safe)
- `fs.write(path, str)` → nil — Write UTF-8 string to file (creates parent dirs)
- `fs.write_bytes(path, data)` → nil — Write raw bytes to file (binary-safe, creates parent dirs)

### File Operations

- `fs.remove(path)` → nil — Remove file or directory (recursive for dirs)
- `fs.copy(src, dst)` → bytes_copied — Copy file
- `fs.rename(src, dst)` → nil — Move/rename file or directory
- `fs.chmod(path, mode)` → nil — Set file permissions (e.g. `"755"`)

### Directory Operations

- `fs.mkdir(path)` → nil — Create directory (and parents)
- `fs.list(path)` → `[{name, type}]` — List directory entries. `type`: `"file"`, `"directory"`,
  `"symlink"`
- `fs.readdir(path, opts?)` → `[{name, path, type}]` — Recursive directory listing. `opts`:
  `{depth = N}` for max recursion
- `fs.glob(pattern)` → `[path]` — Glob pattern matching, returns array of path strings
- `fs.tempdir()` → path — Create a temporary directory

### Metadata

- `fs.stat(path)` → `{size, type, modified, created, permissions}` — File metadata
- `fs.exists(path)` → bool — Check if path exists

### Binary I/O

`fs.read_bytes` and `fs.write_bytes` handle files with arbitrary byte content (images, WASM,
protobuf, compressed data). Lua strings can hold any bytes, so the returned value works with
`http.serve()` response bodies, `base64.encode()`, or any other builtin that accepts strings.

```lua
-- Copy a binary file
local data = fs.read_bytes("image.png")
fs.write_bytes("copy.png", data)

-- Serve binary files via http.serve()
http.serve(8080, {
  GET = {
    ["/*"] = function(req)
      return { status = 200, body = fs.read_bytes("static" .. req.path) }
    end,
  },
})
```
