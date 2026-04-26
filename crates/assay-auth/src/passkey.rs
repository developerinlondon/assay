//! WebAuthn / passkey registration + authentication.
//!
//! Plan 12c task 5.2 reference. Wraps [`webauthn_rs`] 0.5 so HTTP
//! handlers can drive register / authenticate without
//! touching the library's verbose builder surface.
//!
//! In-progress state ([`PasskeyRegistration`], [`PasskeyAuthentication`])
//! is short-lived (~5 min). For phase 5 the manager just returns it; the
//! caller (phase 8 HTTP handlers) parks it however they want — typically
//! the session payload. A dedicated table is overkill for state that
//! lives less than a request round-trip.

use std::sync::Arc;

use url::Url;
use uuid::Uuid;
use webauthn_rs::Webauthn;
use webauthn_rs::prelude::{
    CreationChallengeResponse, Passkey, PasskeyAuthentication, PasskeyRegistration,
    PublicKeyCredential, RegisterPublicKeyCredential, RequestChallengeResponse,
    WebauthnBuilder,
};

use crate::error::{Error, Result};
use crate::store::UserStore;

/// Operator-supplied relying-party config. `rp_id` is the host (no
/// scheme, no port — e.g. `"app.example.com"`); `rp_name` is the
/// human-readable label browsers show; `origin` is the canonical URL of
/// the page that hosts the WebAuthn JS.
///
/// All three come from `engine.toml` so a deployment can run multiple
/// engines behind one RP id without each rebuilding the wiring.
#[derive(Clone, Debug)]
pub struct PasskeyConfig {
    pub rp_id: String,
    pub rp_name: String,
    pub origin: Url,
}

/// Owns the [`Webauthn`] instance + the user store the manager needs to
/// look up existing credentials for the authenticate flow.
///
/// Cheap to clone — both fields are reference-counted.
#[derive(Clone)]
pub struct PasskeyManager {
    webauthn: Arc<Webauthn>,
    users: Arc<dyn UserStore>,
    config: PasskeyConfig,
}

impl PasskeyManager {
    /// Build the manager from operator config + the auth user store.
    /// Errors if the rp_id / origin fail [`webauthn_rs`]'s validation
    /// (e.g. mismatched host, missing TLD on a bare `localhost`-ish
    /// origin in production).
    pub fn new(config: PasskeyConfig, users: Arc<dyn UserStore>) -> Result<Self> {
        let webauthn = WebauthnBuilder::new(&config.rp_id, &config.origin)
            .map_err(|e| Error::Passkey(format!("WebauthnBuilder::new: {e}")))?
            .rp_name(&config.rp_name)
            .build()
            .map_err(|e| Error::Passkey(format!("WebauthnBuilder::build: {e}")))?;
        Ok(Self {
            webauthn: Arc::new(webauthn),
            users,
            config,
        })
    }

    /// Borrow the operator config — handy for `/well-known/...` style
    /// admin endpoints + tests.
    pub fn config(&self) -> &PasskeyConfig {
        &self.config
    }

    /// Borrow the underlying user store. Phase 8 handlers may need it
    /// directly when they upsert the resulting passkey via
    /// [`UserStore::add_passkey`].
    pub fn users(&self) -> &Arc<dyn UserStore> {
        &self.users
    }

