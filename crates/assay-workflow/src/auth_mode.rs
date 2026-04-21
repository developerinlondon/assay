//! Auth configuration types — `AuthMode`, `JwtConfig`, `JwksCache`.
//! Lives at crate root (not under `api/`) so `ctx.rs` can import
//! `AuthMode` without creating a dependency cycle with `api/auth.rs`.
//!
//! `api/auth.rs` re-exports these types so the public path
//! `assay_workflow::api::auth::AuthMode` continues to work.

use std::sync::Arc;
use std::time::{Duration, Instant};

use jsonwebtoken::jwk::JwkSet;
use tokio::sync::RwLock;
use tracing::{debug, info};

const JWKS_CACHE_TTL: Duration = Duration::from_secs(300); // 5 minutes

// ── Auth Mode ───────────────────────────────────────────────

/// Auth configuration for the engine's HTTP API.
///
/// Both authentication methods (JWT and API key) can be enabled at the same time.
/// When both are enabled, the middleware dispatches on token shape — tokens that
/// parse as a JWS header are validated as JWTs, everything else is validated as
/// an API key. This lets the same server accept long-lived machine API keys
/// alongside short-lived OIDC-issued user tokens without the caller picking a
/// mode up front.
#[derive(Clone, Debug, Default)]
pub struct AuthMode {
    /// API-key authentication enabled. When true, Bearer tokens that are not
    /// JWT-shaped are validated against the `api_keys` table.
    pub api_key: bool,
    /// JWT authentication enabled. When set, Bearer tokens that parse as a
    /// JWS header are validated against the issuer's JWKS.
    pub jwt: Option<JwtConfig>,
}

/// JWT validation configuration.
#[derive(Clone, Debug)]
pub struct JwtConfig {
    pub issuer: String,
    pub audience: Option<String>,
    pub jwks_cache: Arc<JwksCache>,
}

impl AuthMode {
    /// Open access — no authentication. All requests allowed.
    pub fn no_auth() -> Self {
        Self::default()
    }

    /// JWT/OIDC only. Tokens are validated against the issuer's JWKS.
    pub fn jwt(issuer: String, audience: Option<String>) -> Self {
        Self {
            api_key: false,
            jwt: Some(JwtConfig::new(issuer, audience)),
        }
    }

    /// API key only. Bearer tokens are hashed and looked up in the store.
    pub fn api_key() -> Self {
        Self {
            api_key: true,
            jwt: None,
        }
    }

    /// Both JWT and API key. Tokens that parse as JWTs take the JWT path;
    /// everything else takes the API-key path.
    pub fn combined(issuer: String, audience: Option<String>) -> Self {
        Self {
            api_key: true,
            jwt: Some(JwtConfig::new(issuer, audience)),
        }
    }

    /// True if any authentication method is enabled.
    pub fn is_enabled(&self) -> bool {
        self.api_key || self.jwt.is_some()
    }

    /// Human-readable summary for startup logging.
    pub fn describe(&self) -> String {
        match (self.jwt.as_ref(), self.api_key) {
            (None, false) => "no-auth (open access)".to_string(),
            (None, true) => "api-key".to_string(),
            (Some(c), false) => format!("jwt (issuer: {})", c.issuer),
            (Some(c), true) => format!("jwt (issuer: {}) + api-key", c.issuer),
        }
    }
}

impl JwtConfig {
    /// Build a JwtConfig with a fresh JWKS cache pointed at `issuer`'s OIDC discovery endpoint.
    pub fn new(issuer: String, audience: Option<String>) -> Self {
        Self {
            jwks_cache: Arc::new(JwksCache::new(issuer.clone())),
            issuer,
            audience,
        }
    }
}

// ── JWKS Cache ──────────────────────────────────────────────

/// Caches JWKS keys fetched from the OIDC provider.
/// Keys are refreshed after `JWKS_CACHE_TTL` or on cache miss for a specific `kid`.
pub struct JwksCache {
    issuer: String,
    cache: RwLock<Option<CachedJwks>>,
    http: reqwest::Client,
}

struct CachedJwks {
    jwks: JwkSet,
    fetched_at: Instant,
}

