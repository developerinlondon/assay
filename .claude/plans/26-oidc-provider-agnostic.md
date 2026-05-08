# 26 · OIDC upstreams — provider-agnostic federation

**Status:** spec **Date:** 2026-05-08 **Branch:** `fix/oidc-upstream-registry-sync` (extends
in-flight branch) **Builds on:** [`12d`](./12d-phase-7-oidc-provider.md),
[`12c`](./12c-phase-4-6-auth-identity-zanzibar.md), and the in-branch baseline at commit `29969fd`
("address review — extract helpers, fix enabled, dedup") **Closes:**
[#133](https://github.com/developerinlondon/assay/issues/133) (registry sync) plus the
agnostic-federation work this spec defines

## Baseline already on the branch (as of `29969fd`)

The first round of review feedback was addressed before this spec landed. To avoid double-spec'ing
work that already exists on the branch:

- `oidc::DEFAULT_UPSTREAM_SCOPES` constant (single source of truth for the hardcoded scope set this
  plan replaces).
- `oidc_provider::upstream_callback_url(public_url, slug)` — trailing-slash safe.
- `oidc_provider::sync_upstream_to_registry(registry, row, public_url, default_scopes)` — handles
  enabled/disabled branching (`add` if true, `remove` if false), discovery, and warn-on-failure.
  Used by both boot paths and admin upsert.
- Admin upsert performs discovery in a `tokio::spawn` so the admin HTTP response is never blocked by
  a slow upstream.
- The 3-way duplication called out in the first review is gone; both `build_auth_ctx_*` functions
  call the helper with `.await`.

This plan extends those helpers rather than introducing parallel ones.

## Why this exists

The fix for #133 (commit `96e9a94` on `fix/oidc-upstream-registry-sync`) closes the DB↔registry gap
that prevented admin-configured OIDC upstreams from reaching the federation handlers. That part is
provider-neutral.

But the federation path it unlocks (even after `29969fd`) is **only fully functional for
Google-shaped IdPs**:

- Scopes are still hardcoded — `oidc::DEFAULT_UPSTREAM_SCOPES = ["openid", "email", "profile"]` is
  the single source after the dedup, but `auth.upstream_providers` still has no per-provider scopes
  column. Every upstream gets the same set.
- `federation::start_upstream_login` plumbs no per-provider authorize-URL params (`prompt`,
  `login_hint`, `domain_hint`, `hd`, `acr_values`, ...). Microsoft Entra's `domain_hint` and
  Google's hosted-domain (`hd`) restriction are unreachable.
- The callback handler skips the RFC 9207 `iss` parameter entirely — IdP-mix-up attacks are
  unmitigated once two upstreams are registered.
- The OIDC `state` value is not bound to the originating browser session — the classic OIDC
  login-CSRF attack ("attacker's code+state in victim's browser logs victim in as attacker") is
  reachable.
- The OIDC discovery HTTP client has no timeout and no SSRF posture beyond redirect-disable; an
  attacker (or a typo) with admin-CRUD access can store an issuer that triggers arbitrary outbound
  requests at boot or admin-spawn time.

The repo's docstrings advertise "Google/Apple/GitHub/upstream" (`lib.rs:13`,
`oidc_provider/mod.rs:17`) but only Google works end-to-end. This plan tightens the federation
surface to **standard OIDC IdPs in general** — Google, Microsoft Entra, Okta, Auth0, Keycloak,
generic OIDC. Apple Sign-In and non-OIDC OAuth (GitHub-style) require structural changes
(POST/form_post callback shape, ES256-JWT secret model, adapter abstraction) and are explicit
non-goals; they get dedicated follow-up plans.

## Goal

Ship a multi-IdP-capable OIDC federation surface on `fix/oidc-upstream-registry-sync`:

