//! OIDC client — discovery, PKCE, callback, userinfo.
//!
//! Plan 12c task 5.1 reference. We wrap the [`openidconnect`] 4 typed
//! `CoreClient` per upstream so callers don't have to thread its
//! type-state generics through every handler. Each provider is
//! discovered once at registration time (`<issuer>/.well-known/openid-configuration`)
//! and the resulting client is cached behind a slug key.
//!
//! The phase-5 surface is intentionally library-only:
//!
//! - [`OidcRegistry`] — slug-keyed registry of discovered providers
//! - [`OidcClient`] — wraps one upstream's discovered metadata + RP creds
//! - [`UpstreamProvider`] — POD record (slug + issuer + client id/secret +
//!   scopes); matches the `auth.upstream_providers` row shape that admin
//!   CRUD will land in a later plan
//! - [`UpstreamUserInfo`] — verified result of one login round-trip
//!
//! Engine boot constructs an empty registry; populated providers come
//! from a future admin API or seed config (out of phase 5 scope).

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::{Duration, Instant};

use openidconnect::core::{
    CoreAuthenticationFlow, CoreClient, CoreIdToken, CoreIdTokenClaims, CoreIdTokenVerifier,
    CoreJsonWebKey, CoreJsonWebKeySet, CoreJwsSigningAlgorithm, CoreProviderMetadata,
    CoreUserInfoClaims,
};
use openidconnect::reqwest as oidc_reqwest;
use openidconnect::{
    AuthorizationCode, ClaimsVerificationError, ClientId, ClientSecret, CsrfToken,
    EndpointMaybeSet, EndpointNotSet, EndpointSet, IssuerUrl, JsonWebKeySetUrl, Nonce,
    OAuth2TokenResponse, PkceCodeChallenge, PkceCodeVerifier, RedirectUrl, Scope,
    SignatureVerificationError, SubjectIdentifier, TokenResponse,
};
use parking_lot::RwLock;
use url::Url;

use crate::error::{Error, Result};

/// Fallback scopes requested during upstream federation login when a
/// row's `scopes` column is empty. The admin API normally fills the
/// column from the request body (or its server-side default of
/// `'["openid","email","profile"]'`); this constant only fires for
/// rows that pre-date the V5 migration filling in the default.
pub const DEFAULT_UPSTREAM_SCOPES: &[&str] = &["openid", "email", "profile"];

/// Lower bound on how long a fetched upstream JWKS is trusted before a
/// proactive re-fetch — even when the certs endpoint advertises a
/// shorter `max-age`. Keeps us from re-fetching on essentially every
/// login.
const JWKS_MIN_TTL: Duration = Duration::from_secs(300);

/// Upper bound on the JWKS cache TTL. Google advertises multi-hour
/// `max-age`s on its certs endpoint; we cap the trusted window so a
/// cache that never gets a rotation-triggered refetch still self-heals
/// within a bounded interval.
const JWKS_MAX_TTL: Duration = Duration::from_secs(6 * 60 * 60);

/// TTL used when the certs response carries no usable `Cache-Control:
/// max-age` directive.
const JWKS_DEFAULT_TTL: Duration = Duration::from_secs(60 * 60);

/// Minimum gap between *reactive* (unknown-`kid`) JWKS re-fetches. A
/// burst of tokens carrying a `kid` we'll never hold — bogus, or from a
/// misconfigured upstream — can't stampede the certs endpoint faster
/// than this.
const JWKS_MIN_REFETCH_INTERVAL: Duration = Duration::from_secs(60);

