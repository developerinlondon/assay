# 12d — Phase 7 — Full OIDC Provider (IdP)

> Sub-plan of [12-v0.13.0-execution.md](./12-v0.13.0-execution.md). Prerequisites: Phases 4–5 from
> [12c](./12c-phase-4-6-auth-identity-zanzibar.md). Consumes `auth.oidc` (Phase 5.1) in "federated
> upstream" mode.
>
> **Source of truth for module rationale:** [11-engine-auth-modules.md](./11-engine-auth-modules.md)
> § "Own IdP with upstream federation" (lines 90–129) and § "Scope — V1" (lines 22–49).

**Phase 7 goal:** `assay-engine` is a conformant OIDC provider. Consumer applications authenticate
against _this_ IdP using the OpenID Connect Core 1.0 authorization code flow with PKCE. Local users
authenticate via password + passkey; federated users authenticate via upstream (Google, Apple,
GitHub, any OIDC provider). The IdP issues its own ID tokens to consumer apps regardless of how the
user signed in — consumers never see upstream tokens.

**Biggest phase at ~25 hours.** Tasks 7.1–7.10 break it into ~2-3h blocks.

---

## Architecture recap

```
Consumer app
  │ OIDC code flow + PKCE
  ▼
assay-engine IdP
  ├── /.well-known/openid-configuration   — discovery doc (task 7.1)
  ├── /jwks.json                          — active + history keys (task 7.1)
  ├── /authorize                          — user-agent entry point (task 7.3)
  │     │
  │     ├─ Local auth → password / passkey (reuses Phase 4/5)
  │     └─ Federated → upstream OIDC (reuses Phase 5.1 client)
  │
  ├── /consent                            — user-facing grant screen (task 7.3)
  ├── /token                              — code / refresh / client-cred grants (task 7.4)
  ├── /userinfo                           — claims endpoint (task 7.5)
  ├── /revoke                             — token revocation (task 7.5)
  ├── /logout                             — end session + back-channel (task 7.7)
  │
  ├── /admin/clients                      — client CRUD (admin auth; task 7.6)
  └── /admin/federation                   — upstream provider CRUD (task 7.6)
```

---

## Task 7.1: Discovery + JWKS endpoints + key rotation scheduler

**Files:**

- Create: `crates/assay-auth/src/oidc_provider/discovery.rs`
- Create: `crates/assay-auth/src/oidc_provider/jwks.rs`
- Create: `crates/assay-auth/src/oidc_provider/mod.rs`

Plan 11 reference: "JWKS history" — rotation without invalidating old tokens.

- [ ] **Step 1: Provider config**

```rust
// oidc_provider/mod.rs
pub struct OidcProvider {
    pub issuer: String,                 // e.g. "https://auth.example.com"
    pub jwt: JwtConfig,                 // active key + history (from Phase 4 jwt module)
    pub clients: Arc<dyn ClientStore>,  // OIDC client registry
    pub federation: Arc<dyn FederationStore>,  // upstream provider registry
    pub rotation_interval: std::time::Duration,
}

impl OidcProvider {
    pub fn discovery_json(&self) -> serde_json::Value {
        serde_json::json!({
            "issuer": self.issuer,
            "authorization_endpoint": format!("{}/authorize", self.issuer),
            "token_endpoint": format!("{}/token", self.issuer),
            "userinfo_endpoint": format!("{}/userinfo", self.issuer),
            "jwks_uri": format!("{}/jwks.json", self.issuer),
            "revocation_endpoint": format!("{}/revoke", self.issuer),
            "end_session_endpoint": format!("{}/logout", self.issuer),
            "scopes_supported": ["openid", "email", "profile", "offline_access"],
            "response_types_supported": ["code"],
            "grant_types_supported": ["authorization_code", "refresh_token", "client_credentials"],
            "subject_types_supported": ["public"],
            "id_token_signing_alg_values_supported": ["RS256"],
            "token_endpoint_auth_methods_supported": ["client_secret_basic", "client_secret_post", "none"],
            "code_challenge_methods_supported": ["S256"],
            "claims_supported": ["sub", "email", "email_verified", "name", "preferred_username"],
        })
    }
}
```

