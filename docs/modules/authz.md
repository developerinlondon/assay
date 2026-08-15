---
category: Security & Identity
tagline: In-process authorization engine — policy statements, grants-at-scope with typed bounds, ABAC conditions, deny-wins, asymmetric fail-closed
---

## assay.authz

In-process authorization engine. Policy statements, grants-at-scope with typed bounds, ABAC
conditions, deny-wins, asymmetric fail-closed. Pure computation — no I/O, no storage, no policy
expression language. The host resolves its grants and hands them over.

- `authz.engine(opts)` → `engine` — Build an engine from a vocabulary and a grant universe.
- `e:check(subjects, action, resource, opts?)` → `decision` — Decide one question.
- `e:validate(statements)` → `statements` | `nil, err` — Refuse a shape the engine could never
  evaluate.
- `e:describe()` → `descriptor` — The declared vocabulary as data, for an administration surface.
- `e:grants_for(subjects, scope_chain?)` → `grants` — What applies over a chain, for a "why" view.

### Semantics

- **Deny wins.** A deny granted at any scope in the resolved chain beats an allow from any other.
  Nothing granted denies.
- **Scope chains inherit downward only.** The host resolves the chain (root first); every grant at
  any scope in it applies, so a root grant reaches every depth while a leaf grant never leaks upward
  or sideways. A malformed entry, or a kind outside `scope_kinds`, denies outright.
- **Asymmetric fail-closed.** A condition that cannot be evaluated — unknown operator or key, wrong
  type for the key, a key the request context does not populate — withdraws an **allow** and leaves
  a **deny** standing. Both directions fail toward less access.
- **Bounds narrow allows only.** A grant's `bounds` are ANDed onto the allow statements of the
  policy it confers. ANDing them onto a deny would make it fire less often.
- **Actions derive, they do not wildcard.** A statement naming a base action covers every action
  declared to derive from it, allow and deny alike, and the expansion stays closed and enumerable.
  Resources take a single trailing `*`; actions never take one.

### Engine options

| Option                | Shape                               | Meaning                                                                                                       |
| --------------------- | ----------------------------------- | ------------------------------------------------------------------------------------------------------------- |
| `grants`              | list of grants                      | The grant universe to consider.                                                                               |
| `synthesized_grants`  | list of grants                      | Grants an app-owned synthesizer contributes (membership roles, an open-mode baseline). Unioned with `grants`. |
| `condition_keys`      | `{ [key] = { type, lowercase? } }`  | Declared keys; `type` is `string`, `number`, `date` or `ip`. Merged with the built-in `request:*` keys.       |
| `scope_kinds`         | list of strings                     | When given, a chain entry of any other kind denies.                                                           |
| `default_scope_chain` | list of scopes                      | Used when a check names none.                                                                                 |
| `actions`             | list of `{ action, derives_from? }` | The closed action registry. A cycle, a duplicate or a wildcard is an error at build time.                     |
| `action_derivation`   | `{ [child] = parent }`              | Derivation alone, for a host with no full catalogue.                                                          |

A grant is `{ subject = {kind, id}, scope = {kind, id}, statements = {...}, bounds? = {...} }`. A
statement is `{ effect = "allow"|"deny", actions = {...}, resources = {...}, conditions? = {...} }`.

### Conditions

Operators are a closed set, each valid only on its key type:

| Key type | Operators                                                                                  |
| -------- | ------------------------------------------------------------------------------------------ |
| `string` | `StringEquals`, `StringNotEquals`, `StringLike`, `StringIn`, `StringNotIn`, `StringLikeIn` |
| `number` | `NumericLessThan`, `NumericGreaterThan`                                                    |
| `date`   | `DateLessThan`, `DateGreaterThan`                                                          |
| `ip`     | `IpAddress`, `NotIpAddress`                                                                |

Scalar operators take `value`; the set operators (`StringIn`, `StringNotIn`, `StringLikeIn`) take
`values`. Carrying both, or the wrong one, is unreadable and therefore unmatchable. The engine
populates three keys itself on every check: `request:Time` (RFC 3339), `request:HourUTC` (0-23) and
`request:SourceIp` — the last only when the check carried one, so a condition on it fails closed.

### Check options

`scope_chain`, `context` (values for the declared keys), `source_ip`, `now` (RFC 3339), and `bypass`
— the host declaring that this caller skips policy entirely, so a careless broad deny cannot lock an
operator out.

The decision is `{ allowed, decision, reason, allowed_by_stored_grants }`. `reason` is one of
`allowed`, `explicit_deny`, `no_matching_grant`, `malformed_scope_chain`, `undeclared_scope_kind` or
`admin_bypass`. `allowed_by_stored_grants` tells an explicit grant apart from one a synthesizer
supplied.

```lua
local authz = require("assay.authz")

local eng = authz.engine({
  scope_kinds = { "root", "space" },
  default_scope_chain = { { kind = "root", id = "*" } },
  condition_keys = { ["app:Region"] = { type = "string" } },
  actions = {
    { action = "docs.read" },
    { action = "docs.write", derives_from = "docs.read" },
  },
  grants = {
    {
      subject = { kind = "user", id = "alice" },
      scope = { kind = "root", id = "*" },
      statements = {
        { effect = "allow", actions = { "docs.read" }, resources = { "doc:*" } },
        { effect = "deny", actions = { "docs.read" }, resources = { "doc:secret" } },
      },
      bounds = { { operator = "StringEquals", key = "app:Region", value = "eu-west" } },
    },
  },
})

local alice = { { kind = "user", id = "alice" } }
local ctx = { context = { ["app:Region"] = "eu-west" } }

assert.eq(eng:check(alice, "docs.read", "doc:42", ctx).allowed, true)
assert.eq(eng:check(alice, "docs.write", "doc:42", ctx).allowed, true) -- derives from docs.read
assert.eq(eng:check(alice, "docs.read", "doc:secret", ctx).allowed, false) -- deny wins
assert.eq(eng:check(alice, "docs.read", "doc:42").allowed, false) -- bound unsatisfied
```

### Conformance

The engine is decision-identical to the
[agentauthz](https://github.com/developerinlondon/agentauthz) reference library: all 149 of its
language-neutral golden fixtures are vendored under `crates/assay-authz/conformance/cases` and run
on every build, through both the pure evaluator and the composed engine.