/// In-memory, refreshable cache of one upstream's signing keys.
///
/// Seeded from the discovery metadata at registration, then re-fetched
/// from `jwks_uri` when the keys go stale — either the TTL lapsed
/// (honoring the certs endpoint's `Cache-Control: max-age`) or an
/// incoming id_token carried a `kid` we don't yet hold (upstream key
/// rotation). Before this cache existed the verifier pinned the
/// boot-time keys forever, so every login signed by a rotated Google
/// key failed with `Signature verification failed`.
struct JwksCache {
    keys: Vec<CoreJsonWebKey>,
    /// Deadline for a proactive refresh, set from the last fetch's TTL.
    expires_at: Instant,
    /// When we last hit `jwks_uri` — rate-limits reactive refetches.
    last_fetch: Instant,
}

/// POD record describing one upstream identity provider. Mirrors the
/// planned `auth.upstream_providers` table shape (see plan 12d) so the
/// admin API can `INSERT … RETURNING *` and feed the row directly into
/// [`OidcRegistry::add`] without a translation step.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpstreamProvider {
    /// Stable slug used in routes (`/login/{slug}`) and as the
    /// `auth.user_upstream.provider` column value. Lower-snake-case
    /// matches the rest of the codebase's naming.
    pub slug: String,
    /// Issuer URL — the value the discovery doc lives under
    /// (`<issuer>/.well-known/openid-configuration`).
    pub issuer: String,
    /// RP client id registered with the upstream.
    pub client_id: String,
    /// RP client secret registered with the upstream. Stored as
    /// plaintext here because phase 5 has no secret-at-rest envelope yet
    /// — admin CRUD lands with the encryption story.
    pub client_secret: String,
    /// Scopes requested at authorize time. Common set:
    /// `["openid", "email", "profile"]`. `openid` is added implicitly
    /// by [`openidconnect`]; we forward the rest unchanged.
    pub scopes: Vec<String>,
    /// Per-IdP authorize-URL parameters (`prompt`, `hd`, `domain_hint`,
    /// `idp_*`, …). Validated against the
    /// [`crate::oidc_provider::auth_params`] whitelist before the row
    /// reaches this struct. Empty by default.
    pub auth_params: BTreeMap<String, String>,
}

/// Verified userinfo returned by [`OidcClient::complete_login`]. Carries
/// the canonical fields the rest of the auth stack needs to upsert into
/// `auth.users` + `auth.user_upstream`. `raw_claims` carries the
/// id_token's full claim set so callers can pluck custom claims (e.g.
/// `groups`, `roles`) without a second parse.
#[derive(Clone, Debug)]
pub struct UpstreamUserInfo {
    pub provider: String,
    pub subject: String,
    pub email: Option<String>,
    pub email_verified: bool,
    pub name: Option<String>,
    pub picture: Option<String>,
    pub raw_claims: serde_json::Value,
}

/// A single discovered upstream — wraps the [`openidconnect`] typed
/// client and the PoD metadata used to construct it.
///
/// The CoreClient generic state after `from_provider_metadata +
/// set_redirect_uri` is `<EndpointSet, EndpointNotSet, EndpointNotSet,
/// EndpointNotSet, EndpointMaybeSet, EndpointMaybeSet>` — auth URL set
/// (so `authorize_url` works), token + userinfo MaybeSet (we error at
/// runtime if the upstream's discovery doc is missing one).
pub struct OidcClient {
    inner: CoreClient<
        EndpointSet,
        EndpointNotSet,
        EndpointNotSet,
        EndpointNotSet,
        EndpointMaybeSet,
        EndpointMaybeSet,
    >,
    /// Original PoD record for round-trip / introspection
    /// (e.g. admin "what's configured" pages).
    provider: UpstreamProvider,
    /// Owned redirect URL — `set_redirect_uri` consumed it on the
    /// builder, but operators sometimes want it back without re-parsing.
    redirect_uri: RedirectUrl,
    /// `jwks_uri` from the upstream's discovery doc — where we re-fetch
    /// signing keys when the cache goes stale.
    jwks_uri: JsonWebKeySetUrl,
    /// Algorithms the upstream advertised for id_token signing
    /// (`id_token_signing_alg_values_supported`), captured at discovery
    /// so a rebuilt verifier restricts algorithms exactly as the
    /// metadata-derived one did.
    signing_algs: Vec<CoreJwsSigningAlgorithm>,
    /// Refreshable signing-key cache. See [`JwksCache`].
    jwks: Arc<RwLock<JwksCache>>,
    /// Dedicated HTTP client for JWKS re-fetches. Kept separate from the
    /// `openidconnect`/`oauth2` reqwest used for token exchange: that
    /// crate pins a different `reqwest` major, so we fetch keys with the
    /// auth crate's own `reqwest` (matching [`crate::external_jwt`]).
    jwks_http: reqwest::Client,
}

