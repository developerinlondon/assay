---
category: Text, URLs & Versions
---

## assay.url

Pure-Lua URL helpers. RFC 3986 percent-encoding plus a deterministic
`application/x-www-form-urlencoded` body builder.

```lua
local url = require("assay.url")
```

- `url.encode(s)` -> `string` — RFC 3986 percent-encode. Letters, digits and `-_.~` pass through;
  every other byte becomes `%XX`. Space encodes as `%20`, not `+`.
- `url.encode_form(t)` -> `string` — Build a form body from a flat `{ key = value, ... }` table.
  Keys and values are percent-encoded, joined with `&`. Keys are sorted for determinism. Numbers are
  stringified via `tostring`; booleans become `"true"` / `"false"`.
- `url.decode(s)` -> `string` — Inverse of `encode`. Decodes `%XX` to bytes and also turns `+` into
  a space (form-decode convention).

### Why it exists

OAuth2 `client_credentials` token bodies must be `application/x-www-form-urlencoded`.
Hand-concatenating the body silently breaks the day a secret rotates to one containing `&`, `=`,
`+`, or `%`:

```lua
-- Safe: secret rotates to "a&b=c+d%e" without 401-ing in production.
local body = url.encode_form({
  grant_type    = "client_credentials",
  client_id     = env.get("CLIENT_ID"),
  client_secret = env.get("CLIENT_SECRET"),
  scope         = "all:write",
})
local resp = http.post(token_url, body, {
  headers = { ["Content-Type"] = "application/x-www-form-urlencoded" },
})
```