impl std::fmt::Debug for JwksCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JwksCache")
            .field("issuer", &self.issuer)
            .finish()
    }
}

impl JwksCache {
    pub fn new(issuer: String) -> Self {
        Self {
            issuer,
            cache: RwLock::new(None),
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .expect("building JWKS HTTP client"),
        }
    }

    /// For testing: create a cache pre-loaded with keys (no HTTP fetching needed).
    pub fn with_jwks(issuer: String, jwks: JwkSet) -> Self {
        Self {
            issuer,
            cache: RwLock::new(Some(CachedJwks {
                jwks,
                fetched_at: Instant::now(),
            })),
            http: reqwest::Client::new(),
        }
    }

    /// Get the JWKS, fetching from the provider if the cache is stale or empty.
    pub async fn get_jwks(&self) -> anyhow::Result<JwkSet> {
        // Check cache
        {
            let cache = self.cache.read().await;
            if let Some(ref cached) = *cache
                && cached.fetched_at.elapsed() < JWKS_CACHE_TTL
            {
                return Ok(cached.jwks.clone());
            }
        }

        // Cache miss or stale — fetch fresh JWKS
        self.refresh().await
    }

    /// Force-refresh the JWKS (e.g., when a kid is not found in the current set).
    pub async fn refresh(&self) -> anyhow::Result<JwkSet> {
        let jwks_uri = self.discover_jwks_uri().await?;
        debug!("Fetching JWKS from {jwks_uri}");

        let jwks: JwkSet = self.http.get(&jwks_uri).send().await?.json().await?;
        info!(
            "Fetched {} keys from JWKS endpoint",
            jwks.keys.len()
        );

        let mut cache = self.cache.write().await;
        *cache = Some(CachedJwks {
            jwks: jwks.clone(),
            fetched_at: Instant::now(),
        });

        Ok(jwks)
    }

    /// Discover the JWKS URI from the OIDC discovery endpoint.
    async fn discover_jwks_uri(&self) -> anyhow::Result<String> {
        let discovery_url = format!(
            "{}/.well-known/openid-configuration",
            self.issuer.trim_end_matches('/')
        );

        let resp: serde_json::Value = self
            .http
            .get(&discovery_url)
            .send()
            .await?
            .json()
            .await?;

        resp.get("jwks_uri")
            .and_then(|v| v.as_str())
            .map(String::from)
            .ok_or_else(|| anyhow::anyhow!("OIDC discovery response missing jwks_uri"))
    }

    /// Find a decoding key by `kid` (key ID) from the cached JWKS.
    /// If the kid isn't found, refreshes the cache once and retries.
    pub async fn find_key(&self, kid: &str) -> anyhow::Result<jsonwebtoken::DecodingKey> {
        let jwks = self.get_jwks().await?;

        // Try to find by kid
        if let Some(key) = find_key_in_set(&jwks, kid) {
            return Ok(key);
        }

        // kid not found — refresh and retry (key rotation)
        debug!("kid '{kid}' not in JWKS cache, refreshing");
        let jwks = self.refresh().await?;

        find_key_in_set(&jwks, kid)
            .ok_or_else(|| anyhow::anyhow!("No key with kid '{kid}' in JWKS"))
    }

    /// Find a decoding key when no kid is provided (use the first matching key).
    pub async fn find_any_key(
        &self,
        alg: jsonwebtoken::Algorithm,
    ) -> anyhow::Result<jsonwebtoken::DecodingKey> {
        let jwks = self.get_jwks().await?;

        for key in &jwks.keys {
            if let Ok(dk) = jsonwebtoken::DecodingKey::from_jwk(key) {
                let _ = alg;
                return Ok(dk);
            }
        }

        anyhow::bail!("No suitable key found in JWKS for algorithm {alg:?}")
    }
}

fn find_key_in_set(
    jwks: &JwkSet,
    kid: &str,
) -> Option<jsonwebtoken::DecodingKey> {
    jwks.keys
        .iter()
        .find(|k| k.common.key_id.as_deref() == Some(kid))
        .and_then(|k| jsonwebtoken::DecodingKey::from_jwk(k).ok())
}