impl OidcClient {
    /// Borrow the original PoD record.
    pub fn provider(&self) -> &UpstreamProvider {
        &self.provider
    }

    /// Borrow the configured redirect URI.
    pub fn redirect_uri(&self) -> &RedirectUrl {
        &self.redirect_uri
    }

    /// Step 1 of the authorization-code-+-PKCE flow. Generates a PKCE
    /// pair, asks the [`openidconnect`] client for the redirect URL,
    /// returns the URL alongside the verifier + nonce for round-trip
    /// (callers persist them, typically in the session).
    ///
    /// `state` lets callers pin a known CSRF value (e.g. the session id)
    /// rather than the library-generated random one — useful when the
    /// callback handler uses `state` to look the in-progress login up.
    /// Pass [`CsrfToken::new_random`] via `CsrfToken::new(...)` if you
    /// don't have one already.
    pub fn start_login(&self, state: CsrfToken) -> StartedLogin {
        let (pkce_challenge, pkce_verifier) = PkceCodeChallenge::new_random_sha256();
        let mut request = self.inner.authorize_url(
            CoreAuthenticationFlow::AuthorizationCode,
            move || state,
            Nonce::new_random,
        );
        for scope in &self.provider.scopes {
            // `openid` scope is added by openidconnect when
            // `use_openid_scope` is true (default after
            // `from_provider_metadata`); skip a duplicate so the URL
            // stays clean.
            if scope == "openid" {
                continue;
            }
            request = request.add_scope(Scope::new(scope.clone()));
        }
        for (k, v) in &self.provider.auth_params {
            request = request.add_extra_param(k.clone(), v.clone());
        }
        let (url, csrf_token, nonce) = request.set_pkce_challenge(pkce_challenge).url();
        StartedLogin {
            url,
            csrf_token,
            nonce,
            pkce_verifier,
        }
    }