1. **Per-provider config** — `scopes` and `auth_params` columns on `auth.upstream_providers`,
   plumbed through `sync_upstream_to_registry` to the authorize URL. Replaces the
   `DEFAULT_UPSTREAM_SCOPES` constant baseline as the source of truth (constant survives only as the
   fallback when a row's scopes column is empty).
2. **RFC 9207 `iss` callback verification** — lenient mode (warn-on-missing, reject-on-mismatch)
   until provider coverage stabilises.
3. **Login-CSRF binding** — cookie-pinned binding token tied to the in-flight state row, blocking
   the cross-session code+state replay attack.
4. **Discovery hardening** — explicit connect/read timeouts, issuer scheme/host validation,
   private-IP rejection at both literal-host and DNS-resolved layers.

## Non-goals

- **Apple Sign-In.** Requires `response_mode=form_post` POST callback, ES256-JWT client_secret
  generated from `.p8`, and name-from-form-body extraction. Adapter-level work — separate plan.
- **Non-OIDC OAuth (GitHub web login etc.).** GitHub web OAuth has no
  `/.well-known/openid-configuration`; supporting it needs an adapter abstraction over
  `OidcRegistry` / `OidcClient`. Separate plan.
- **SAML / WS-Fed enterprise IdP federation.** Different protocol stack — would be a parallel
  `SamlRegistry` next to `OidcRegistry`.
- **`client_secret` envelope-encryption via `sysops-vault`.** CWE-256 fix tracked separately; the
  vault crate landed in `21242bc` on `main` and the integration is straightforward but out of scope
  here.
- **Refresh-token storage for upstreams.** `offline_access` becomes plumbable via `auth_params`
  after this plan, but there is no upstream-refresh-token table yet. Follow-up.
- **Admin UI changes** in the assay-dashboard / sysops Lua pages. Backend-only. UI consumes the new
  fields via the existing admin JSON API.
- **OIDC dynamic client registration (RFC 7591)** for the engine acting as a relying party at
  multiple IdPs. Out of scope.

## Threat model focus

This plan closes three named attacks reachable from `main` once a second upstream is registered:

1. **Login-CSRF via cross-session state replay.** Attacker logs in to upstream as themselves,
   intercepts their own callback URL pre-redirect, sends
   `https://victim/auth/oidc/upstream/<slug>/callback?code=X&state=Y` to the victim. Without a
   per-session binding, the victim's browser arrives at a valid in-flight state row, completes the
   upstream code-exchange, and gets a session cookie minted for the attacker's account. Mitigation:
   cookie-bound `binding_token`.
2. **IdP-mix-up.** With multiple upstreams registered, attacker tricks the engine into exchanging a
   code from one IdP at another IdP's token endpoint. Mitigation: RFC 9207 `iss` callback param
   verified against `client.provider().issuer`.
3. **Boot-time SSRF / DoS via stored issuer URL.** Anyone with admin-CRUD (or DB) write access can
   store an `issuer` whose discovery endpoint hangs forever or points at internal infra. Mitigation:
   explicit timeouts on the discovery HTTP client, scheme/host validation, private-IP rejection at
   admin upsert and at boot load.

## Schema migrations

### `auth.upstream_providers` (sqlite + pg, mirrored migrations)

| Column        | Type          | Default                  | Notes                                                                                                                             |
| ------------- | ------------- | ------------------------ | --------------------------------------------------------------------------------------------------------------------------------- |
| `scopes`      | TEXT NOT NULL | `'openid email profile'` | Space-separated, OAuth-standard. NULL or empty treated as default. `openid` always re-added at runtime in case an admin omits it. |
| `auth_params` | TEXT NOT NULL | `'{}'`                   | JSON-as-text. Validated at app layer against the `auth_params` whitelist (see below).                                             |

Both columns nullable in the migration step itself for sqlite reasons, then
defaulted-and-NOT-NULL-set with a follow-up `UPDATE` to fill nulls. Pg single-statement.

### `auth.oidc_upstream_states` (the in-flight federation state table)

| Column         | Type                       | Notes                                                                                                                                                                                                                                                                                                |
| -------------- | -------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `binding_hash` | TEXT NOT NULL DEFAULT `''` | SHA-256 hex of the cookie binding token. Empty string is the migration sentinel: rows with `binding_hash = ''` skip the binding check (compat for in-flight flows that started before deploy). New rows always populate it. Once the deploy window passes (5 min, the row TTL), no `''` rows remain. |

Skipping the check on sentinel rows is the explicit user choice over fail-closed — gives a 5-min
deploy bypass window in exchange for not breaking in-flight users. Acceptable because (a) the window
is bounded by row TTL and (b) the attack requires foreknowledge of the deploy timing plus an
in-flight victim flow.

## `auth_params` whitelist

Validated at admin upsert before the DB write; same validator runs at boot before `registry.add()`.

**Allowed keys:**

```
prompt, login_hint, domain_hint, hd, acr_values, max_age, ui_locales
```

**Allowed prefix:** any key starting with `idp_` is forwarded verbatim. Lets operators pass
IdP-specific extension params without a code change for each new key.

**Rejected keys (framework-owned):**

```
client_id, redirect_uri, scope, state, nonce, response_type,
code_challenge, code_challenge_method, request, request_uri
```

These are owned by the framework and must never be admin-overridable. A row containing any rejected
key fails admin upsert with HTTP 400 and a per-key error.

**Value rules:** strings only (no nested objects). Length ≤ 256 chars per value. URL-encoded at use
site, not at storage site.

## Issuer validation

`crates/assay-auth/src/oidc_provider/issuer_validation.rs` (new):

```rust
pub fn validate_issuer(input: &str, allow_insecure: bool) -> Result<Url, IssuerError>
```

**Rules (all must pass):**

- Parses as a `Url`.
- Scheme is `https`, OR scheme is `http` _and_ (`allow_insecure == true` _or_ host is `localhost` /
  `127.0.0.1` / `::1`). The flag comes from `[auth.oidc] allow_insecure_issuers` in engine config;
  defaults to false.
- Host is present and non-empty.
- No userinfo (`https://user:pass@…`) — OIDC discovery URLs in `tracing::warn!` would otherwise leak
  credentials.
- No fragment.
- Host does not literal-resolve to a loopback / link-local / private range (RFC 1918 + RFC 4193 +
  RFC 6598 + 169.254.0.0/16 + ::1 + fc00::/7 + fe80::/10), unless `allow_insecure == true`.

The DNS-resolved IP check (separate from literal-host) happens lazily inside
`build_oidc_http_client` via a custom `reqwest::dns::Resolve` that rejects answers in the same
private ranges, again gated by the same flag. This catches `evil.example.com → 192.168.x.x` while
still allowing operators to point at internal IdPs when they opt in.

## Login-CSRF binding

`crates/assay-auth/src/oidc_provider/binding.rs` (new):

```rust
pub fn generate() -> (String, String); // (raw_token, sha256_hex_hash)
pub fn verify(raw: &str, hash: &str) -> bool; // constant-time
```

Raw token is 32 random bytes, base64url-no-pad. Hash is hex-encoded SHA-256 of the raw bytes.
Returning the pair from a single helper prevents call sites from accidentally swapping them.

**Cookie shape (set on `/oidc/upstream/{slug}/start`'s 302 response):**

```
Set-Cookie: assay_oidc_binding=<raw_token>;
            Path=/oidc/upstream/;
            HttpOnly;
            Secure;          (omitted only when public_url scheme is http and host is localhost)
            SameSite=Lax;    (Lax — Strict would block the upstream's top-level redirect)
            Max-Age=300      (matches UPSTREAM_STATE_LIFETIME_SECS)
```

Single cookie, not per-slug. A user starting two federation flows simultaneously will have the
second clobber the first; the first tab's callback fails closed with `binding mismatch`. Documented
behaviour, not a bug.

**Verification path:**

`upstream_callback` parses the cookie, passes the raw token to `complete_upstream_login`. The
state-row lookup happens first (so a stolen `state` without a valid row still fails fast). If the
row's `binding_hash == ''` (sentinel), the binding check is skipped. Otherwise
`binding::verify(raw, &row.binding_hash)` must return true, else
`Error::Oidc("oidc state binding mismatch")` (or `"oidc state binding missing"` if no cookie). Both
paths reject before `client.complete_login` so a stolen `code` is never spent.

On any callback path (success or failure), the cookie is cleared with
`Set-Cookie: assay_oidc_binding=; Max-Age=0; Path=/oidc/upstream/`.

## RFC 9207 `iss` check

`UpstreamCallbackQuery` gains `iss: Option<String>`. Inside `complete_upstream_login`:

```
if let Some(got) = iss {
    let expected = &client.provider().issuer;
    if &got != expected {
        return Err(Error::Oidc(format!("issuer mismatch: expected {expected}, got {got}")));
    }
}
```

**Lenient mode** (chosen): missing `iss` →
`tracing::warn!("upstream {} did not return iss param", slug)` and proceed. Strict mode is a
follow-up flag once provider coverage stabilises.

## HTTP client hardening

`oidc.rs:352-357` `build_oidc_http_client` — currently:

```rust
oidc_reqwest::ClientBuilder::new()
    .redirect(oidc_reqwest::redirect::Policy::none())
    .build()
```

Becomes:

```rust
oidc_reqwest::ClientBuilder::new()
    .redirect(oidc_reqwest::redirect::Policy::none())
    .connect_timeout(Duration::from_secs(connect_timeout_secs))
    .timeout(Duration::from_secs(request_timeout_secs))
    .dns_resolver(Arc::new(PrivateRangeRejectingResolver { allow_insecure }))
    .build()
```

Defaults `connect_timeout_secs = 5`, `request_timeout_secs = 10`. Both configurable via
`[auth.oidc] discovery_connect_timeout_secs`, `discovery_request_timeout_secs`. The DNS resolver
wrapper is a thin shim over the system resolver that filters answers per the issuer-validation
private-range rules.

## File layout

### New modules

```
crates/assay-auth/src/oidc_provider/
├── auth_params.rs         whitelist validator + URL injection
├── issuer_validation.rs   validate_issuer() + private-range checks (literal + DNS)
└── binding.rs             generate() / verify() for CSRF binding token (sha256)
```

The registry-sync helper already exists at `oidc_provider::sync_upstream_to_registry` (added in
`29969fd`); this plan extends its signature rather than introducing a parallel `registry_sync.rs`.

### Modified

```
crates/assay-auth/src/oidc.rs
  - UpstreamProvider POD gains auth_params: BTreeMap<String, String>
  - build_oidc_http_client gains timeouts + DNS resolver

crates/assay-auth/src/oidc_provider/mod.rs
  - sync_upstream_to_registry: signature drops `default_scopes: &[String]` and instead reads
    row.scopes / row.auth_params from the DB row directly; falls back to DEFAULT_UPSTREAM_SCOPES
    only when row.scopes is empty
  - upstream_callback_url: unchanged

crates/assay-auth/src/oidc_provider/types.rs
  - UpstreamProvider DB row gains scopes: Vec<String>, auth_params: BTreeMap<String, String>
  - UpstreamLoginState gains binding_hash: String

crates/assay-auth/src/oidc_provider/store.rs
  - upstream + state SQL widened to read/write the new columns
  - Sqlite + Pg backends updated in lockstep

crates/assay-auth/src/oidc_provider/federation.rs
  - start_upstream_login plumbs scopes + auth_params, returns binding_token in StartedUpstreamLogin
  - complete_upstream_login takes &binding_token + Option<&iss>, runs both checks
  - UPSTREAM_STATE_LIFETIME_SECS unchanged

crates/assay-auth/src/oidc_provider/handlers.rs
  - upstream_start: sets assay_oidc_binding cookie on 302
  - upstream_callback: parses cookie, accepts iss query param, clears cookie on response

crates/assay-auth/src/oidc_provider/admin.rs
  - upsert_upstream: validate_issuer + auth_params whitelist before DB write
  - upsert_upstream: tokio::spawn discovery path stays (already in 29969fd); the spawned task now
    receives the validated auth_params + scopes
  - delete_upstream: unchanged

crates/assay-engine/src/lib.rs
  - build_auth_ctx_pg + build_auth_ctx_sqlite: unchanged structure (already collapsed in 29969fd);
    they now pass row.auth_params/row.scopes through the helper
```

### Migrations

```
crates/assay-auth/migrations/sqlite/{NNNN}_upstream_provider_per_idp.sql
crates/assay-auth/migrations/sqlite/{NNNN}_upstream_state_binding.sql
crates/assay-auth/migrations/pg/{NNNN}_upstream_provider_per_idp.sql
crates/assay-auth/migrations/pg/{NNNN}_upstream_state_binding.sql
```

Migration numbers picked at impl time from the existing sequence. Both columns added with defaults
so the migration is non-blocking.

## API surface

`POST /auth/admin/oidc/upstream` and `PUT /auth/admin/oidc/upstream/{slug}` request body gains:

```json
{
  "slug": "google",
  "issuer": "https://accounts.google.com",
  "client_id": "…",
  "client_secret": "…",
  "display_name": "Google",
  "icon_url": "…",
  "enabled": true,
  "scopes": ["openid", "email", "profile"],
  "auth_params": { "hd": "example.com", "prompt": "consent" }
}
```

`scopes` and `auth_params` are optional; omitted = defaults (`["openid","email","profile"]` and `{}`
respectively). Response echoes both fields. The admin response returns immediately after the DB
write; in-memory registry sync runs in `tokio::spawn` (per `29969fd`), so the response cannot report
sync status synchronously. Sync failures surface in `tracing::warn!` only — operators verify success
by retrying `/start` or by listing providers.

`GET /auth/oidc/upstream/{slug}/callback` query string gains the optional `iss` param (lenient).

Existing single-Google deployments: zero behaviour change. Default scopes match today's hardcoded
value; default `auth_params: {}` matches today's no-extra-params behaviour.

## Test plan

### Unit

- `auth_params::validate` — accepts each whitelisted key, rejects each framework-owned key, accepts
  `idp_*` prefix, rejects values >256 chars and non-string values.
- `issuer_validation::validate_issuer` — accepts `https://accounts.google.com`, rejects http
  (without flag), rejects empty host, rejects userinfo, rejects fragment, rejects literal
  `192.168.1.1` / `10.0.0.1` / `127.0.0.1` (without flag), accepts `localhost` (with flag).
- `binding::generate` — returns differing `(raw, hash)` across calls;
  `binding::verify(raw, hash) == true` for matching pair, false for mismatched.
- `binding::verify` — uses constant-time comparison (no timing-side-channel test, just ensure
  `subtle::ConstantTimeEq` or equivalent is used).
- `federation::complete_upstream_login` — three negatives: binding-missing → `Error::Oidc`,
  binding-mismatch → `Error::Oidc`, iss-mismatch → `Error::Oidc`. One positive with all three
  correct.
- `OidcRegistry::add` called twice for the same slug rotates the inner `OidcClient` (covers
  upsert-with-changed-issuer).
- `oidc_provider::store` round-trip: write row with scopes/auth_params, read back, fields match.

### Integration (uses wiremock-style discovery doc; pattern likely already exists in `crates/assay-auth/tests/` — reuse if present, build once if not)

- **Boot hydration:** insert two `enabled=true` rows + one `enabled=false` row directly into the
  SQLite store, call `build_auth_ctx_sqlite`, assert registry has exactly the two enabled.
- **Boot fail-soft:** insert a row whose issuer points at a wiremock that returns 500 on discovery;
  assert `build_auth_ctx_sqlite` returns Ok, registry has the other working providers, a warn was
  emitted.
- **Admin lifecycle:** upsert with `enabled=false` does not appear in registry. Flip to
  `enabled=true`, registry now has it. Flip back, registry drops it.
- **Auth params plumbing:** upsert with `auth_params: {prompt: "consent", hd: "example.com"}`, GET
  `/oidc/upstream/{slug}/start`, parse the 302 Location header, assert query string contains
  `prompt=consent` and `hd=example.com`.
- **Auth params rejection:** upsert with `auth_params: {redirect_uri: "evil"}` → 400 with per-key
  error.
- **CSRF binding happy-path:** GET `/start` → 302 with `Set-Cookie: assay_oidc_binding=...`; GET
  `/callback` with the cookie + matching state → 302 to `return_to` +
  `Set-Cookie: assay_oidc_binding=; Max-Age=0`.
- **CSRF binding negatives:** strip the cookie → 400 "binding missing"; tamper the cookie → 400
  "binding mismatch".
- **`iss` mismatch:** mock the upstream's discovery to advertise issuer A, callback with
  `?iss=B&...` → 400 "issuer mismatch".
- **`iss` missing (lenient):** callback without `iss` → success + warn log.
- **Discovery timeout:** wiremock that delays response past `discovery_request_timeout_secs` →
  upsert returns 200 immediately (spawned discovery task fails behind the scenes), DB row written,
  `tracing::warn!` emitted with the timeout error. Subsequent `/start` returns "unknown upstream
  provider" until the operator retries the upsert against a working IdP.
- **Issuer validation:** POST upsert with `issuer: "http://192.168.1.1"` → 400.
- **Pre-migration compat:** insert a state row with `binding_hash = ''` directly, complete the flow
  without a cookie → succeeds (sentinel skip), warn logged.

## Rollout / migration risk

- **Existing single-Google deployments:** zero behaviour change. Default scopes column matches
  today's hardcoded value; default `auth_params: {}` matches today's no-extra-params behaviour.
- **In-flight federation logins at deploy:** sentinel `binding_hash = ''` rows skip the check —
  five-minute window where pre-migration in-flight users complete without binding enforcement.
  Acceptable given (a) the window is bounded by `UPSTREAM_STATE_LIFETIME_SECS`, (b) attack requires
  foreknowledge of deploy timing plus a live victim flow.
- **Boot becomes slightly slower per-upstream** — discovery now bounded at 10s read timeout per
  provider instead of unbounded. Boot remains fail-soft per provider (warn + continue).
- **Admin upsert for unreachable IdP** — admin response is no longer blocked (`29969fd` moved
  discovery to `tokio::spawn`); the spawned task simply warns and the in-memory registry stays
  un-synced for that slug. Operators detect this by retrying `/start` or by checking logs.

## Explicit deferrals

Filed as follow-up issues at PR time:

1. **Apple Sign-In** — POST `response_mode=form_post` callback handler, ES256-JWT client_secret
   model (generated from `.p8`), name-from-form-body extraction. Requires adapter abstraction.
2. **Non-OIDC OAuth providers (GitHub web login etc.)** — adapter abstraction over `OidcRegistry` /
   `OidcClient` for IdPs without `/.well-known/openid-configuration`.
3. **`client_secret` envelope-encryption via `sysops-vault`** — CWE-256 fix. Vault crate landed in
   `21242bc`; integration is straightforward but blocked behind a separate plan.
4. **Refresh-token storage for upstreams** — `offline_access` becomes plumbable here; storage
   table + rotation is its own work.
5. **Strict `iss` mode** — flip from lenient (warn-on-missing) to strict (reject-on-missing) once
   all supported IdPs in the wild emit it. Config flag, not code change.
6. **OIDC dynamic client registration (RFC 7591)** — auto-register the engine as an RP at multiple
   IdPs. Out of scope.

## Open questions for the implementation plan

These don't change the design but get resolved at impl time:

- Does `crates/assay-auth/tests/` already have a wiremock-backed discovery harness? If yes, reuse;
  if not, the plan adds one (and that's its own ~50 LOC chunk).
- Migration numbering: confirm next free number in both sqlite + pg sequences at impl start.
- Does the existing `UpstreamLoginState` SQL use named columns or `*`? The binding_hash column add
  is trivial either way but affects the diff size.
