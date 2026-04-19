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

### Line Iteration & In-Place Editing

- `fs.lines(path)` → iterator — Streaming line reader. Designed for
  `for line in fs.lines(path) do … end`. Reads from a buffered reader
  so multi-GB files never land in memory; each line is returned with
  the trailing `\n` (or `\r\n`) stripped. Equivalent to `while read
  line; do …; done < file` in bash.
- `fs.sub_in_file(path, pattern, repl)` → count — In-place
  search-and-replace; equivalent to `sed -i 's/pattern/repl/g' path`
  but portable (no BSD-vs-GNU `sed -i` split). `pattern` uses Lua
  patterns (same as `string.gsub`); `repl` can be a replacement
  string with `%0`-`%9` backreferences OR a function per
  `string.gsub`. Writes only when at least one match is found, so
  repeated calls on an already-substituted file are no-ops on disk.

```lua
-- grep-equivalent: count lines matching a pattern
local n = 0
for line in fs.lines("/var/log/app.log") do
  if line:match("ERROR") then n = n + 1 end
end

-- sed -i equivalent: bump a version string across every file
for _, p in ipairs(fs.glob("apps/**/values.yaml")) do
  fs.sub_in_file(p, "image: foo:v%d+%.%d+%.%d+", "image: foo:v1.2.3")
end
```

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
