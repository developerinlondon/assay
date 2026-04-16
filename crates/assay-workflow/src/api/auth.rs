use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use jsonwebtoken::jwk::JwkSet;
use jsonwebtoken::{DecodingKey, Validation};
use sha2::{Digest, Sha256};
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

use crate::api::AppState;
use crate::store::WorkflowStore;

const JWKS_CACHE_TTL: Duration = Duration::from_secs(300); // 5 minutes

// ── Auth Mode ───────────────────────────────────────────────

/// Auth configuration — determines which mode the engine runs in.
#[derive(Clone, Debug, Default)]
pub enum AuthMode {
    /// No authentication — all requests allowed (dev mode).
    #[default]
    NoAuth,
    /// API key authentication — Bearer token validated against hashed keys in DB.
    ApiKey,
    /// JWT/OIDC — validate Bearer JWT signature against JWKS from the issuer.
    Jwt {
        issuer: String,
        audience: Option<String>,
        jwks_cache: Arc<JwksCache>,
    },
}

impl AuthMode {
    /// Create a JWT auth mode that fetches JWKS from the issuer's OIDC discovery.
    pub fn jwt(issuer: String, audience: Option<String>) -> Self {
        Self::Jwt {
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
    async fn get_jwks(&self) -> anyhow::Result<JwkSet> {
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
    async fn refresh(&self) -> anyhow::Result<JwkSet> {
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
    async fn find_key(&self, kid: &str) -> anyhow::Result<DecodingKey> {
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
    async fn find_any_key(&self, alg: jsonwebtoken::Algorithm) -> anyhow::Result<DecodingKey> {
        let jwks = self.get_jwks().await?;

        for key in &jwks.keys {
            if let Ok(dk) = DecodingKey::from_jwk(key) {
                // Use the first key that decodes successfully — if the JWK
                // specifies an algorithm, the jsonwebtoken Validation will
                // catch mismatches during decode.
                let _ = alg; // algorithm check happens at decode time
                return Ok(dk);
            }
        }

        anyhow::bail!("No suitable key found in JWKS for algorithm {alg:?}")
    }
}

fn find_key_in_set(jwks: &JwkSet, kid: &str) -> Option<DecodingKey> {
    jwks.keys
        .iter()
        .find(|k| k.common.key_id.as_deref() == Some(kid))
        .and_then(|k| DecodingKey::from_jwk(k).ok())
}

// ── Middleware ───────────────────────────────────────────────

/// Axum middleware that enforces authentication based on the configured mode.
pub async fn auth_middleware<S: WorkflowStore>(
    State(state): State<Arc<AppState<S>>>,
    request: Request,
    next: Next,
) -> Response {
    match &state.auth_mode {
        AuthMode::NoAuth => next.run(request).await,
        AuthMode::ApiKey => validate_api_key(state, request, next).await,
        AuthMode::Jwt {
            issuer,
            audience,
            jwks_cache,
        } => validate_jwt(issuer, audience.as_deref(), jwks_cache, request, next).await,
    }
}

async fn validate_api_key<S: WorkflowStore>(
    state: Arc<AppState<S>>,
    request: Request,
    next: Next,
) -> Response {
    let token = match extract_bearer(&request) {
        Some(t) => t,
        None => return auth_error("Missing Authorization: Bearer <api-key>"),
    };

    let hash = hash_api_key(token);
    match state.engine.store().validate_api_key(&hash).await {
        Ok(true) => next.run(request).await,
        Ok(false) => {
            warn!(
                "Invalid API key (prefix: {}...)",
                &token[..8.min(token.len())]
            );
            auth_error("Invalid API key")
        }
        Err(e) => {
            warn!("API key validation error: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "auth check failed"})),
            )
                .into_response()
        }
    }
}

async fn validate_jwt(
    issuer: &str,
    audience: Option<&str>,
    jwks_cache: &JwksCache,
    request: Request,
    next: Next,
) -> Response {
    let token = match extract_bearer(&request) {
        Some(t) => t,
        None => return auth_error("Missing Authorization: Bearer <jwt>"),
    };

    // Decode header to get algorithm and kid
    let header = match jsonwebtoken::decode_header(token) {
        Ok(h) => h,
        Err(e) => {
            warn!("Invalid JWT header: {e}");
            return auth_error("Invalid JWT");
        }
    };

    // Find the decoding key from JWKS
    let decoding_key = match &header.kid {
        Some(kid) => match jwks_cache.find_key(kid).await {
            Ok(key) => key,
            Err(e) => {
                warn!("JWKS key lookup failed: {e}");
                return auth_error("JWT validation failed: key not found");
            }
        },
        None => match jwks_cache.find_any_key(header.alg).await {
            Ok(key) => key,
            Err(e) => {
                warn!("JWKS key lookup failed (no kid): {e}");
                return auth_error("JWT validation failed: no suitable key");
            }
        },
    };

    // Build validation rules
    let mut validation = Validation::new(header.alg);
    validation.set_issuer(&[issuer]);
    if let Some(aud) = audience {
        validation.set_audience(&[aud]);
    } else {
        validation.validate_aud = false;
    }

    // Validate signature + claims
    match jsonwebtoken::decode::<serde_json::Value>(token, &decoding_key, &validation) {
        Ok(_) => next.run(request).await,
        Err(e) => {
            warn!("JWT validation failed: {e}");
            auth_error(&format!("JWT validation failed: {e}"))
        }
    }
}

fn extract_bearer(request: &Request) -> Option<&str> {
    request
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
}

fn auth_error(msg: &str) -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(serde_json::json!({"error": msg})),
    )
        .into_response()
}

// ── API Key Helpers ─────────────────────────────────────────

/// Hash an API key with SHA-256 for storage/lookup.
pub fn hash_api_key(key: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(key.as_bytes());
    data_encoding::HEXLOWER.encode(&hasher.finalize())
}

/// Generate a new random API key (32 bytes, hex-encoded).
pub fn generate_api_key() -> String {
    use rand::Rng;
    let bytes: [u8; 32] = rand::rng().random();
    format!("assay_{}", data_encoding::HEXLOWER.encode(&bytes))
}

/// Extract the prefix (first 8 chars after "assay_") for display.
pub fn key_prefix(key: &str) -> String {
    let stripped = key.strip_prefix("assay_").unwrap_or(key);
    format!("assay_{}...", &stripped[..8.min(stripped.len())])
}