    /// Step 2 — exchange the upstream's `code` for tokens, validate the
    /// id_token against the cached JWKS + nonce, and (when the upstream
    /// publishes a userinfo endpoint) supplement the claims with a
    /// userinfo call.
    ///
    /// `pkce_verifier` and `nonce` must be the values returned from
    /// [`OidcClient::start_login`] for the same login — callers persist
    /// them server-side keyed by `state`.
    pub async fn complete_login(
        &self,
        code: String,
        pkce_verifier: PkceCodeVerifier,
        nonce: Nonce,
    ) -> Result<UpstreamUserInfo> {
        let http = build_oidc_http_client(HttpClientOptions::default())?;
        let token_response = self
            .inner
            .exchange_code(AuthorizationCode::new(code))
            .map_err(|e| Error::Oidc(format!("exchange_code config: {e}")))?
            .set_pkce_verifier(pkce_verifier)
            .request_async(&http)
            .await
            .map_err(|e| Error::Oidc(format!("token exchange: {e}")))?;

        let id_token = token_response
            .id_token()
            .ok_or_else(|| Error::Oidc("upstream returned no id_token".to_string()))?;
        let claims = self.verify_id_token_claims(id_token, &nonce).await?;

        let subject = claims.subject().to_string();
        let mut email = claims.email().map(|e| e.to_string());
        let mut email_verified = claims.email_verified().unwrap_or(false);
        let mut name = claims
            .name()
            .and_then(|map| map.get(None))
            .map(|n| n.to_string());
        let mut picture = claims
            .picture()
            .and_then(|map| map.get(None))
            .map(|u| u.to_string());

        // Best-effort userinfo fetch. Some upstreams omit email/name from
        // the id_token and only expose them via /userinfo. If the
        // upstream doesn't publish a userinfo endpoint or the call fails,
        // we keep what the id_token gave us — login still works, the
        // missing fields just show up as None.
        let mut raw_claims =
            serde_json::to_value(claims).unwrap_or_else(|_| serde_json::json!({"sub": subject}));
        if let Ok(req) = self.inner.user_info(
            token_response.access_token().clone(),
            Some(SubjectIdentifier::new(subject.clone())),
        ) && let Ok(userinfo) = req.request_async(&http).await
        {
            let user_claims: CoreUserInfoClaims = userinfo;
            if email.is_none() {
                email = user_claims.email().map(|e| e.to_string());
            }
            if !email_verified {
                email_verified = user_claims.email_verified().unwrap_or(email_verified);
            }
            if name.is_none() {
                name = user_claims
                    .name()
                    .and_then(|map| map.get(None))
                    .map(|n| n.to_string());
            }
            if picture.is_none() {
                picture = user_claims
                    .picture()
                    .and_then(|map| map.get(None))
                    .map(|u| u.to_string());
            }
            // Merge userinfo into raw_claims so downstream code that
            // wants e.g. `groups` from userinfo can pluck it out.
            if let Ok(userinfo_value) = serde_json::to_value(&user_claims) {
                merge_json(&mut raw_claims, userinfo_value);
            }
        }

        Ok(UpstreamUserInfo {
            provider: self.provider.slug.clone(),
            subject,
            email,
            email_verified,
            name,
            picture,
            raw_claims,
        })
    }

