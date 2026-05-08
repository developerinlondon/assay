//! Upstream-OIDC federation — `start_upstream_login` +
//! `complete_upstream_login`.
//!
//! Plan note: when a user signs in via the upstream, the upstream
//! authenticates them; the engine creates its own user record linked to
//! the upstream identity (via `auth.user_upstream`) and issues **its
//! own** id_token to the consumer app. Consumers never see the upstream
//! directly.
//!
//! This module handles the upstream leg only — minting the redirect
//! URL, persisting the in-flight state row, and reconciling on
//! callback. The actual sign-in (creating an assay session, resuming
//! the consumer's `/authorize`) happens in the route layer.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use openidconnect::CsrfToken;

use crate::error::{Error, Result};
use crate::oidc::OidcRegistry;

use super::binding;
use super::store::OidcUpstreamStateStore;
use super::types::UpstreamLoginState;

/// Lifetime of an in-flight upstream-login state row — five minutes.
/// Long enough for a real human to bounce through the upstream's
/// consent screen, short enough that abandoned flows don't pile up
/// forever.
pub const UPSTREAM_STATE_LIFETIME_SECS: f64 = 300.0;

/// Result of `start_upstream_login` — the URL to redirect the user to,
/// the `state` value the callback will use to look the row up, and the
/// raw binding token the route layer pins on the response cookie.
#[derive(Clone, Debug)]
pub struct StartedUpstreamLogin {
    pub redirect_url: String,
    pub state: String,
    pub binding_token: String,
}

/// Kick off an upstream OIDC login. Looks up `provider_slug` in the
/// in-memory [`OidcRegistry`] (loaded from `auth.upstream_providers`
/// on boot), generates a fresh PKCE pair + binding token, persists the
/// state row, and returns the redirect URL plus the raw binding token
/// for the cookie.
///
/// `return_to` is the consumer's `/authorize` URL the user was on
/// before federation kicked in — restored after the callback.
pub async fn start_upstream_login(
    registry: &OidcRegistry,
    state_store: &Arc<dyn OidcUpstreamStateStore>,
    provider_slug: &str,
    return_to: Option<String>,
) -> Result<StartedUpstreamLogin> {
    let client = registry
        .client(provider_slug)
        .ok_or_else(|| Error::Oidc(format!("unknown upstream provider {provider_slug}")))?;

    // We let the openidconnect library generate the actual state value
    // it bakes into the URL; we re-read it via `csrf_token` on the
    // resulting StartedLogin so the row we persist matches the URL
    // exactly.
    let started = client.start_login(CsrfToken::new_random());
    let state_string = started.csrf_token.secret().clone();
    let nonce_string = started.nonce.secret().clone();
    let pkce_verifier = started.pkce_verifier.secret().clone();
    let (binding_token, binding_hash) = binding::generate();
    let now = now_secs();

    state_store
        .create(&UpstreamLoginState {
            state: state_string.clone(),
            provider_slug: provider_slug.to_string(),
            nonce: nonce_string,
            pkce_verifier,
            return_to,
            created_at: now,
            expires_at: now + UPSTREAM_STATE_LIFETIME_SECS,
            binding_hash,
        })
        .await
        .map_err(Error::Backend)?;

    Ok(StartedUpstreamLogin {
        redirect_url: started.url.to_string(),
        state: state_string,
        binding_token,
    })
}

/// Outcome of a completed upstream login — minimum fields the caller
/// needs to upsert the assay user + link the upstream identity.
#[derive(Clone, Debug)]
pub struct CompletedUpstreamLogin {
    pub provider_slug: String,
    pub subject: String,
    pub email: Option<String>,
    pub email_verified: bool,
    pub display_name: Option<String>,
    pub return_to: Option<String>,
}

/// Look up the in-flight state row, run the binding + RFC 9207 `iss`
/// checks, exchange the upstream code, and return the verified upstream
/// userinfo. The caller persists the `auth.users` + `auth.user_upstream`
/// rows and creates an assay session.
///
/// `binding_token` is the raw cookie value from `assay_oidc_binding`;
/// `iss` is the optional `iss` query param per RFC 9207. Both checks
/// happen *before* the upstream code-exchange so a stolen `code` is
/// never spent.
pub async fn complete_upstream_login(
    registry: &OidcRegistry,
    state_store: &Arc<dyn OidcUpstreamStateStore>,
    code: &str,
    state: &str,
    binding_token: Option<&str>,
    iss: Option<&str>,
) -> Result<CompletedUpstreamLogin> {
    let row = state_store
        .take(state)
        .await
        .map_err(Error::Backend)?
        .ok_or_else(|| Error::Oidc("upstream state unknown or already consumed".to_string()))?;
    if row.expires_at <= now_secs() {
        return Err(Error::Oidc("upstream state expired".to_string()));
    }

    if row.binding_hash.is_empty() {
        // Migration sentinel — pre-deploy in-flight row. Skip the check
        // (bounded by the row's TTL) and warn so operators see the bypass
        // window in the logs.
        tracing::warn!(
            slug = %row.provider_slug,
            "upstream login state predates binding migration; skipping CSRF binding check"
        );
    } else {
        // Single cookie per browser — a second concurrent /start in
        // another tab clobbers the first cookie. The first tab's
        // callback then lands here with a hash that doesn't match,
        // and we surface that case in the error so operators don't
        // chase a phantom CSRF mismatch.
        let raw =
            binding_token.ok_or_else(|| Error::Oidc("oidc state binding missing".to_string()))?;
        if !binding::verify(raw, &row.binding_hash) {
            return Err(Error::Oidc(
                "oidc state binding mismatch (cookie may have been overwritten by a \
                 concurrent flow; retry login)"
                    .to_string(),
            ));
        }
    }

    let client = registry
        .client(&row.provider_slug)
        .ok_or_else(|| Error::Oidc(format!("unknown upstream provider {}", row.provider_slug)))?;

    match iss {
        Some(got) => {
            let expected = &client.provider().issuer;
            if got != expected {
                return Err(Error::Oidc(format!(
                    "issuer mismatch: expected {expected}, got {got}"
                )));
            }
        }
        None => {
            tracing::warn!(slug = %row.provider_slug, "upstream did not return iss param");
        }
    }

    use openidconnect::{Nonce, PkceCodeVerifier};
    let info = client
        .complete_login(
            code.to_string(),
            PkceCodeVerifier::new(row.pkce_verifier),
            Nonce::new(row.nonce),
        )
        .await?;

    Ok(CompletedUpstreamLogin {
        provider_slug: row.provider_slug,
        subject: info.subject,
        email: info.email,
        email_verified: info.email_verified,
        display_name: info.name,
        return_to: row.return_to,
    })
}

