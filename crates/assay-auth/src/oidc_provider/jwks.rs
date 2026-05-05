//! `/.well-known/jwks.json` — JWK Set endpoint.
//!
//! Reads the `public_jwk` column off every row in `auth.jwks_keys`
//! (active + history) and emits the standard JWK Set envelope per
//! RFC 7517. Consumers fetch this once and cache; rotation just appends
//! a new row, so consumers that re-fetch get the new key without losing
//! the old.

use axum::{Json, extract::State};
use serde_json::{Value, json};

use crate::ctx::AuthCtx;

/// Build a JWK Set from a list of `public_jwk` JSON values. Pure
/// function — used by both the runtime handler and tests.
pub fn build_jwks(public_jwks: Vec<Value>) -> Value {
    json!({ "keys": public_jwks })
}

/// `GET /.well-known/jwks.json`. Returns 200 + `{"keys": []}` even when
/// no keys are loaded — caller-friendly default that matches what
/// well-known consumers tolerate.
pub async fn jwks_handler(State(ctx): State<AuthCtx>) -> Json<Value> {
    let keys = match &ctx.oidc_provider {
        Some(p) => load_public_jwks(p).await.unwrap_or_default(),
        None => Vec::new(),
    };
    Json(build_jwks(keys))
}

/// Fetch every `public_jwk` from `auth.jwks_keys` (active first by
/// `rotated_at IS NULL`, then history ordered by `created_at`).
///
/// Returns the parsed JSON values. Failures (DB unavailable, malformed
/// JSON in a row) are surfaced as a single error so the handler can
/// degrade gracefully — an empty key list is preferable to a 500 in
/// most operational scenarios.
pub async fn load_public_jwks(provider: &super::OidcProviderConfig) -> anyhow::Result<Vec<Value>> {
    match &provider.jwks_source {
        #[cfg(feature = "backend-postgres")]
        super::JwksSource::Postgres(pool) => load_pg(pool).await,
        #[cfg(feature = "backend-sqlite")]
        super::JwksSource::Sqlite(pool) => load_sqlite(pool).await,
        super::JwksSource::Memory(values) => Ok(values.clone()),
    }
}

#[cfg(feature = "backend-postgres")]
async fn load_pg(pool: &sqlx::PgPool) -> anyhow::Result<Vec<Value>> {
    use sqlx::Row;
    let rows = sqlx::query(
        "SELECT public_jwk FROM auth.jwks_keys
         ORDER BY (rotated_at IS NULL) DESC, created_at",
    )
    .fetch_all(pool)
    .await?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let v: Value = row.get("public_jwk");
        out.push(v);
    }
    Ok(out)
}

#[cfg(feature = "backend-sqlite")]
async fn load_sqlite(pool: &sqlx::SqlitePool) -> anyhow::Result<Vec<Value>> {
    use sqlx::Row;
    let rows = sqlx::query(
        "SELECT public_jwk FROM auth.jwks_keys
         ORDER BY (rotated_at IS NULL) DESC, created_at",
    )
    .fetch_all(pool)
    .await?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let s: String = row.get("public_jwk");
        let v: Value = serde_json::from_str(&s).unwrap_or_else(|_| json!({}));
        out.push(v);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_jwks_returns_keys_array() {
        let k1 = json!({"kty": "OKP", "crv": "Ed25519", "kid": "kid_a"});
        let k2 = json!({"kty": "OKP", "crv": "Ed25519", "kid": "kid_b"});
        let set = build_jwks(vec![k1.clone(), k2.clone()]);
        assert_eq!(set["keys"], json!([k1, k2]));
    }

    #[test]
    fn build_jwks_empty_is_well_formed() {
        let set = build_jwks(Vec::new());
        assert_eq!(set, json!({"keys": []}));
    }
}