    /// Step 1 of registration. Returns the challenge to ship to the
    /// browser plus the in-progress state to round-trip via the session.
    /// The state is short-lived; do NOT persist it long-term.
    ///
    /// `user_unique_id` is the [`Uuid`] [`webauthn_rs`] uses internally
    /// — typically a deterministic UUIDv5 derived from the user's
    /// `auth.users.id` (or any stable opaque id mapped to UUID space).
    /// `user_name` is the WebAuthn "name" (typically the email);
    /// `display_name` is the human-readable label.
    ///
    /// `auth_user_id` is the canonical opaque id stored on
    /// `auth.users.id` — used to look up existing passkeys so the
    /// browser can exclude them from the prompt. Pass `None` for fresh
    /// signups where no row exists yet.
    pub async fn start_registration(
        &self,
        user_unique_id: Uuid,
        user_name: &str,
        display_name: &str,
        auth_user_id: Option<&str>,
    ) -> Result<(CreationChallengeResponse, PasskeyRegistration)> {
        // Pre-load the user's existing passkeys so the browser can
        // exclude them from the prompt (avoids a duplicate-credential
        // attestation error). Failing this lookup is non-fatal — we just
        // skip exclusion and let webauthn-rs' own duplicate detection
        // catch it on `finish_registration`.
        let exclude = if let Some(uid) = auth_user_id {
            self.users
                .list_passkeys(uid)
                .await
                .map(|creds| {
                    creds
                        .into_iter()
                        .map(|c| c.credential_id.into())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let exclude = if exclude.is_empty() {
            None
        } else {
            Some(exclude)
        };
        self.webauthn
            .start_passkey_registration(user_unique_id, user_name, display_name, exclude)
            .map_err(|e| Error::Passkey(format!("start_passkey_registration: {e}")))
    }

    /// Step 2 of registration. Verifies the browser's
    /// [`RegisterPublicKeyCredential`] against the stored
    /// [`PasskeyRegistration`] state and returns the
    /// [`webauthn_rs::prelude::Passkey`] for the caller to persist via
    /// [`UserStore::add_passkey`].
    ///
    /// We return the library's `Passkey` rather than our
    /// [`crate::store::PasskeyCred`] so handlers can also stash the
    /// serialised form for later re-verification — converting via
    /// [`passkey_to_cred`] is a one-liner when persistence is wanted.
    pub fn finish_registration(
        &self,
        state: &PasskeyRegistration,
        response: &RegisterPublicKeyCredential,
    ) -> Result<Passkey> {
        self.webauthn
            .finish_passkey_registration(response, state)
            .map_err(|e| Error::Passkey(format!("finish_passkey_registration: {e}")))
    }

    /// Step 1 of authentication. Loads the user's stored passkeys via
    /// [`UserStore::list_passkeys`] (caller passes the user_id) and
    /// asks [`webauthn_rs`] for a fresh challenge. Returns the challenge
    /// to ship to the browser plus the in-progress state to round-trip
    /// via the session.
    ///
    /// Errors with [`Error::Passkey`] when the user has no registered
    /// passkeys — callers should fall back to a different auth method
    /// instead of presenting an empty challenge.
    pub async fn start_authentication(
        &self,
        user_id: &str,
    ) -> Result<(RequestChallengeResponse, PasskeyAuthentication)> {
        let stored = self
            .users
            .list_passkeys(user_id)
            .await
            .map_err(|e| Error::Backend(anyhow::anyhow!("list_passkeys({user_id}): {e}")))?;
        if stored.is_empty() {
            return Err(Error::Passkey(format!(
                "no passkeys registered for user {user_id}"
            )));
        }
        // We don't persist the full `Passkey` blob in `auth.passkeys`
        // (only credential_id + public_key + sign_count), so we can't
        // round-trip a `webauthn_rs::Passkey` from the table without a
        // second column carrying the serialised form. For phase 5 we
        // raise a clear error; phase 8 will introduce that column when
        // it wires the actual HTTP handler. Until then, callers that
        // hold a freshly-registered Passkey can call
        // [`PasskeyManager::start_authentication_with`] directly.
        Err(Error::Passkey(format!(
            "passkey reauthentication needs the serialised Passkey blob (count={}); \
             use PasskeyManager::start_authentication_with after wiring `auth.passkeys.passkey_json`",
            stored.len()
        )))
    }

    /// Variant of [`PasskeyManager::start_authentication`] that takes
    /// the already-deserialised [`webauthn_rs::prelude::Passkey`] list
    /// directly. Useful for tests + for any future caller that holds the
    /// serialised blob outside of the canonical store layout.
    pub fn start_authentication_with(
        &self,
        creds: &[Passkey],
    ) -> Result<(RequestChallengeResponse, PasskeyAuthentication)> {
        if creds.is_empty() {
            return Err(Error::Passkey(
                "passkey list is empty; cannot start authentication".to_string(),
            ));
        }
        self.webauthn
            .start_passkey_authentication(creds)
            .map_err(|e| Error::Passkey(format!("start_passkey_authentication: {e}")))
    }

    /// Step 2 of authentication. Verifies the browser's
    /// [`PublicKeyCredential`] and returns the
    /// [`AuthenticatedPasskey`] result the caller persists (sign-count
    /// bump, backup-state changes, etc.) via the user store.
    pub fn finish_authentication(
        &self,
        state: &PasskeyAuthentication,
        response: &PublicKeyCredential,
    ) -> Result<AuthenticatedPasskey> {
        let result = self
            .webauthn
            .finish_passkey_authentication(response, state)
            .map_err(|e| Error::Passkey(format!("finish_passkey_authentication: {e}")))?;
        Ok(AuthenticatedPasskey {
            credential_id: result.cred_id().as_ref().to_vec(),
            sign_count: result.counter(),
            user_verified: result.user_verified(),
            needs_update: result.needs_update(),
        })
    }
}

/// Successful authentication result — carries the credential id the
/// caller looks up in `auth.passkeys`, plus the new sign-count the
/// caller persists (cheap UPDATE keyed on credential_id).
#[derive(Clone, Debug)]
pub struct AuthenticatedPasskey {
    /// Raw bytes of the verified credential id. Matches the
    /// `auth.passkeys.credential_id` primary key.
    pub credential_id: Vec<u8>,
    /// New sign-count from the authenticator. Spec requires the server
    /// to assert this is greater than the stored value — when it isn't,
    /// the caller MAY treat the credential as cloned and revoke it.
    pub sign_count: u32,
    /// Whether the user verified themselves on this authentication
    /// (PIN, biometric, …). Useful for step-up flows.
    pub user_verified: bool,
    /// `webauthn-rs` thinks the stored Passkey blob is out-of-date with
    /// respect to the new `AuthenticationResult` (counter or backup
    /// state changed). Re-persist via the user store when true.
    pub needs_update: bool,
}

/// Project a [`webauthn_rs::prelude::Passkey`] into a
/// [`crate::store::PasskeyCred`] for persistence in `auth.passkeys`.
/// Phase 5 lacks a place for the full serialised `Passkey` JSON blob
/// (the table doesn't have a payload column yet), so the projection is
/// lossy — re-authentication needs a future column to round-trip the
/// blob. Tests and admin tooling that just want to enumerate stored
/// credentials are fine with the projection.
pub fn passkey_to_cred(passkey: &Passkey, created_at: f64) -> crate::store::PasskeyCred {
    crate::store::PasskeyCred {
        credential_id: passkey.cred_id().as_ref().to_vec(),
        // Public key bytes aren't directly exposed by webauthn-rs'
        // public surface; we fall back to a JSON serialisation of the
        // COSE key for storage. Phase 8 may swap this for the raw COSE
        // bytes once the schema lands.
        public_key: serde_json::to_vec(passkey.get_public_key()).unwrap_or_default(),
        sign_count: 0,
        transports: Vec::new(),
        created_at,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::types::{PasskeyCred, Session, User};
    use crate::store::{SessionStore, UserStore};
    use std::collections::HashMap;
    use std::sync::Mutex;

    /// Trivial in-memory user store for unit tests — no persistence,
    /// just enough to satisfy the trait so `PasskeyManager::new` works.
    struct MemUserStore(Mutex<HashMap<String, Vec<PasskeyCred>>>);

    #[async_trait::async_trait]
    impl UserStore for MemUserStore {
        async fn create_user(&self, _user: &User) -> anyhow::Result<()> {
            Ok(())
        }
        async fn get_user_by_id(&self, _id: &str) -> anyhow::Result<Option<User>> {
            Ok(None)
        }
        async fn get_user_by_email(&self, _email: &str) -> anyhow::Result<Option<User>> {
            Ok(None)
        }
        async fn update_user(&self, _user: &User) -> anyhow::Result<()> {
            Ok(())
        }
        async fn set_password_hash(&self, _user_id: &str, _hash: &str) -> anyhow::Result<()> {
            Ok(())
        }
        async fn get_password_hash(&self, _user_id: &str) -> anyhow::Result<Option<String>> {
            Ok(None)
        }
        async fn list_passkeys(&self, user_id: &str) -> anyhow::Result<Vec<PasskeyCred>> {
            Ok(self.0.lock().unwrap().get(user_id).cloned().unwrap_or_default())
        }
        async fn add_passkey(
            &self,
            user_id: &str,
            cred: &PasskeyCred,
        ) -> anyhow::Result<()> {
            self.0
                .lock()
                .unwrap()
                .entry(user_id.to_string())
                .or_default()
                .push(cred.clone());
            Ok(())
        }
        async fn remove_passkey(&self, _credential_id: &[u8]) -> anyhow::Result<bool> {
            Ok(true)
        }
        async fn link_upstream(
            &self,
            _user_id: &str,
            _provider: &str,
            _subject: &str,
        ) -> anyhow::Result<()> {
            Ok(())
        }
        async fn get_user_by_upstream(
            &self,
            _provider: &str,
            _subject: &str,
        ) -> anyhow::Result<Option<User>> {
            Ok(None)
        }
        async fn list_users(
            &self,
            _limit: i64,
            _offset: i64,
            _search: Option<&str>,
        ) -> anyhow::Result<Vec<User>> {
            Ok(vec![])
        }
        async fn count_users(&self, _search: Option<&str>) -> anyhow::Result<i64> {
            Ok(0)
        }
        async fn delete_user(&self, _id: &str) -> anyhow::Result<bool> {
            Ok(false)
        }
        async fn list_upstream_for_user(
            &self,
            _user_id: &str,
        ) -> anyhow::Result<Vec<(String, String)>> {
            Ok(vec![])
        }
    }

    #[allow(dead_code)]
    struct MemSessionStore(Mutex<HashMap<String, Session>>);
    #[async_trait::async_trait]
    impl SessionStore for MemSessionStore {
        async fn create(&self, s: &Session) -> anyhow::Result<()> {
            self.0.lock().unwrap().insert(s.id.clone(), s.clone());
            Ok(())
        }
        async fn get(&self, id: &str) -> anyhow::Result<Option<Session>> {
            Ok(self.0.lock().unwrap().get(id).cloned())
        }
        async fn delete(&self, id: &str) -> anyhow::Result<bool> {
            Ok(self.0.lock().unwrap().remove(id).is_some())
        }
        async fn list_for_user(&self, _u: &str) -> anyhow::Result<Vec<Session>> {
            Ok(vec![])
        }
        async fn delete_for_user(&self, _u: &str) -> anyhow::Result<u64> {
            Ok(0)
        }
        async fn purge_expired(&self, _n: f64) -> anyhow::Result<u64> {
            Ok(0)
        }
        async fn list_all(
            &self,
            _limit: i64,
            _offset: i64,
            _user_filter: Option<&str>,
        ) -> anyhow::Result<Vec<Session>> {
            Ok(vec![])
        }
        async fn count_all(&self, _user_filter: Option<&str>) -> anyhow::Result<i64> {
            Ok(0)
        }
    }

    fn manager() -> PasskeyManager {
        let cfg = PasskeyConfig {
            rp_id: "localhost".to_string(),
            rp_name: "Assay Test".to_string(),
            origin: Url::parse("http://localhost:3000").unwrap(),
        };
        let users: Arc<dyn UserStore> =
            Arc::new(MemUserStore(Mutex::new(HashMap::new())));
        PasskeyManager::new(cfg, users).unwrap()
    }

    #[test]
    fn manager_construction_succeeds_for_localhost() {
        let m = manager();
        assert_eq!(m.config().rp_id, "localhost");
        assert_eq!(m.config().rp_name, "Assay Test");
    }

    #[tokio::test]
    async fn start_registration_emits_a_challenge_and_state() {
        let m = manager();
        let user_id = Uuid::new_v4();
        let (challenge, _state) = m
            .start_registration(user_id, "alice@example.com", "Alice", None)
            .await
            .expect("start_registration");
        // The challenge struct exposes `public_key` — sanity-check the
        // user shape made it through.
        assert_eq!(challenge.public_key.user.name, "alice@example.com");
        assert_eq!(challenge.public_key.user.display_name, "Alice");
    }

    #[tokio::test]
    async fn start_authentication_errors_when_no_passkeys() {
        let m = manager();
        let result = m.start_authentication("user_with_no_keys").await;
        assert!(matches!(result, Err(Error::Passkey(_))));
    }

    #[test]
    fn start_authentication_with_empty_list_errors() {
        let m = manager();
        let result = m.start_authentication_with(&[]);
        assert!(matches!(result, Err(Error::Passkey(_))));
    }
}