fn now_secs() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal in-memory state store — enough to exercise the row
    /// shape contract without touching sqlx.
    struct MemStore(parking_lot::Mutex<std::collections::HashMap<String, UpstreamLoginState>>);

    #[async_trait::async_trait]
    impl OidcUpstreamStateStore for MemStore {
        async fn create(&self, s: &UpstreamLoginState) -> anyhow::Result<()> {
            self.0.lock().insert(s.state.clone(), s.clone());
            Ok(())
        }
        async fn take(&self, s: &str) -> anyhow::Result<Option<UpstreamLoginState>> {
            Ok(self.0.lock().remove(s))
        }
    }

    fn mem_store() -> Arc<dyn OidcUpstreamStateStore> {
        Arc::new(MemStore(parking_lot::Mutex::new(Default::default())))
    }

    #[tokio::test]
    async fn complete_with_unknown_state_errors() {
        let registry = OidcRegistry::new();
        let store = mem_store();
        let result =
            complete_upstream_login(&registry, &store, "code_abc", "state_unknown", None, None)
                .await;
        assert!(matches!(result, Err(Error::Oidc(_))));
    }

    #[tokio::test]
    async fn start_with_unknown_provider_errors() {
        let registry = OidcRegistry::new();
        let store = mem_store();
        let result = start_upstream_login(&registry, &store, "nonexistent", None).await;
        assert!(matches!(result, Err(Error::Oidc(_))));
    }

    #[tokio::test]
    async fn binding_missing_errors_when_row_has_hash() {
        let registry = OidcRegistry::new();
        let store = mem_store();
        let (_raw, hash) = binding::generate();
        store
            .create(&UpstreamLoginState {
                state: "s1".into(),
                provider_slug: "google".into(),
                nonce: "n".into(),
                pkce_verifier: "v".into(),
                return_to: None,
                created_at: now_secs(),
                expires_at: now_secs() + 300.0,
                binding_hash: hash,
            })
            .await
            .unwrap();
        let result = complete_upstream_login(&registry, &store, "c", "s1", None, None).await;
        assert!(matches!(result, Err(Error::Oidc(ref m)) if m.contains("binding missing")));
    }

    #[tokio::test]
    async fn binding_mismatch_errors() {
        let registry = OidcRegistry::new();
        let store = mem_store();
        let (_raw, hash) = binding::generate();
        store
            .create(&UpstreamLoginState {
                state: "s1".into(),
                provider_slug: "google".into(),
                nonce: "n".into(),
                pkce_verifier: "v".into(),
                return_to: None,
                created_at: now_secs(),
                expires_at: now_secs() + 300.0,
                binding_hash: hash,
            })
            .await
            .unwrap();
        let result =
            complete_upstream_login(&registry, &store, "c", "s1", Some("wrong_token"), None).await;
        assert!(matches!(result, Err(Error::Oidc(ref m)) if m.contains("binding mismatch")));
    }

    #[tokio::test]
    async fn binding_sentinel_skips_check() {
        let registry = OidcRegistry::new();
        let store = mem_store();
        store
            .create(&UpstreamLoginState {
                state: "s1".into(),
                provider_slug: "ghost".into(),
                nonce: "n".into(),
                pkce_verifier: "v".into(),
                return_to: None,
                created_at: now_secs(),
                expires_at: now_secs() + 300.0,
                binding_hash: String::new(),
            })
            .await
            .unwrap();
        // Without a registered provider, the call still proceeds past
        // the binding check and fails on the registry lookup — proves
        // the sentinel branch is reached.
        let result = complete_upstream_login(&registry, &store, "c", "s1", None, None).await;
        assert!(
            matches!(result, Err(Error::Oidc(ref m)) if m.contains("unknown upstream provider"))
        );
    }
}