    /// Verify the upstream id_token against the cached signing keys,
    /// transparently refreshing the JWKS when it has gone stale.
    ///
    /// Two refresh triggers, matching the OIDC key-rotation guidance:
    ///  * **Proactive** — the cache TTL (from the certs endpoint's
    ///    `Cache-Control: max-age`) has lapsed, so we re-fetch before
    ///    even trying.
    ///  * **Reactive** — verification fails with `NoMatchingKey`: the
    ///    token's `kid` isn't one we hold, i.e. the upstream just
    ///    rotated. We re-fetch once (rate-limited) and retry.
    ///
    /// A proactive-refresh network failure is non-fatal — we fall back
    /// to the cached keys and let verification decide. Only a missing
    /// key triggers the reactive retry; other failures (bad audience,
    /// expired, genuinely bad signature) can't be fixed by re-fetching
    /// keys, so we surface them directly.
    async fn verify_id_token_claims<'t>(
        &self,
        id_token: &'t CoreIdToken,
        nonce: &Nonce,
    ) -> Result<&'t CoreIdTokenClaims> {
        let mut refreshed = false;
        let keys = if self.jwks.read().expires_at <= Instant::now() {
            match self.refresh_jwks().await {
                Ok(fresh) => {
                    refreshed = true;
                    fresh
                }
                Err(e) => {
                    tracing::warn!(
                        slug = %self.provider.slug,
                        error = %e,
                        "proactive jwks refresh failed; verifying against cached keys"
                    );
                    self.jwks.read().keys.clone()
                }
            }
        } else {
            self.jwks.read().keys.clone()
        };

        let verifier = self.id_token_verifier(keys)?;
        match id_token.claims(&verifier, nonce) {
            Ok(claims) => Ok(claims),
            Err(e)
                if is_unknown_signing_key(&e) && !refreshed && self.reactive_refetch_allowed() =>
            {
                tracing::info!(
                    slug = %self.provider.slug,
                    "id_token kid not in cached jwks; refetching upstream keys (likely rotation)"
                );
                let fresh = self.refresh_jwks().await?;
                let verifier = self.id_token_verifier(fresh)?;
                id_token
                    .claims(&verifier, nonce)
                    .map_err(|e| Error::Oidc(format!("id_token verify: {e}")))
            }
            Err(e) => Err(Error::Oidc(format!("id_token verify: {e}"))),
        }
    }

    /// Build an id_token verifier from `keys`, mirroring the
    /// public/confidential-client choice and allowed-algorithms set that
    /// [`CoreClient::id_token_verifier`] would have produced from the
    /// discovery metadata.
    fn id_token_verifier(&self, keys: Vec<CoreJsonWebKey>) -> Result<CoreIdTokenVerifier<'static>> {
        let issuer = IssuerUrl::new(self.provider.issuer.clone())
            .map_err(|e| Error::Oidc(format!("issuer url {}: {e}", self.provider.issuer)))?;
        let client_id = ClientId::new(self.provider.client_id.clone());
        let jwks = CoreJsonWebKeySet::new(keys);
        let verifier = if self.provider.client_secret.is_empty() {
            CoreIdTokenVerifier::new_public_client(client_id, issuer, jwks)
        } else {
            CoreIdTokenVerifier::new_confidential_client(
                client_id,
                ClientSecret::new(self.provider.client_secret.clone()),
                issuer,
                jwks,
            )
        };
        Ok(verifier.set_allowed_algs(self.signing_algs.clone()))
    }

    /// Whether enough time has elapsed since the last fetch to permit a
    /// reactive (unknown-`kid`) refetch.
    fn reactive_refetch_allowed(&self) -> bool {
        self.jwks.read().last_fetch.elapsed() >= JWKS_MIN_REFETCH_INTERVAL
    }

    /// Re-fetch the upstream JWKS, store it (with a TTL derived from the
    /// certs endpoint's `Cache-Control: max-age`), and return the fresh
    /// keys.
    async fn refresh_jwks(&self) -> Result<Vec<CoreJsonWebKey>> {
        let uri = self.jwks_uri.url().to_string();
        let resp = self
            .jwks_http
            .get(&uri)
            .send()
            .await
            .map_err(|e| Error::Oidc(format!("fetch jwks {uri}: {e}")))?
            .error_for_status()
            .map_err(|e| Error::Oidc(format!("fetch jwks {uri}: {e}")))?;
        let ttl = jwks_cache_ttl(resp.headers());
        let fetched: CoreJsonWebKeySet = resp
            .json()
            .await
            .map_err(|e| Error::Oidc(format!("parse jwks {uri}: {e}")))?;
        let keys = fetched.keys().clone();

        let now = Instant::now();
        {
            let mut cache = self.jwks.write();
            cache.keys = keys.clone();
            cache.expires_at = now + ttl;
            cache.last_fetch = now;
        }
        tracing::debug!(
            slug = %self.provider.slug,
            ttl_secs = ttl.as_secs(),
            keys = keys.len(),
            "refreshed upstream jwks"
        );
        Ok(keys)
    }
}

/// Result of [`OidcClient::start_login`]. The HTTP layer redirects the
/// user to `url` and persists the rest server-side (typically in the
/// session payload, keyed by `csrf_token` so the callback can look the
/// in-progress login up via the `state` query param).
pub struct StartedLogin {
    pub url: Url,
    pub csrf_token: CsrfToken,
    pub nonce: Nonce,
    pub pkce_verifier: PkceCodeVerifier,
}

/// Slug-keyed registry of discovered upstreams.
///
/// Cheap to clone — interior is `Arc<RwLock<…>>` so HTTP handlers can
/// share a single registry while admin endpoints add / remove providers
/// at runtime.
#[derive(Clone, Default)]
pub struct OidcRegistry {
    inner: Arc<RwLock<HashMap<String, Arc<OidcClient>>>>,
}

