## crypto

Cryptography utilities. No `require()` needed.

- `crypto.jwt_sign(claims, key, alg, opts?)` → string — Sign JWT token
  - `claims`: table with `{iss, sub, exp, ...}` — standard JWT claims
  - `key`: string — signing key (secret or PEM private key)
  - `alg`: `"HS256"` | `"HS384"` | `"HS512"` | `"RS256"` | `"RS384"` | `"RS512"`
  - `opts`: `{kid = "key-id"}` — optional key ID header
- `crypto.jwt_decode(token)` → `{header, claims}` — Decode a JWT WITHOUT verifying its signature
  - Returns `header` and `claims` parsed from the base64url segments
  - Use when the JWT travels through a trusted channel (your own session cookie over TLS) and you just need to read the claims
  - For untrusted JWTs, verify the signature with a JWKS-aware verifier instead
- `crypto.hash(str, alg)` → string — Hash string (hex output)
  - `alg`: `"sha256"` | `"sha384"` | `"sha512"` | `"md5"`
- `crypto.hmac(key, data, alg?, raw?)` → string — HMAC signature
  - `alg`: `"sha256"` (default) | `"sha384"` | `"sha512"`
  - `raw`: `true` for binary output, `false` (default) for hex
- `crypto.random(len)` → string — Secure random hex string of `len` bytes

## base64

Base64 encoding. No `require()` needed.

- `base64.encode(str)` → string — Base64 encode
- `base64.decode(str)` → string — Base64 decode
