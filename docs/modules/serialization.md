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

- `yaml.parse(str, opts?)` → table — Parse YAML string to Lua table
- `yaml.parse_all(str, opts?)` → table[] — Parse a YAML stream into a Lua array, skipping empty
  documents
- `yaml.encode(table)` → string — Encode Lua table to YAML string

### Tagged nodes

A local or custom tag — `!reference`, `!custom`, `!Ref`, anything the YAML core schema does not
resolve — becomes a one-key table keyed by the tag text, bang included, with the payload converted
recursively. A scalar, a sequence and a mapping all wrap the same way.

```lua
local data = yaml.parse("job:\n  script:\n    - !reference [.anchor, script]\n")
data.job.script[1]["!reference"] -- { ".anchor", "script" }
```

Standard tags resolve to their scalar meaning rather than being wrapped, in both modes: `!!str 1` is
the string `"1"`, `!!int "5"` is `5`, and `!!bool`, `!!float` and `!!null` behave the same, as do
the `tag:yaml.org,2002:` long forms. `!!binary` and `!!timestamp` yield the scalar text as written —
neither is decoded or parsed.

`opts.tags` picks the shape:

- `"wrap"` (default) — the one-key table above, so a script can act on the tag or on its payload
- `"strip"` — the payload alone, as if the node carried no tag

```lua
yaml.parse("a: !custom {x: 1}\n") -- { a = { ["!custom"] = { x = 1 } } }
yaml.parse("a: !custom {x: 1}\n", { tags = "strip" }) -- { a = { x = 1 } }
```

Any other value for `tags` is a runtime error. `yaml.encode` is the inverse of `"wrap"`: a one-key
table whose sole key starts with `!` encodes back to a tagged node, so a wrapped parse round-trips.

```lua
yaml.encode({ ["!reference"] = { ".anchor", "script" } }) -- "!reference\n- '.anchor'\n- script\n"
```

## toml

TOML serialization. No `require()` needed.

- `toml.parse(str)` → table — Parse TOML string to Lua table
- `toml.encode(table)` → string — Encode Lua table to TOML string