impl OidcRegistry {
    /// Empty registry — engine boot creates one of these and feeds it to
    /// [`crate::ctx::AuthCtx::with_oidc`]. Providers are added later via
    /// admin CRUD or seed config.
    pub fn new() -> Self {
        Self::default()
    }

    /// Discover and cache one upstream. Performs a network round-trip to
    /// `<issuer>/.well-known/openid-configuration` plus the JWKS fetch,
    /// so call this from boot or from an admin endpoint, not from a
    /// per-request handler.
    ///
    /// `redirect_uri` is the absolute URL the upstream redirects back
    /// to after login (typically `<public_url>/login/<slug>/callback`).
    pub async fn add(&self, provider: UpstreamProvider, redirect_uri: Url) -> Result<()> {
        let issuer = IssuerUrl::new(provider.issuer.clone())
            .map_err(|e| Error::Oidc(format!("issuer url {}: {e}", provider.issuer)))?;
        let http = build_oidc_http_client(HttpClientOptions::default())?;
        let metadata = CoreProviderMetadata::discover_async(issuer, &http)
            .await
            .map_err(|e| Error::Oidc(format!("discover {}: {e}", provider.slug)))?;
        // Capture the bits we need to rebuild an id_token verifier later
        // (on JWKS refresh) before `from_provider_metadata` consumes the
        // metadata. `discover_async` already fetched the JWKS into the
        // metadata, so `initial_keys` is a valid seed for the cache.
        let jwks_uri = metadata.jwks_uri().clone();
        let signing_algs = metadata.id_token_signing_alg_values_supported().clone();
        let initial_keys = metadata.jwks().keys().clone();
        let redirect = RedirectUrl::new(redirect_uri.to_string())
            .map_err(|e| Error::Oidc(format!("redirect_uri {redirect_uri}: {e}")))?;
        let client_secret = if provider.client_secret.is_empty() {
            None
        } else {
            Some(ClientSecret::new(provider.client_secret.clone()))
        };
        let inner = CoreClient::from_provider_metadata(
            metadata,
            ClientId::new(provider.client_id.clone()),
            client_secret,
        )
        .set_redirect_uri(redirect.clone());
        let jwks_http = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| Error::Oidc(format!("build jwks http client: {e}")))?;
        let now = Instant::now();
        let client = OidcClient {
            inner,
            provider: provider.clone(),
            redirect_uri: redirect,
            jwks_uri,
            signing_algs,
            jwks: Arc::new(RwLock::new(JwksCache {
                keys: initial_keys,
                expires_at: now + JWKS_DEFAULT_TTL,
                last_fetch: now,
            })),
            jwks_http,
        };
        self.inner
            .write()
            .insert(provider.slug.clone(), Arc::new(client));
        Ok(())
    }

    /// Look up a discovered upstream by slug. Returns the same `Arc`
    /// stored at registration time so callers can hold the client for
    /// the duration of a long-running flow.
    pub fn client(&self, slug: &str) -> Option<Arc<OidcClient>> {
        self.inner.read().get(slug).cloned()
    }

    /// List the slugs of every registered provider (for admin /
    /// debugging UIs).
    pub fn slugs(&self) -> Vec<String> {
        self.inner.read().keys().cloned().collect()
    }

    /// Remove a provider from the registry. Returns `true` if a row was
    /// dropped. Pending in-flight logins keep working because they hold
    /// an `Arc<OidcClient>` from before the removal.
    pub fn remove(&self, slug: &str) -> bool {
        self.inner.write().remove(slug).is_some()
    }

    /// Number of registered providers — handy for tests + metrics.
    pub fn len(&self) -> usize {
        self.inner.read().len()
    }

    /// Whether the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.inner.read().is_empty()
    }
}

/// Tunables for the discovery / token / userinfo HTTP client.
///
/// Defaults: 5s connect timeout, 10s overall request timeout.
#[derive(Clone, Debug)]
pub struct HttpClientOptions {
    pub connect_timeout_secs: u64,
    pub request_timeout_secs: u64,
}

