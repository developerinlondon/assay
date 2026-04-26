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

use std::collections::HashMap;
use std::sync::Arc;

use openidconnect::core::{
    CoreAuthenticationFlow, CoreClient, CoreProviderMetadata, CoreUserInfoClaims,
};
use openidconnect::reqwest as oidc_reqwest;
use openidconnect::{
    AuthorizationCode, ClientId, ClientSecret, CsrfToken, EndpointMaybeSet, EndpointNotSet,
    EndpointSet, IssuerUrl, Nonce, OAuth2TokenResponse, PkceCodeChallenge, PkceCodeVerifier,
    RedirectUrl, Scope, SubjectIdentifier, TokenResponse,
};
use parking_lot::RwLock;
use url::Url;

use crate::error::{Error, Result};

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
    pub fn start_login(
        &self,
        state: CsrfToken,
    ) -> StartedLogin {
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
        let http = build_oidc_http_client()?;
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
        let id_token_verifier = self.inner.id_token_verifier();
        let claims = id_token
            .claims(&id_token_verifier, &nonce)
            .map_err(|e| Error::Oidc(format!("id_token verify: {e}")))?;

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
        let mut raw_claims = serde_json::to_value(claims)
            .unwrap_or_else(|_| serde_json::json!({"sub": subject}));
        if let Ok(req) = self
            .inner
            .user_info(token_response.access_token().clone(), Some(SubjectIdentifier::new(subject.clone())))
            && let Ok(userinfo) = req.request_async(&http).await
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
        let http = build_oidc_http_client()?;
        let metadata = CoreProviderMetadata::discover_async(issuer, &http)
            .await
            .map_err(|e| Error::Oidc(format!("discover {}: {e}", provider.slug)))?;
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
        let client = OidcClient {
            inner,
            provider: provider.clone(),
            redirect_uri: redirect,
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

/// Build the reqwest client `openidconnect` uses for discovery, token
/// exchange, JWKS fetches, and userinfo. We disable redirects on the
/// security advice in the [`openidconnect`] crate docs (SSRF mitigation)
/// and use rustls — matches the rest of assay's HTTP stack.
fn build_oidc_http_client() -> Result<oidc_reqwest::Client> {
    oidc_reqwest::ClientBuilder::new()
        .redirect(oidc_reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| Error::Oidc(format!("build oidc http client: {e}")))
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
        };
        let dup = p.clone();
        assert_eq!(p, dup);
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
        };
        let redirect = Url::parse("https://example.com/login/ghost/callback").unwrap();
        let result = reg.add(provider, redirect).await;
        assert!(matches!(result, Err(Error::Oidc(_))));
    }
}
