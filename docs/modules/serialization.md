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
- `yaml.encode(table, opts?)` → string — Encode Lua table to YAML string

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

A tag over an empty payload (`a: !foo`) wraps with the empty string, because a Lua table cannot hold
a nil value and the tag would otherwise be lost.

`opts.tags` on `yaml.parse` and `yaml.parse_all` picks the shape:

- `"wrap"` (default) — the one-key table above, so a script can act on the tag or on its payload
- `"strip"` — the payload alone, as if the node carried no tag

```lua
yaml.parse("a: !custom {x: 1}\n") -- { a = { ["!custom"] = { x = 1 } } }
yaml.parse("a: !custom {x: 1}\n", { tags = "strip" }) -- { a = { x = 1 } }
```

`yaml.encode` takes the same option and **defaults to `"strip"`**: a key starting with `!` is an
ordinary mapping key unless you ask for tags. Pass `{ tags = "wrap" }` to turn a one-key table whose
sole key starts with `!` back into a tagged node, which is what round-trips a wrapped parse.

```lua
yaml.encode({ ["!reference"] = { ".anchor", "script" } }) -- "'!reference':\n- '.anchor'\n- script\n"
yaml.encode({ ["!reference"] = { ".anchor", "script" } }, { tags = "wrap" })
-- "!reference\n- '.anchor'\n- script\n"
```

Any other value for `tags` is a runtime error, in either direction.

### Untagged documents

Nothing about an untagged document changed when tags were understood, and that is enforced by test.
Mapping keys arrive as the text the document wrote — `9000`, `~`, `TRUE` and `1.50` are the Lua
string keys `"9000"`, `"~"`, `"TRUE"` and `"1.50"` — a repeated key keeps its last value, anchors
and aliases expand, a `<<` merge key stays a literal key, and `.inf` / `.nan` are `nil`.

## toml

TOML serialization. No `require()` needed.

- `toml.parse(str)` → table — Parse TOML string to Lua table
- `toml.encode(table)` → string — Encode Lua table to TOML string