- [ ] **Step 2: Rotation scheduler**

```rust
pub fn spawn_key_rotation(provider: Arc<OidcProvider>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(provider.rotation_interval);
        interval.tick().await;  // skip immediate tick
        loop {
            interval.tick().await;
            let new_key = match crate::jwt::generate_rs256_keypair() {
                Ok(k) => k,
                Err(e) => { tracing::error!(?e, "key generation failed; skipping rotation"); continue; }
            };
            provider.jwt.rotate(new_key);
            tracing::info!("rotated JWKS signing key");
        }
    })
}
```

- [ ] **Step 3: Routes**

```rust
pub fn router() -> Router<AuthCtx> {
    Router::new()
        .route("/.well-known/openid-configuration", get(|State(ctx): State<AuthCtx>| async move {
            axum::Json(ctx.oidc_provider.discovery_json())
        }))
        .route("/jwks.json", get(|State(ctx): State<AuthCtx>| async move {
            axum::Json(ctx.oidc_provider.jwt.jwks_json())
        }))
}
```

- [ ] **Step 4: Tests**

- Hit `/.well-known/openid-configuration`, assert all required OIDC Core fields present.
- Rotate keys, verify old JWKS entry still present in response.
- Hit `/jwks.json` after 3 rotations, assert 3 keys (active + 2 history) returned.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat(auth/oidc-provider): discovery + JWKS + key rotation scheduler"
```

---

## Task 7.2: Client registry + federation registry

**Files:**

- Create: `crates/assay-auth/src/oidc_provider/store.rs`
- Migrations: `migrations/{postgres,sqlite}/03_oidc_provider.sql`

- [ ] **Step 1: Types**

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OidcClient {
    pub client_id: String,
    pub client_secret_hash: Option<String>,  // None = public client (PKCE-only)
    pub redirect_uris: Vec<String>,
    pub name: String,
    pub logo_url: Option<String>,
    pub token_endpoint_auth_method: TokenAuthMethod,
    pub default_scopes: Vec<String>,
    pub require_consent: bool,
    pub created_at: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum TokenAuthMethod { ClientSecretBasic, ClientSecretPost, None }

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UpstreamProvider {
    pub slug: String,          // "google", "apple", "github", ...
    pub issuer: String,        // https://accounts.google.com
    pub client_id: String,
    pub client_secret: String,
    pub display_name: String,
    pub icon_url: Option<String>,
    pub enabled: bool,
}
```

- [ ] **Step 2: Store traits**

```rust
#[async_trait::async_trait]
pub trait ClientStore: Send + Sync + 'static {
    async fn create(&self, client: &OidcClient) -> anyhow::Result<()>;
    async fn get(&self, client_id: &str) -> anyhow::Result<Option<OidcClient>>;
    async fn list(&self) -> anyhow::Result<Vec<OidcClient>>;
    async fn update(&self, client: &OidcClient) -> anyhow::Result<()>;
    async fn delete(&self, client_id: &str) -> anyhow::Result<bool>;
}

#[async_trait::async_trait]
pub trait FederationStore: Send + Sync + 'static {
    async fn upsert(&self, p: &UpstreamProvider) -> anyhow::Result<()>;
    async fn get(&self, slug: &str) -> anyhow::Result<Option<UpstreamProvider>>;
    async fn list(&self) -> anyhow::Result<Vec<UpstreamProvider>>;
    async fn delete(&self, slug: &str) -> anyhow::Result<bool>;
}
```

- [ ] **Step 3: Schema**