impl Default for HttpClientOptions {
    fn default() -> Self {
        // TODO: plumb via [auth.oidc] config (discovery_connect_timeout_secs,
        //       discovery_request_timeout_secs).
        Self {
            connect_timeout_secs: 5,
            request_timeout_secs: 10,
        }
    }
}

/// Build the reqwest client `openidconnect` uses for discovery, token
/// exchange, JWKS fetches, and userinfo. We disable redirects on the
/// security advice in the [`openidconnect`] crate docs (SSRF mitigation)
/// and use rustls — matches the rest of assay's HTTP stack.
///
/// Plumbs explicit connect/read timeouts so a stored `issuer` pointing
/// at a hung endpoint can't pin the engine on boot or admin upsert.
/// SSRF protection is handled at the literal-host layer in
/// [`crate::oidc_provider::issuer_validation`].
pub fn build_oidc_http_client(opts: HttpClientOptions) -> Result<oidc_reqwest::Client> {
    oidc_reqwest::ClientBuilder::new()
        .redirect(oidc_reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(opts.connect_timeout_secs))
        .timeout(Duration::from_secs(opts.request_timeout_secs))
        .build()
        .map_err(|e| Error::Oidc(format!("build oidc http client: {e}")))
}

/// TTL for a freshly-fetched JWKS, parsed from `Cache-Control: max-age`
/// and clamped to `[JWKS_MIN_TTL, JWKS_MAX_TTL]`. Falls back to
/// [`JWKS_DEFAULT_TTL`] when no usable directive is present.
fn jwks_cache_ttl(headers: &reqwest::header::HeaderMap) -> Duration {
    headers
        .get(reqwest::header::CACHE_CONTROL)
        .and_then(|v| v.to_str().ok())
        .and_then(parse_max_age_secs)
        .map(|secs| Duration::from_secs(secs).clamp(JWKS_MIN_TTL, JWKS_MAX_TTL))
        .unwrap_or(JWKS_DEFAULT_TTL)
}

/// Pull the `max-age` value (in seconds) out of a `Cache-Control`
/// header. Returns `None` if there's no `max-age` directive (e.g.
/// `s-maxage` only, or `no-cache`).
fn parse_max_age_secs(cache_control: &str) -> Option<u64> {
    cache_control.split(',').find_map(|directive| {
        directive
            .trim()
            .strip_prefix("max-age")?
            .trim_start()
            .strip_prefix('=')?
            .trim()
            .parse::<u64>()
            .ok()
    })
}

/// Whether an id_token verification error means the signing key wasn't
/// found — i.e. the token's `kid` isn't in our cached JWKS, the
/// fingerprint of upstream key rotation. Other verification failures
/// (bad audience, expired, genuinely bad signature) are *not* fixable by
/// re-fetching keys, so we don't retry on them.
fn is_unknown_signing_key(e: &ClaimsVerificationError) -> bool {
    matches!(
        e,
        ClaimsVerificationError::SignatureVerification(SignatureVerificationError::NoMatchingKey)
    )
}

