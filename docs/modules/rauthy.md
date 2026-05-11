---
category: Security & Identity
---

## assay.rauthy

Rauthy IdP admin API client. OAuth2 client reconciliation, secret rotation, discovery, and health.
Client: `rauthy.client(url, api_key)`.

The `api_key` argument is the `<name>$<secret>` form Rauthy expects in the
`Authorization: API-Key …` header (see Rauthy bootstrap docs for how that key is minted).

### System / Health (`c.sys`)

- `c.sys:health()` → bool — Hit `/health`; returns `true` on 2xx
- `c.sys:wait_healthy(timeout_secs?)` → true — Block until `/health` returns 2xx, polling every 2 s
  (default timeout 120 s). Raises if the deadline passes.

### Discovery (`c.discovery`)

- `c.discovery:config()` → `{issuer, authorization_endpoint, token_endpoint, jwks_uri, ...}` — OIDC
  discovery document.
- `c.discovery:jwks()` → `{keys}` — JSON Web Key Set (fetched from `jwks_uri`).

### Clients (`c.clients`)

- `c.clients:list()` → `[{id, name, …}]` — All OAuth2 clients.
- `c.clients:get(id)` → client|nil — Read a single client; returns `nil` on 404.
- `c.clients:create(payload)` → nil — `POST` a `NewClientRequest` subset then `PUT` the full
  `UpdateClientRequest`. Two-call shape matches Rauthy's typed API.
- `c.clients:put(id, payload)` → nil — In-place update without rotating the client_secret.
- `c.clients:delete(id)` → nil — Delete a client; 404 is treated as success.
- `c.clients:rebuild(payload)` → nil — DELETE + create. Sidesteps a Rauthy 0.35 cache quirk where
  `challenges` set via `PUT` after a subset `POST` reads back correctly via `GET` but stays
  invisible to the OIDC handler at login (`self.challenge` cached as `None`). For confidential
  clients this rotates the secret as a side effect.
- `c.clients:rotate_secret(id)` → string — Regenerate and return a fresh client_secret.

### Reconcile (`c.clients:reconcile(payload)`)

Idempotent reconcile. Decision tree:

| Current state                                                | Action           | Returns                                                         |
| ------------------------------------------------------------ | ---------------- | --------------------------------------------------------------- |
| 404                                                          | create + rotate  | `{action="create", secret=string?}`                             |
| Exists, `challenges` declared in payload but missing in live | rebuild + rotate | `{action="rebuild", reason="challenges-drift", secret=string?}` |
| Exists, any other field drifts                               | put-only         | `{action="put", drift_on=string}`                               |
| Exists, no drift                                             | noop             | `{action="noop"}`                                               |

`secret` is present iff a rotation happened (only on `create` / `rebuild`, only for
`confidential =
true` clients). Callers should write it to a Kubernetes Secret (or wherever
consumers read it from) the same run. On `put` and `noop`, the existing managed secret stays valid —
do not overwrite it.

The `challenges`-drift rebuild path exists because of an upstream Rauthy quirk; reconcile collapses
to plain `put` / `noop` for all other drift, preserving secrets across reconciles.

### Client presets (`rauthy.client_presets`)

Ready-to-use payloads for common consumers. Each preset bakes in the OIDC verifier quirks of its
consumer so a Rauthy-fronted deployment doesn't have to rediscover them via failure logs.

- `rauthy.client_presets.openbao({ host, id?, name? })` → payload — OpenBao / Vault confidential
  client. Forces `id_token_alg = RS256` (upstream `go-oidc` rejects EdDSA with
  `unsupported signing algorithm`) and `challenges = {"S256"}` (OpenBao sends `code_challenge` even
  for confidential clients per OAuth 2.1 default; Rauthy rejects PKCE flows whose client doesn't
  declare challenges). Redirect URIs cover both the UI callback and the `bao login -method=oidc`
  device-login loopback.
- `rauthy.client_presets.argocd({ host, id?, name? })` → payload — ArgoCD PKCE-public client (no
  shared secret). Uses EdDSA. Redirect URIs cover both the browser flow and the `argocd login --sso`
  device-login loopback.

### Example

```lua
local rauthy = require("assay.rauthy")

local c = rauthy.client("http://rauthy:8080/auth/v1", "ansible$" .. os.getenv("BOOTSTRAP_API_KEY"))
c.sys:wait_healthy()

-- Use a preset; override id/name if you ship multiple OpenBao instances.
local payload = rauthy.client_presets.openbao({ host = "openbao.fcar.ai" })
local r = c.clients:reconcile(payload)

if r.action == "create" or r.action == "rebuild" then
  -- Rotated; write r.secret into a k8s Secret consumer apps mount.
  print("rotated", payload.id, "→ secret needs publishing")
else
  print(payload.id, r.action, r.drift_on or "")
end
```