```sql
-- migrations/postgres/03_oidc_provider.sql
CREATE TABLE oidc_clients (
    client_id TEXT PRIMARY KEY,
    client_secret_hash TEXT,
    redirect_uris TEXT NOT NULL,  -- JSON array
    name TEXT NOT NULL,
    logo_url TEXT,
    token_endpoint_auth_method TEXT NOT NULL,
    default_scopes TEXT NOT NULL,  -- JSON array
    require_consent BOOLEAN NOT NULL DEFAULT TRUE,
    created_at DOUBLE PRECISION NOT NULL
);

CREATE TABLE upstream_providers (
    slug TEXT PRIMARY KEY,
    issuer TEXT NOT NULL,
    client_id TEXT NOT NULL,
    client_secret TEXT NOT NULL,  -- encrypted at rest in a later iteration
    display_name TEXT NOT NULL,
    icon_url TEXT,
    enabled BOOLEAN NOT NULL DEFAULT TRUE
);

CREATE TABLE oidc_authorization_codes (
    code TEXT PRIMARY KEY,
    client_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    redirect_uri TEXT NOT NULL,
    scopes TEXT NOT NULL,  -- JSON array
    code_challenge TEXT NOT NULL,
    code_challenge_method TEXT NOT NULL,
    nonce TEXT,
    issued_at DOUBLE PRECISION NOT NULL,
    expires_at DOUBLE PRECISION NOT NULL,
    consumed BOOLEAN NOT NULL DEFAULT FALSE
);

CREATE TABLE oidc_refresh_tokens (
    token_hash TEXT PRIMARY KEY,
    client_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    scopes TEXT NOT NULL,
    issued_at DOUBLE PRECISION NOT NULL,
    expires_at DOUBLE PRECISION NOT NULL,
    revoked BOOLEAN NOT NULL DEFAULT FALSE
);
CREATE INDEX oidc_refresh_user ON oidc_refresh_tokens (user_id);
```

- [ ] **Step 4: PG + SQLite + Surreal impls**

Mirror the pattern from Phase 4 Task 4.6 + Phase 6 Task 6.7.

- [ ] **Step 5: Admin routes (placeholder)**

Real admin routes land in Task 7.6 with proper auth. For now, stubs that return `501` when hit.

- [ ] **Step 6: Commit**

```bash
git commit -m "feat(auth/oidc-provider): client + federation + auth-code + refresh stores"
```

---

## Task 7.3: `/authorize` + consent screen + local / federated login

**Files:**

- Create: `crates/assay-auth/src/oidc_provider/authorize.rs`
- Create: `crates/assay-auth/src/oidc_provider/templates/{login.html,consent.html}`

The single most intricate route in the IdP — it orchestrates session, chosen auth method, and
consent decisions. Keep it narrow by delegating:

- Auth method choice → Phase 4/5 modules (password, passkey, OIDC client for upstream).
- Session management → Phase 4 session module.
- Claim mapping → Task 7.8.

- [ ] **Step 1: Request validation**

Parse query params into a struct; reject malformed / unregistered clients / missing PKCE. Per OIDC
Core §3.1.2.1:

```rust
#[derive(Deserialize)]
struct AuthorizeRequest {
    response_type: String,       // must be "code"
    client_id: String,
    redirect_uri: String,        // must match client's registered list
    scope: String,               // space-separated; must include "openid"
    state: String,               // echoed back unchanged
    nonce: Option<String>,
    code_challenge: String,      // required; PKCE mandatory
    code_challenge_method: String,  // "S256"
    prompt: Option<String>,      // "none" / "login" / "consent" / "select_account"
    max_age: Option<u32>,
    ui_locales: Option<String>,
}
```

Validate + look up client. If invalid, redirect to `redirect_uri` with `error=invalid_request` (when
safe) or render an error page (when redirect_uri itself is untrusted).

- [ ] **Step 2: Session probe**

- If the user has an active assay session → skip login UI; go to consent (or straight to code
  issuance when `require_consent=false`).
- If no session → render `login.html` with method choices (password + registered upstream
  providers).

- [ ] **Step 3: Local login POST**

`POST /authorize/password` — verify email + password via Phase 4 `Password`, create session,
redirect back to the `/authorize?…` URL with cookies set.

