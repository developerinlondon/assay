---
category: Infrastructure
---

## assay.servicenow

ServiceNow Table API client plus a small CMDB helper. Client:
`servicenow.client(url, {user="...", password="..."})` (basic) or
`servicenow.client(url, {token="..."})` (bearer / OAuth).

### Table API

- `c.table:list(table, opts?)` → `[record]` — List rows (`GET /api/now/table/<table>`). **Read.**
  `opts`: `{query, limit, offset, fields, display_value}` — `query` maps to `sysparm_query`,
  `fields` may be a string or a list of column names.
- `c.table:get(table, sys_id)` → `record`|nil — Get one row by sys_id
  (`GET /api/now/table/<table>/<sys_id>`). **Read.** Returns nil on 404.
- `c.table:create(table, body)` → `record` — Create a row (`POST /api/now/table/<table>`).
  **Mutates.**
- `c.table:update(table, sys_id, body)` → `record` — Update a row
  (`PATCH /api/now/table/<table>/<sys_id>`). **Mutates.**

### CMDB

- `c.cmdb:query(class, body)` → `[record]` — Query a CMDB class
  (`POST /api/now/cmdb/instance/<class>`). **Read by contract** — this is a lookup even though it
  uses `POST`. Because assay's read-only / approval gate classifies mutation by HTTP verb, this call
  goes through the gated `http.post` and is therefore suspended/blocked in read-only and approval
  modes despite being a read. Run it in unrestricted mode when you need it in a gated context.

### Mutation summary

`table:create` and `table:update` are the true mutations. `cmdb:query` is a read that happens to use
`POST`. All three route through the already-gated `http.post` / `http.patch` verbs.

Example:

```lua
local servicenow = require("assay.servicenow")
local c = servicenow.client("https://example.service-now.com", {
  user = env.get("SNOW_USER"),
  password = env.get("SNOW_PASSWORD"),
})

local rows = c.table:list("incident", { query = "active=true", limit = 10, fields = { "number", "state" } })
for _, r in ipairs(rows) do
  print(r.number, r.state)
end

local created = c.table:create("incident", { short_description = "disk pressure on app-1" })
c.table:update("incident", created.sys_id, { state = "6" })

local servers = c.cmdb:query("cmdb_ci_server", { sysparm_query = "nameLIKEapp" })
```
