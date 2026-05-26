---
category: Builtins
---

## json

JSON serialization. No `require()` needed.

- `json.parse(str)` → table — Parse JSON string to Lua table
- `json.encode(value)` → string — Encode Lua value to JSON string
- `json.array(t?)` → table — Tag a table to encode as a JSON array (defaults to a fresh empty table
  when called with no argument)
- `json.object(t?)` → table — Tag a table to encode as a JSON object (defaults to a fresh empty
  table when called with no argument)

### Empty tables

Lua has one composite type covering both arrays and objects, so the encoder has to pick a shape for
`{}`. The default is **object** — `json.encode({})` returns `"{}"`. To express an empty JSON array,
use `json.array({})` (or just `json.array()`) which tags the table via a `__jsontype = "array"`
metatable marker that the encoder honours regardless of contents.

```lua
json.encode({})                 -- "{}"
json.encode(json.array({}))     -- "[]"
json.encode(json.array({1,2}))  -- "[1,2]"
json.encode(json.object({"a"})) -- '{"1":"a"}'  (object with stringified key)
```

For non-empty tables the heuristic still applies: contiguous `1..N` integer keys encode as arrays,
anything else as objects. The helpers only matter when you need to override that.

## yaml

YAML serialization. No `require()` needed.

- `yaml.parse(str)` → table — Parse YAML string to Lua table
- `yaml.parse_all(str)` → table[] — Parse a YAML stream into a Lua array, skipping empty documents
- `yaml.encode(table)` → string — Encode Lua table to YAML string

## toml

TOML serialization. No `require()` needed.

- `toml.parse(str)` → table — Parse TOML string to Lua table
- `toml.encode(table)` → string — Encode Lua table to TOML string