`POST /authorize/passkey/start` + `/authorize/passkey/finish` — delegate to Phase 5 Task 5.2 flows.

- [ ] **Step 4: Upstream federation login**

`GET /authorize/federate/{provider_slug}` — delegate to Phase 5 Task 5.1 OIDC client authorize flow.
On callback, look up the user-upstream link (`user_upstream` table from Task 4.6), create the user
if first seen, create session, redirect back.

- [ ] **Step 5: Consent page**

Renders the scopes being requested. User clicks "Approve" → POST records consent, generates an
authorization code, stores it in `oidc_authorization_codes` with `code_challenge`, redirects to the
client's `redirect_uri` with `?code=…&state=…`.

"Deny" → redirects with `?error=access_denied&state=…`.

- [ ] **Step 6: Remember-consent**

Optional: persist per-user / per-client consent so `prompt=consent` (or forced re-consent) is the
exception. For V1 keep simple: consent required on every authorize flow unless client's
`require_consent=false`.

- [ ] **Step 7: Tests**

- Happy path: login with password → consent → code issued → consumer exchanges → id_token returned
  with expected claims.
- Bad redirect URI: rejected.
- PKCE missing: rejected.
- Expired session: redirected to login.
- `prompt=none` + no session: `error=login_required`.

- [ ] **Step 8: Commit**

```bash
git commit -m "feat(auth/oidc-provider): /authorize + login + consent flows"
```

---

## Task 7.4: `/token` endpoint (authorization_code / refresh_token / client_credentials)

**Files:**

- Create: `crates/assay-auth/src/oidc_provider/token.rs`

- [ ] **Step 1: Request routing by grant_type**

```rust
pub async fn token_handler(
    State(ctx): State<AuthCtx>,
    headers: HeaderMap,
    Form(req): Form<TokenRequest>,
) -> Result<Json<TokenResponse>, TokenError> {
    let client = authenticate_client(&ctx, &headers, &req).await?;
    match req.grant_type.as_str() {
        "authorization_code" => grant_authorization_code(&ctx, &client, &req).await,
        "refresh_token"      => grant_refresh(&ctx, &client, &req).await,
        "client_credentials" => grant_client_credentials(&ctx, &client, &req).await,
        other                => Err(TokenError::unsupported(other)),
    }
}
```

- [ ] **Step 2: Client authentication**

Support all three methods from the discovery doc: `client_secret_basic` (Basic header),
`client_secret_post` (form field), `none` (public PKCE-only). Compare `client_secret_hash` using
constant-time equality.

- [ ] **Step 3: `authorization_code` grant**

1. Look up code in `oidc_authorization_codes`, atomically mark `consumed=true` using a
   `WHERE consumed = FALSE` clause. If the update affects 0 rows, the code was already used — error.
2. Verify expiry, `client_id`, `redirect_uri` match.
3. Verify PKCE: `BASE64URL(SHA256(code_verifier)) == code_challenge`.
4. Issue an `id_token` (JWT signed with active key, claims per Task 7.8), an `access_token` (JWT or
   opaque), and optionally a `refresh_token` (opaque, hashed in DB).
5. Return.

- [ ] **Step 4: `refresh_token` grant**

1. Hash the provided refresh_token, look it up in `oidc_refresh_tokens`.
2. Check expiry, revocation, client match.
3. Rotate: mark the old one revoked, issue a new one. Return new access + id + refresh tokens.
4. If a revoked token is presented, revoke ALL refresh tokens for that user — replay detection.

- [ ] **Step 5: `client_credentials` grant**

Machine-to-machine flow. Client authenticates with secret, gets an access_token bound to its own
identity (no user). Useful for internal services calling engine APIs.

- [ ] **Step 6: Error taxonomy**

Match OIDC/OAuth2 spec error codes: `invalid_request`, `invalid_client`, `invalid_grant`,
`unauthorized_client`, `unsupported_grant_type`, `invalid_scope`.

- [ ] **Step 7: Tests**