/// Recursive merge of two JSON values — used so userinfo claims top up
/// the id_token claims without overwriting them. Object fields merge
/// recursively; everything else is replaced.
fn merge_json(target: &mut serde_json::Value, src: serde_json::Value) {
    match (target, src) {
        (serde_json::Value::Object(a), serde_json::Value::Object(b)) => {
            for (k, v) in b {
                merge_json(a.entry(k).or_insert(serde_json::Value::Null), v);
            }
        }
        (slot, src) => {
            // Don't clobber a non-null target with a null source — the
            // id_token's value wins when userinfo doesn't add anything.
            if !src.is_null() {
                *slot = src;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_starts_empty() {
        let reg = OidcRegistry::new();
        assert!(reg.is_empty());
        assert_eq!(reg.len(), 0);
        assert!(reg.client("google").is_none());
        assert!(reg.slugs().is_empty());
    }

    #[test]
    fn merge_json_merges_objects_and_keeps_existing_on_null() {
        let mut a = serde_json::json!({"email": "a@x", "groups": ["a"]});
        let b = serde_json::json!({"email": serde_json::Value::Null, "name": "Alice"});
        merge_json(&mut a, b);
        assert_eq!(a["email"], "a@x");
        assert_eq!(a["name"], "Alice");
        assert_eq!(a["groups"], serde_json::json!(["a"]));
    }

    #[test]
    fn upstream_provider_record_is_clonable() {
        let p = UpstreamProvider {
            slug: "google".to_string(),
            issuer: "https://accounts.google.com".to_string(),
            client_id: "client".to_string(),
            client_secret: "secret".to_string(),
            scopes: vec!["openid".to_string(), "email".to_string()],
            auth_params: BTreeMap::new(),
        };
        let dup = p.clone();
        assert_eq!(p, dup);
    }

    #[test]
    fn parses_max_age_from_cache_control() {
        assert_eq!(parse_max_age_secs("max-age=3600"), Some(3600));
        assert_eq!(
            parse_max_age_secs("public, max-age=600, must-revalidate"),
            Some(600)
        );
        assert_eq!(parse_max_age_secs("max-age = 42"), Some(42));
        assert_eq!(parse_max_age_secs("private, no-cache"), None);
        // `s-maxage` is a distinct directive — must not be mistaken for
        // `max-age`.
        assert_eq!(parse_max_age_secs("s-maxage=120"), None);
    }

    #[test]
    fn jwks_ttl_clamps_and_defaults() {
        use reqwest::header::{CACHE_CONTROL, HeaderMap, HeaderValue};

        // No header → default.
        assert_eq!(jwks_cache_ttl(&HeaderMap::new()), JWKS_DEFAULT_TTL);

        let ttl = |v: &'static str| {
            let mut h = HeaderMap::new();
            h.insert(CACHE_CONTROL, HeaderValue::from_static(v));
            jwks_cache_ttl(&h)
        };
        // Below floor → clamped up; above ceiling → clamped down.
        assert_eq!(ttl("max-age=10"), JWKS_MIN_TTL);
        assert_eq!(ttl("public, max-age=999999"), JWKS_MAX_TTL);
        // In range → passthrough.
        assert_eq!(ttl("max-age=1800"), Duration::from_secs(1800));
    }

    #[test]
    fn only_missing_key_triggers_refetch() {
        // A missing `kid` (rotation) is retryable...
        assert!(is_unknown_signing_key(
            &ClaimsVerificationError::SignatureVerification(
                SignatureVerificationError::NoMatchingKey
            )
        ));
        // ...but a genuine bad signature or a non-signature failure is
        // not — re-fetching keys wouldn't help.
        assert!(!is_unknown_signing_key(
            &ClaimsVerificationError::SignatureVerification(
                SignatureVerificationError::CryptoError("bad sig".into())
            )
        ));
        assert!(!is_unknown_signing_key(&ClaimsVerificationError::Expired(
            "stale".into()
        )));
    }

    /// Discovery against an unreachable URL should fail with `Error::Oidc`,
    /// not panic. We don't network out from unit tests; this just exercises
    /// the error path.
    #[tokio::test]
    async fn discover_unreachable_issuer_returns_oidc_error() {
        let reg = OidcRegistry::new();
        let provider = UpstreamProvider {
            slug: "ghost".to_string(),
            issuer: "http://127.0.0.1:1/oidc".to_string(),
            client_id: "client".to_string(),
            client_secret: "secret".to_string(),
            scopes: vec!["openid".to_string()],
            auth_params: BTreeMap::new(),
        };
        let redirect = Url::parse("https://example.com/login/ghost/callback").unwrap();
        let result = reg.add(provider, redirect).await;
        assert!(matches!(result, Err(Error::Oidc(_))));
    }
}
