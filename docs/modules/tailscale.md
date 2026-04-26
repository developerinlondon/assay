## assay.tailscale

Tailscale REST API client. OAuth2 `client_credentials` flow with cached bearer tokens, mint
short-lived auth keys, list/find devices, manage device key expiry, set tags, authorize, delete, and
ACL preview.

```lua
local tailscale = require("assay.tailscale")

-- env: TS_CLIENT_ID / TS_CLIENT_SECRET
local ts = tailscale.client()

-- or explicit
local ts = tailscale.client({
  client_id     = "...",
  client_secret = "...",
  tailnet       = "-",                            -- default "-"
  base_url      = "https://api.tailscale.com",   -- override for tests
  scope         = "all:write",                    -- default
})
```

The token is fetched lazily on first call, cached in the client closure, and refreshed automatically
when `os.time() >= expires_at - 30` (30s skew margin). Every authed call sends
`Authorization: Bearer <token>`.

### Auth keys

- `ts:mint_key(opts)` -> `key` — `POST /tailnet/{tailnet}/keys`. Builds the nested
  `capabilities.devices.create` payload from flat options.

```lua
local key = ts:mint_key({
  reusable       = false,
  ephemeral      = false,
  preauthorized  = true,
  tags           = { "tag:server" },
  expiry_seconds = 600,
  description    = "ansible mint for hostname-x",
})
```

### Devices

- `ts:list_devices()` -> `[device]` — `GET /tailnet/{tailnet}/devices`.
- `ts:find_device({ hostname = "x" })` -> `device|nil` — Match against `device.hostname`, fall back
  to `device.name` (also matches the `"<host>.<tailnet>..."` prefix). Returns `nil` if no match.
- `ts:get_device(id)` -> `device` — `GET /device/{id}`.

### Per-device operations

- `ts:set_key_expiry(id, { disabled = bool })` -> `"changed"|"unchanged"` — Idempotent: `GET`s the
  device first, compares `keyExpiryDisabled` to the desired value, only `POST`s `/device/{id}/key`
  if it differs.
- `ts:authorize_device(id)` — `POST /device/{id}/authorized` with `{ authorized = true }`.
- `ts:set_device_tags(id, { "tag:foo" })` — `POST /device/{id}/tags`.
- `ts:delete_device(id)` — `DELETE /device/{id}`.

### ACL preview

- `ts:acl_test(opts)` (or `tailscale.acl_test(ts, opts)`) — `POST
  /tailnet/{tailnet}/acl/preview`
  for CI ACL diffing.

### Errors

Every HTTP non-2xx (including the OAuth token exchange) raises `tailscale.<fn>: <reason>`; nothing
silently returns `nil` on a network or auth failure.