- Happy code exchange; assert `id_token` verifies against JWKS and contains expected claims.
- Reuse code → `invalid_grant`.
- Wrong PKCE verifier → `invalid_grant`.
- Refresh rotation + replay → original token invalid after rotation; replay revokes all.
- Client credentials → access_token has correct `client_id` claim, no `sub` claim.

- [ ] **Step 8: Commit**

```bash
git commit -m "feat(auth/oidc-provider): /token (code + refresh + client_credentials)"
```

---

## Task 7.5: `/userinfo` + `/revoke`

**Files:**

- Create: `crates/assay-auth/src/oidc_provider/userinfo.rs`
- Create: `crates/assay-auth/src/oidc_provider/revoke.rs`

- [ ] **Step 1: `/userinfo` endpoint**

Validates bearer access_token (from JWT or opaque-lookup), returns JSON with claims matching the
scopes granted in the token.

- [ ] **Step 2: `/revoke` endpoint**

Per RFC 7009. Accepts a token (access or refresh) + client credentials. Marks it revoked. Returns
200 regardless of validity (per spec — avoid leaking token existence).

- [ ] **Step 3: Tests**

- `/userinfo` with valid access_token returns claims.
- `/userinfo` with missing / expired token returns 401.
- `/revoke` with valid refresh token marks it revoked; subsequent use fails.
- `/revoke` with invalid token still returns 200.

- [ ] **Step 4: Commit**

```bash
git commit -m "feat(auth/oidc-provider): /userinfo + /revoke"
```

---

## Task 7.6: Admin HTTP API for client + federation management

**Files:**

- Create: `crates/assay-auth/src/oidc_provider/admin.rs`

Plan 11 notes: "Admin UI is V2 (primitives + HTTP admin API in V1)". This task lands the HTTP API.

- [ ] **Step 1: Routes**

```
POST   /admin/oidc/clients                    (create)
GET    /admin/oidc/clients                    (list)
GET    /admin/oidc/clients/{id}               (get)
PATCH  /admin/oidc/clients/{id}               (update)
DELETE /admin/oidc/clients/{id}               (delete)

POST   /admin/oidc/federation                 (create / upsert)
GET    /admin/oidc/federation                 (list)
GET    /admin/oidc/federation/{slug}          (get)
DELETE /admin/oidc/federation/{slug}          (delete)
```

- [ ] **Step 2: Auth**

All admin routes require one of:

- Bearer token with `admin` Biscuit fact, OR
- Session cookie belonging to a user with `admin` role (via Zanzibar check on
  `assay_admin:root#admin@user:<id>`).

Middleware gate at the router level.

- [ ] **Step 3: Validation**

- Client: redirect URIs must parse, at least one required, logos URL https-only. Generate
  client_secret if `token_endpoint_auth_method != none`, return it ONCE in the create response
  (never readable again).
- Federation: issuer must discover (HTTP GET to `<issuer>/.well-known/openid-configuration`
  succeeds).

- [ ] **Step 4: CLI counterpart**

The `assay-engine` binary grows subcommands (Phase 8):

```bash
assay-engine admin client create --name "My App" --redirect "https://app.example.com/cb"
assay-engine admin federation add --slug google --issuer https://accounts.google.com --client-id X --client-secret Y
```

Shell out to the HTTP API locally or speak to the DB directly — decide based on whether the binary
is running as a server at the moment.

- [ ] **Step 5: Tests**

- Create client, list, patch, delete.
- Create federation → subsequent authorize flow can use it.
- Admin auth rejects non-admin sessions.

- [ ] **Step 6: Commit**

```bash
git commit -m "feat(auth/oidc-provider): admin HTTP API for clients + federation"
```

---

## Task 7.7: Logout + back-channel logout + SSO session registry

**Files:**

- Create: `crates/assay-auth/src/oidc_provider/logout.rs`
- Modify: `crates/assay-auth/src/session.rs` (add SSO tracking)

