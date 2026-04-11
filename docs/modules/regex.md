## regex

Regular expressions (Rust regex syntax). No `require()` needed.

- `regex.match(pattern, str)` → bool — Test if pattern matches string
- `regex.find(pattern, str)` → string|nil — Find first match
- `regex.find_all(pattern, str)` → [string] — Find all matches
- `regex.replace(pattern, str, replacement)` → string — Replace all matches
