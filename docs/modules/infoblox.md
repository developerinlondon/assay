---
category: Infrastructure
---

## assay.infoblox

Infoblox WAPI client for DNS records, networks, DHCP ranges, and grid status. Client:
`infoblox.client(url, {user="...", password="..."})` (basic auth). The WAPI version defaults to
`v2.12`; override with `{wapi_version="v2.13"}`.

### Records

- `c.records:get(type, opts?)` → `[record]`|nil — Get records of a WAPI object type such as
  `record:a`, `record:host`, `record:cname` (`GET /wapi/<ver>/<type>`). **Read.** Returns nil on
  404. `opts` keys map directly to WAPI query parameters
  (`{name = "host.example.com", ["_return_fields"] = "name,ipv4addr", ["_max_results"] = 50}`).
- `c.records:create(type, body)` → `ref` — Create a record (`POST /wapi/<ver>/<type>`). **Mutates.**
  Returns the new object reference string.
- `c.records:update(ref, body)` → `ref` — Update a record by reference (`PUT /wapi/<ver>/<ref>`).
  **Mutates.**
- `c.records:delete(ref)` → `ref` — Delete a record by reference (`DELETE /wapi/<ver>/<ref>`).
  **Mutates.**

### Networks and Ranges

- `c.network:get(opts?)` → `[network]` — Get networks (`GET /wapi/<ver>/network`). **Read.**
- `c.range:get(opts?)` → `[range]` — Get DHCP ranges (`GET /wapi/<ver>/range`). **Read.**

### Grid

- `c.grid:status()` → `[grid]` — Get grid status (`GET /wapi/<ver>/grid`). **Read.**

### Mutation summary

`records:create` / `records:update` / `records:delete` are the mutations; they route through the
already-gated `http.post` / `http.put` / `http.delete` verbs, so read-only and approval modes cover
them. The read methods use `http.get`.

### TLS / self-signed certificates

Infoblox Grid appliances typically serve WAPI over a self-signed certificate. Assay's `http` builtin
verifies TLS and does **not** expose a per-request "skip verification" option, so the Grid CA (or
the appliance certificate) must be trusted by the runtime — install it into the system trust store
of the host or container image running the script. There is intentionally no `insecure` flag on this
client.

Example:

```lua
local infoblox = require("assay.infoblox")
local c = infoblox.client("https://infoblox.example.com", {
  user = env.get("INFOBLOX_USER"),
  password = env.get("INFOBLOX_PASSWORD"),
})

local records = c.records:get("record:a", { name = "host.example.com" })
for _, r in ipairs(records) do
  print(r.name, r.ipv4addr)
end

local ref = c.records:create("record:a", { name = "new.example.com", ipv4addr = "10.0.0.9" })
c.records:delete(ref)
```