Plan 11 lines 122–127: session registry enables SSO across clients; back-channel logout propagates
to all clients.

- [ ] **Step 1: SSO session tracking**

When `/token` issues an id_token, record a row in `oidc_sessions`:

```sql
CREATE TABLE oidc_sessions (
    sid TEXT PRIMARY KEY,          -- matches id_token `sid` claim
    user_id TEXT NOT NULL,
    client_id TEXT NOT NULL,
    assay_session_id TEXT,         -- link to the underlying assay session
    issued_at DOUBLE PRECISION NOT NULL,
    backchannel_logout_uri TEXT
);
CREATE INDEX oidc_sessions_user ON oidc_sessions (user_id);
CREATE INDEX oidc_sessions_assay ON oidc_sessions (assay_session_id);
```

- [ ] **Step 2: `/logout` endpoint**

Per OIDC RP-Initiated Logout 1.0:

```
GET /logout
  ?id_token_hint=...                  (optional but recommended)
  &post_logout_redirect_uri=...       (client's registered URL)
  &state=...
```

Revoke the underlying assay session. Optionally render a "logging out" page or redirect.

- [ ] **Step 3: Back-channel logout**

When a user's session is revoked, for every `oidc_session` row with that `assay_session_id`, POST a
logout token JWT to the client's `backchannel_logout_uri`. Fire-and-forget with timeouts; log
failures but don't retry (clients are supposed to be idempotent).

- [ ] **Step 4: Tests**

- Log in via two clients, share SSO session. Logout at IdP → both clients receive back-channel
  logout.
- `/userinfo` fails after logout.

- [ ] **Step 5: Commit**

```bash
git commit -m "feat(auth/oidc-provider): logout + back-channel logout + SSO registry"
```

---

## Task 7.8: Claim mapping + upstream federation integration

**Files:**

- Create: `crates/assay-auth/src/oidc_provider/claims.rs`

When a user signs in via Google, the IdP creates a local user linked to Google's `sub`, then issues
its _own_ id_token with its own `sub`. Claim mapping decides how to populate local claims from
upstream.

- [ ] **Step 1: Mapping config (per upstream provider)**

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ClaimMapping {
    pub email_claim: String,           // default "email"
    pub email_verified_claim: String,  // default "email_verified"
    pub name_claim: String,            // default "name"
    pub preferred_username_claim: Option<String>,
}

