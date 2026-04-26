//! Composed auth context — the value engine state holds for the auth
//! module.
//!
//! Phase 4 wires user/session stores and (when JWT is enabled) the
//! [`crate::jwt::JwtConfig`]. Later phases extend this with the
//! Zanzibar store and OIDC provider registry. The struct is `Clone`
//! because axum's `FromRef` model requires it.

use std::sync::Arc;

use crate::biscuit::BiscuitConfig;
use crate::store::{SessionStore, UserStore};

#[cfg(feature = "auth-jwt")]
use crate::jwt::JwtConfig;
#[cfg(feature = "auth-oidc")]
use crate::oidc::OidcRegistry;
#[cfg(feature = "auth-oidc-provider")]
use crate::oidc_provider::OidcProviderConfig;
#[cfg(feature = "auth-passkey")]
use crate::passkey::PasskeyManager;
#[cfg(feature = "auth-zanzibar")]
use crate::zanzibar::ZanzibarStore;

#[derive(Clone)]
pub struct AuthCtx {
    /// Authoritative user record store. Carries password hashes,
    /// upstream-provider links, and passkeys.
    pub users: Arc<dyn UserStore>,
    /// Session record store — opaque session id + CSRF token + expiry.
    pub sessions: Arc<dyn SessionStore>,
    /// Biscuit capability-token issuer + verifier. Foundational
    /// (always present): wraps the active root keypair loaded from
    /// `auth.biscuit_root_keys` (or generated on first boot). Used for
    /// share links, delegated upload caps, worker capability tokens,
    /// edge auth, and any flow that wants offline-verifiable bearer
    /// tokens. See [`crate::biscuit::BiscuitConfig`].
    pub biscuit: BiscuitConfig,
    /// JWT issuance/verification configuration. Active key + history;
    /// see [`crate::jwt::JwtConfig`]. Present only when the
    /// `auth-jwt` feature is enabled.
    #[cfg(feature = "auth-jwt")]
    pub jwt: Option<JwtConfig>,
    /// Slug-keyed registry of discovered upstream OIDC providers.
    /// Engine boot constructs an empty registry; admin CRUD (or seed
    /// config) populates it. See [`crate::oidc::OidcRegistry`].
    #[cfg(feature = "auth-oidc")]
    pub oidc: Option<OidcRegistry>,
    /// WebAuthn / passkey manager. Wraps a single
    /// [`webauthn_rs::Webauthn`] built from the operator's RP config.
    /// See [`crate::passkey::PasskeyManager`].
    #[cfg(feature = "auth-passkey")]
    pub passkeys: Option<PasskeyManager>,
    /// Zanzibar / ReBAC permission store. Optional — engine boot wires
    /// the appropriate backend (Postgres / SQLite) once the auth schema
    /// migration has run. See [`crate::zanzibar::ZanzibarStore`] for
    /// the trait surface; full Keto/SpiceDB feature parity (recursive
    /// CTE walk, expand, lookup_*) lives behind it.
    #[cfg(feature = "auth-zanzibar")]
    pub zanzibar: Option<Arc<dyn ZanzibarStore>>,
    /// Full OIDC provider — discovery, JWKS, /authorize, /token,
    /// /userinfo, /revoke, /introspect, federation. Optional because a
    /// deployment may use assay-engine purely as an OIDC client; engine
    /// boot constructs the config once the V4 migration has run and
    /// the upstream provider rows are loaded into the registry.
    #[cfg(feature = "auth-oidc-provider")]
    pub oidc_provider: Option<OidcProviderConfig>,
}

impl AuthCtx {
    /// Construct a context from the bare minimum required by phase 4 —
    /// user and session stores. Biscuit is initialised with an
    /// ephemeral keypair (no DB row) so unit tests + downstream callers
    /// that don't run engine boot can still construct an [`AuthCtx`].
    /// Engine boot replaces the biscuit field via
    /// [`AuthCtx::with_biscuit`] once the persistent root key has been
    /// loaded from `auth.biscuit_root_keys`.
    pub fn new(users: Arc<dyn UserStore>, sessions: Arc<dyn SessionStore>) -> Self {
        Self {
            users,
            sessions,
            biscuit: BiscuitConfig::generate_ephemeral(),
            #[cfg(feature = "auth-jwt")]
            jwt: None,
            #[cfg(feature = "auth-oidc")]
            oidc: None,
            #[cfg(feature = "auth-passkey")]
            passkeys: None,
            #[cfg(feature = "auth-zanzibar")]
            zanzibar: None,
            #[cfg(feature = "auth-oidc-provider")]
            oidc_provider: None,
        }
    }

    /// Replace the JWT configuration. Used by engine boot once the
    /// JWKS keys have been loaded from `auth.jwks_keys`.
    #[cfg(feature = "auth-jwt")]
    pub fn with_jwt(mut self, jwt: JwtConfig) -> Self {
        self.jwt = Some(jwt);
        self
    }

    /// Replace the OIDC registry. Engine boot creates an empty registry
    /// for unconfigured deployments; once admin CRUD lands, the same
    /// builder runs after the seed providers are loaded.
    #[cfg(feature = "auth-oidc")]
    pub fn with_oidc(mut self, oidc: OidcRegistry) -> Self {
        self.oidc = Some(oidc);
        self
    }

    /// Replace the passkey manager. Optional — the manager owns a live
    /// [`webauthn_rs::Webauthn`] built from the engine's RP config and
    /// is only constructible when that config is present.
    #[cfg(feature = "auth-passkey")]
    pub fn with_passkeys(mut self, passkeys: PasskeyManager) -> Self {
        self.passkeys = Some(passkeys);
        self
    }

    /// Replace the biscuit configuration. Engine boot loads the active
    /// root key from `auth.biscuit_root_keys` (or generates one on
    /// first boot) and feeds the result here.
    pub fn with_biscuit(mut self, biscuit: BiscuitConfig) -> Self {
        self.biscuit = biscuit;
        self
    }

    /// Replace the Zanzibar store. Engine boot constructs the
    /// appropriate backend impl after the auth schema migration runs;
    /// see `crates/assay-engine/src/init.rs`. Phase 6 only wires the
    /// builder + the migration; full AuthCtx composition happens in
    /// phase 8 alongside HTTP route mounting.
    #[cfg(feature = "auth-zanzibar")]
    pub fn with_zanzibar(mut self, zanzibar: Arc<dyn ZanzibarStore>) -> Self {
        self.zanzibar = Some(zanzibar);
        self
    }

    /// Replace the OIDC provider configuration. Engine boot constructs
    /// the appropriate stores (PG / SQLite) after the V4 auth schema
    /// migration runs; see `crates/assay-engine/src/init.rs`.
    /// only wires the builder + the migrations + the placeholder
    /// router; phase 8 weaves the resolved AuthCtx into the actual
    /// `/authorize` and `/token` HTTP handlers.
    #[cfg(feature = "auth-oidc-provider")]
    pub fn with_oidc_provider(mut self, oidc_provider: OidcProviderConfig) -> Self {
        self.oidc_provider = Some(oidc_provider);
        self
    }
}
