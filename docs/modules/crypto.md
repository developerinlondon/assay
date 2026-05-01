---
category: Builtins
---

## crypto

Cryptography utilities. No `require()` needed.

- `crypto.jwt_sign(claims, key, alg, opts?)` → string — Sign JWT token
  - `claims`: table with `{iss, sub, exp, ...}` — standard JWT claims
  - `key`: string — signing key (secret or PEM private key)
  - `alg`: `"HS256"` | `"HS384"` | `"HS512"` | `"RS256"` | `"RS384"` | `"RS512"`
  - `opts`: `{kid = "key-id"}` — optional key ID header
- `crypto.jwt_decode(token)` → `{header, claims}` — Decode a JWT WITHOUT verifying its signature
  - Returns `header` and `claims` parsed from the base64url segments
  - Use when the JWT travels through a trusted channel (your own session cookie over TLS) and you
    just need to read the claims
  - For untrusted JWTs, use `crypto.jwt_verify` instead
- `crypto.jwt_verify(token, key, opts?)` → `{header, claims}` — Verify signature and validate claims
  - `key`: PEM-encoded RSA public key string, OR a JWKS table `{ keys = { ... } }`
    - PEM path uses `opts.algorithm` (default `"RS256"`)
    - JWKS path dispatches on the JWT header's `kid` and uses the matching JWK's `alg`
  - `opts`:
    `{algorithm = "RS256"|"RS384"|"RS512", audience = "x" | {"x","y"}, issuer = "x" | {"x","y"}, leeway = 0, validate_exp = true, validate_nbf = false, required_claims = {"exp"}}`
  - Returns the same shape as `jwt_decode`; raises on signature mismatch, expired token, claim
    mismatch, malformed token, or missing JWK
  - Pair with `assay.ory.hydra` `c.discovery:jwks()` to fetch the issuer's JWKS table at boot
- `crypto.hash(str, alg)` → string — Hash string (hex output)
  - `alg`: `"sha256"` | `"sha384"` | `"sha512"` | `"md5"`
- `crypto.hmac(key, data, alg?, raw?)` → string — HMAC signature
  - `alg`: `"sha256"` (default) | `"sha384"` | `"sha512"`
  - `raw`: `true` for binary output, `false` (default) for hex
- `crypto.random(len)` → string — Secure random hex string of `len` bytes
- `crypto.hash_file(path, algo?)` → string — Hash a file on disk, returning lowercase hex
  (v0.15.5+).
  - `path` (string): file to hash
  - `algo` (string, optional): algorithm — same set as `crypto.hash`: `"sha224"` | `"sha256"` |
    `"sha384"` | `"sha512"` | `"sha3-224"` | `"sha3-256"` | `"sha3-384"` | `"sha3-512"`. Defaults to
    `"sha256"`.
  - Streams the file in chunks; memory usage does not scale with file size
  ```lua
  local digest = crypto.hash_file("/tmp/release.tar.gz")            -- sha256, default
  local digest = crypto.hash_file("/tmp/release.tar.gz", "sha512")  -- explicit algo
  ```

## base64

Base64 encoding. No `require()` needed.

- `base64.encode(str)` → string — Base64 encode
- `base64.decode(str)` → string — Base64 decode