impl Default for ClaimMapping {
    fn default() -> Self {
        Self {
            email_claim: "email".into(),
            email_verified_claim: "email_verified".into(),
            name_claim: "name".into(),
            preferred_username_claim: Some("preferred_username".into()),
        }
    }
}
```

- [ ] **Step 2: Build ID token claims**

```rust
pub fn build_id_token_claims(
    user: &User,
    client_id: &str,
    session: &Session,
    scopes: &[String],
    nonce: Option<&str>,
) -> serde_json::Value {
    let mut claims = serde_json::json!({
        "iss": /* provider.issuer */,
        "sub": user.id,
        "aud": client_id,
        "iat": now(),
        "exp": now() + 3600.0,
        "sid": session.id,
    });
    if scopes.iter().any(|s| s == "email") {
        claims["email"] = serde_json::Value::String(user.email.clone().unwrap_or_default());
        claims["email_verified"] = serde_json::Value::Bool(user.email_verified);
    }
    if scopes.iter().any(|s| s == "profile") {
        if let Some(name) = &user.display_name {
            claims["name"] = serde_json::Value::String(name.clone());
        }
    }
    if let Some(n) = nonce {
        claims["nonce"] = serde_json::Value::String(n.into());
    }
    claims
}
```

- [ ] **Step 3: Tests**

- Scope `openid` only → minimal claims.
- Scope `openid email` → adds email.
- Scope `openid profile email` → adds everything.
- Nonce round-trip.

- [ ] **Step 4: Commit**

```bash
git commit -m "feat(auth/oidc-provider): scope-driven claim mapping"
```

---

## Task 7.9: Hardening — PKCE enforcement, replay protection, rate limits

**Files:**

- Modify: existing oidc_provider modules (cross-cutting)

- [ ] **Step 1: PKCE mandatory for public clients**

When `client.token_endpoint_auth_method == None`, require `code_challenge`. Public clients without
PKCE are rejected at `/authorize`.

- [ ] **Step 2: Authorization code replay detection**

Already covered by `UPDATE ... WHERE consumed=false`. Add a test that asserts the second exchange
with the same code errors with `invalid_grant`.

- [ ] **Step 3: Rate limits**

Per-IP rate limit on `/token`, `/authorize`, password/passkey verification endpoints. Use a simple
in-memory sliding window (1000 req/min/IP default); back-pressure to `429`. A distributed rate limit
is V2.

```rust
// crates/assay-auth/src/oidc_provider/rate_limit.rs
pub struct RateLimiter {
    windows: parking_lot::Mutex<HashMap<String, (Instant, u32)>>,
    limit: u32,
    window: Duration,
}
```

- [ ] **Step 4: Open-redirect prevention**

`redirect_uri` must be an exact match against the client's registered list (no prefix matching).

- [ ] **Step 5: Nonce replay**

`nonce` is echoed back in the id_token but not persisted — the consumer app is responsible for
validating nonce freshness. Document this.

- [ ] **Step 6: Security tests**

- Replay code → rejected.
- Non-matching redirect_uri → rejected.
- Public client without PKCE → rejected.
- Rate limit: 1001 requests in 60s → 1001st gets 429.
- Admin route without auth → 401.

- [ ] **Step 7: Commit**

```bash
git commit -m "feat(auth/oidc-provider): hardening (PKCE, replay, rate limit, redirect uri)"
```

---

## Task 7.10: Conformance against OpenID Foundation test suite

Plan 11: "Conformance test pass (OpenID Foundation where feasible)". Full OpenID Foundation
conformance testing requires a hosted suite at
[openid.net/certification](https://openid.net/certification/) — that's a post-release activity. For
0.13.0, land a self-administered conformance suite covering the Core 1.0 profile.

- [ ] **Step 1: Deploy the OID provider to a local testcontainers endpoint**

Package the engine + SQLite into a test fixture that spins up an engine with the OIDC provider
enabled.

- [ ] **Step 2: Use a known-good OIDC client as the test driver**

Use the Phase 5.1 `auth.oidc` client, point it at the local IdP, run the full flow. Assert:

1. Discovery doc parseable.
2. Authorize → login → consent → code → token → userinfo → id_token verifies.
3. Refresh token round-trip.
4. Logout clears session.
5. JWKS rotation mid-session: old id_token still verifies; new tokens use new key.

- [ ] **Step 3: Commit**

```bash
git commit -m "test(auth/oidc-provider): self-administered conformance suite"
```

Post-release: submit to the OpenID Foundation certified test suite. Not gating for 0.13.0.

---

## Phase 7 exit criteria

- Every OIDC Core 1.0 endpoint implemented and responding correctly.
- `auth.oidc` client (Phase 5.1) drives `assay-engine`'s OIDC provider end-to-end; id_token
  verifies.
- Consumer app (mock in test) gets its own id_token with claims from the IdP, never from the
  upstream (when federated).
- SSO: two clients, one user, both get identified. Logout at IdP → back-channel fires to both
  clients.
- Admin API: create client + federation via HTTP + CLI; flow uses them.
- Security tests all green: PKCE mandatory, replay rejected, rate limits enforced, open-redirect
  prevented.
- Binary size for `cargo bloat -p assay-engine --release --features "auth auth-oidc-provider"`
  within 5 MB of the non-provider build.

---

## What's next

**[12e](./12e-phase-8-10-binary-ci-ship.md)** — the final mile. `assay-engine` binary wires
`EngineState` with FromRef composition, the dashboard learns engine views (queue stats, worker
registry, auth client browser), CI publishes per-crate tags, and v0.13.0 ships.
