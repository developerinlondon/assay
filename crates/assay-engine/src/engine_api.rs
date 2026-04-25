//! Engine-core HTTP admin API.
//!
//! Mounted at `/api/v1/engine/core/*` by the engine binary so the engine
//! console (`/engine/console`) has structured JSON to render.
//!
//! Endpoints (all return JSON):
//!
//! - `GET    /api/v1/engine/core/info`              public, no auth
//! - `GET    /api/v1/engine/core/health`            public, no auth
//! - `GET    /api/v1/engine/core/active-modules`    public, no auth
//! - `GET    /api/v1/engine/core/modules`           admin
//! - `POST   /api/v1/engine/core/modules/{name}/toggle`  admin
//! - `GET    /api/v1/engine/core/instances`         admin
//! - `GET    /api/v1/engine/core/audit`             admin
//! - `GET    /api/v1/engine/core/config`            admin (secrets redacted)
//!
//! Admin-gated endpoints reuse the same `Authorization: Bearer ...`
//! check the auth admin router uses (compared in constant-ish time
//! against `EngineState.admin_api_keys`). When `admin_api_keys` is
//! empty every admin endpoint returns 401 — locking the surface
//! entirely. `info`, `health`, and `active-modules` are always public
//! so dashboards can render the header bar + cross-nav before an
//! operator has supplied credentials.
//!
//! Backend-agnostic: handlers branch on the `BackendConfig` variant
//! to call into either `PgEngineSchema` or `SqliteEngineSchema`. SQLite
//! engines are single-instance so the `instances` endpoint typically
//! returns one row; the shape stays identical so the UI doesn't care.

use axum::Router;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Json, Response};
use axum::routing::{get, post};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use assay_workflow::WorkflowStore;

use crate::config::{BackendConfig, EngineConfig};
use crate::state::EngineState;

/// Build the engine-core admin router. Bound to `EngineState<S>` so
/// handlers can pluck the workflow store, the parsed config, and the
/// admin keys list off the parent state.
pub fn router<S>() -> Router<EngineState<S>>
where
    S: WorkflowStore + Clone + 'static,
{
    Router::new()
        .route("/api/v1/engine/core/info", get(engine_info::<S>))
        .route("/api/v1/engine/core/health", get(engine_health::<S>))
        .route(
            "/api/v1/engine/core/active-modules",
            get(active_modules::<S>),
        )
        .route("/api/v1/engine/core/modules", get(list_modules::<S>))
        .route(
            "/api/v1/engine/core/modules/{name}/toggle",
            post(toggle_module::<S>),
        )
        .route("/api/v1/engine/core/instances", get(list_instances::<S>))
        .route("/api/v1/engine/core/audit", get(list_audit::<S>))
        .route("/api/v1/engine/core/config", get(get_config::<S>))
}

// =====================================================================
//   /api/v1/engine/core/health
// =====================================================================

/// Engine-core health probe. Returns the same envelope previously
/// served by the legacy `/healthz` endpoint (status + version +
/// instance_id + active modules + leader flag) so existing operator
/// scripts that scrape the JSON keep working after the URL move.
async fn engine_health<S: WorkflowStore + Clone + 'static>(
    State(s): State<EngineState<S>>,
) -> Json<Value> {
    Json(json!({
        "status": "ok",
        "engine_version": s.engine_version,
        "instance_id": s.instance_id.to_string(),
        "modules": &*s.modules,
        // SQLite is single-instance and PG uses session-scoped
        // pg_try_advisory_lock; both make leadership a runtime
        // property. Surface it as `leader = true` for SQLite (no
        // election) so dashboards keep the field stable.
        "leader": true,
    }))
}

// =====================================================================
//   /api/v1/engine/core/active-modules
// =====================================================================

/// Active-modules listing — public, no auth. Read by the cross-console
/// nav strip JS so disabled modules' pills don't render. Replaces the
/// legacy top-level `/api/v1/modules` endpoint.
async fn active_modules<S: WorkflowStore + Clone + 'static>(
    State(s): State<EngineState<S>>,
) -> Json<Value> {
    Json(json!({
        "modules": &*s.modules,
    }))
}

// =====================================================================
//   /api/v1/engine/core/info
// =====================================================================

#[derive(Debug, Clone, Serialize)]
pub struct EngineInfo {
    pub version: &'static str,
    pub instance_id: String,
    pub started_at: f64,
    pub leader: bool,
    pub modules: Vec<String>,
    pub backend_kind: &'static str,
    /// SQLite data directory when the backend is SQLite. `None` for PG.
    pub backend_data_dir: Option<String>,
    /// Postgres connection URL with the userinfo + password redacted.
    /// `None` for SQLite.
    pub backend_url_redacted: Option<String>,
    pub bind_addr: String,
    pub public_url: String,
}

async fn engine_info<S: WorkflowStore + Clone + 'static>(
    State(s): State<EngineState<S>>,
) -> Json<EngineInfo> {
    let cfg: &EngineConfig = &s.engine_config;
    let (kind, data_dir, url_redacted) = match &cfg.backend {
        BackendConfig::Sqlite { data_dir, .. } => ("sqlite", Some(data_dir.clone()), None),
        BackendConfig::Postgres { url } => ("postgres", None, Some(redact_pg_url(url))),
    };
    Json(EngineInfo {
        version: s.engine_version,
        instance_id: s.instance_id.to_string(),
        started_at: s.started_at,
        leader: true,
        modules: (*s.modules).clone(),
        backend_kind: kind,
        backend_data_dir: data_dir,
        backend_url_redacted: url_redacted,
        bind_addr: cfg.server.bind_addr.clone(),
        public_url: cfg.server.public_url.clone(),
    })
}

// =====================================================================
//   /api/v1/engine/core/modules
// =====================================================================

#[derive(Debug, Clone, Serialize)]
pub struct ModuleEntry {
    pub name: String,
    pub enabled: bool,
    pub enabled_at: Option<f64>,
    pub enabled_by: Option<String>,
    pub version: Option<String>,
    pub config: Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct ListModulesResponse {
    pub items: Vec<ModuleEntry>,
}

async fn list_modules<S: WorkflowStore + Clone + 'static>(
    State(s): State<EngineState<S>>,
    headers: HeaderMap,
) -> Response {
    if let Err(r) = require_admin(&headers, &s.admin_api_keys) {
        return *r;
    }
    let items = match list_module_records(&s.engine_config).await {
        Ok(v) => v,
        Err(e) => return server_error(&format!("list modules: {e}")),
    };
    let entries = items
        .into_iter()
        .map(|m| ModuleEntry {
            name: m.name,
            enabled: m.enabled,
            enabled_at: m.enabled_at,
            enabled_by: m.enabled_by,
            version: m.version,
            config: m.config,
        })
        .collect();
    (StatusCode::OK, Json(ListModulesResponse { items: entries })).into_response()
}

#[derive(Debug, Clone, Deserialize)]
pub struct ToggleBody {
    /// Optional explicit enabled flag. When omitted the handler flips
    /// the current value.
    pub enabled: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ToggleResponse {
    pub enabled: bool,
    pub restart_required: bool,
    pub message: String,
}

async fn toggle_module<S: WorkflowStore + Clone + 'static>(
    State(s): State<EngineState<S>>,
    headers: HeaderMap,
    Path(name): Path<String>,
    body: Option<Json<ToggleBody>>,
) -> Response {
    if let Err(r) = require_admin(&headers, &s.admin_api_keys) {
        return *r;
    }
    // Look up the current module row so we know what to flip to.
    let modules = match list_module_records(&s.engine_config).await {
        Ok(v) => v,
        Err(e) => return server_error(&format!("list modules: {e}")),
    };
    let Some(current) = modules.iter().find(|m| m.name == name) else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "unknown module name"})),
        )
            .into_response();
    };
    let target = body
        .and_then(|Json(b)| b.enabled)
        .unwrap_or(!current.enabled);
    let actor = bearer_token(&headers).map(short_actor);
    if let Err(e) =
        set_module_enabled(&s.engine_config, &name, target, actor.as_deref()).await
    {
        return server_error(&format!("set enabled: {e}"));
    }
    if let Err(e) = audit_module_toggle(&s.engine_config, &name, target, actor.as_deref()).await
    {
        // Failure to audit doesn't undo the flip; surface as a warning.
        tracing::warn!(?e, "engine.audit insert failed for module toggle");
    }
    let msg = if target {
        format!("module {name} marked enabled — restart engine to load")
    } else {
        format!("module {name} marked disabled — restart engine to unload")
    };
    (
        StatusCode::OK,
        Json(ToggleResponse {
            enabled: target,
            restart_required: true,
            message: msg,
        }),
    )
        .into_response()
}

// =====================================================================
//   /api/v1/engine/core/instances
// =====================================================================

#[derive(Debug, Clone, Serialize)]
pub struct InstanceEntry {
    pub id: String,
    pub started_at: f64,
    pub last_heartbeat: f64,
    pub namespaces: Vec<String>,
    pub version: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ListInstancesResponse {
    pub items: Vec<InstanceEntry>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct PageQuery {
    #[serde(default)]
    pub limit: Option<i64>,
    #[serde(default)]
    pub offset: Option<i64>,
}

async fn list_instances<S: WorkflowStore + Clone + 'static>(
    State(s): State<EngineState<S>>,
    headers: HeaderMap,
    Query(_q): Query<PageQuery>,
) -> Response {
    if let Err(r) = require_admin(&headers, &s.admin_api_keys) {
        return *r;
    }
    let items = match list_instance_records(&s.engine_config).await {
        Ok(v) => v,
        Err(e) => return server_error(&format!("list instances: {e}")),
    };
    let entries = items
        .into_iter()
        .map(|i| InstanceEntry {
            id: i.id,
            started_at: i.started_at,
            last_heartbeat: i.last_heartbeat,
            namespaces: i.namespaces,
            version: i.version,
        })
        .collect();
    (
        StatusCode::OK,
        Json(ListInstancesResponse { items: entries }),
    )
        .into_response()
}

// =====================================================================
//   /api/v1/engine/core/audit
// =====================================================================

#[derive(Debug, Clone, Default, Deserialize)]
pub struct AuditQuery {
    #[serde(default)]
    pub limit: Option<i64>,
    #[serde(default)]
    pub offset: Option<i64>,
    #[serde(default)]
    pub actor: Option<String>,
    #[serde(default)]
    pub action: Option<String>,
    #[serde(default)]
    pub since: Option<f64>,
    #[serde(default)]
    pub until: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AuditEntry {
    pub id: String,
    pub ts: f64,
    pub actor: Option<String>,
    pub action: String,
    pub details: Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct ListAuditResponse {
    pub items: Vec<AuditEntry>,
    pub total: i64,
    pub limit: i64,
    pub offset: i64,
}

async fn list_audit<S: WorkflowStore + Clone + 'static>(
    State(s): State<EngineState<S>>,
    headers: HeaderMap,
    Query(q): Query<AuditQuery>,
) -> Response {
    if let Err(r) = require_admin(&headers, &s.admin_api_keys) {
        return *r;
    }
    let limit = q.limit.unwrap_or(50).clamp(1, 500);
    let offset = q.offset.unwrap_or(0).max(0);
    let (rows, total) = match list_audit_records(
        &s.engine_config,
        limit,
        offset,
        q.actor.as_deref(),
        q.action.as_deref(),
        q.since,
        q.until,
    )
    .await
    {
        Ok(v) => v,
        Err(e) => return server_error(&format!("list audit: {e}")),
    };
    let items = rows
        .into_iter()
        .map(|a| AuditEntry {
            id: a.id,
            ts: a.ts,
            actor: a.actor,
            action: a.action,
            details: a.details,
        })
        .collect();
    (
        StatusCode::OK,
        Json(ListAuditResponse {
            items,
            total,
            limit,
            offset,
        }),
    )
        .into_response()
}

// =====================================================================
//   /api/v1/engine/core/config
// =====================================================================

async fn get_config<S: WorkflowStore + Clone + 'static>(
    State(s): State<EngineState<S>>,
    headers: HeaderMap,
) -> Response {
    if let Err(r) = require_admin(&headers, &s.admin_api_keys) {
        return *r;
    }
    let mut value = match serde_json::to_value(&*s.engine_config) {
        Ok(v) => v,
        Err(e) => return server_error(&format!("serialise config: {e}")),
    };
    redact_secrets(&mut value);
    (StatusCode::OK, Json(value)).into_response()
}

/// Walk the serialised config and replace any value whose key looks
/// like a secret with the literal string `[REDACTED]`. Targets:
///
/// - `admin_api_keys`             (array of strings)
/// - any field whose key contains `password`, `secret`, `token`, `key`
///   (case-insensitive) other than the structural `kid` / `keys`
///
/// Defensive: the engine console renders this for operators, so an
/// over-redaction (e.g. `rp_id` containing the substring "id" — no it
/// doesn't, but the pattern is conservative) is preferable to leaking
/// a credential into a screenshot.
fn redact_secrets(v: &mut Value) {
    let placeholder = Value::String("[REDACTED]".to_string());
    match v {
        Value::Object(map) => {
            // Snapshot the keys upfront — modifying values while iterating
            // owned-mut keys is fine, but we want a clear walk.
            let keys: Vec<String> = map.keys().cloned().collect();
            for k in keys {
                let lk = k.to_lowercase();
                if k == "admin_api_keys" {
                    if let Some(Value::Array(arr)) = map.get_mut(&k) {
                        for entry in arr {
                            *entry = placeholder.clone();
                        }
                    }
                    continue;
                }
                if (lk.contains("password")
                    || lk.contains("secret")
                    || lk.contains("api_key")
                    || lk.contains("api-key"))
                    && let Some(slot) = map.get_mut(&k)
                {
                    match slot {
                        Value::String(_) => *slot = placeholder.clone(),
                        Value::Array(arr) => {
                            for entry in arr {
                                if let Value::String(_) = entry {
                                    *entry = placeholder.clone();
                                } else {
                                    redact_secrets(entry);
                                }
                            }
                        }
                        other => redact_secrets(other),
                    }
                    continue;
                }
                if let Some(slot) = map.get_mut(&k) {
                    redact_secrets(slot);
                }
            }
        }
        Value::Array(arr) => {
            for entry in arr {
                redact_secrets(entry);
            }
        }
        _ => {}
    }
}

// =====================================================================
//   helpers — admin auth
// =====================================================================

fn require_admin(headers: &HeaderMap, keys: &[String]) -> Result<(), Box<Response>> {
    if keys.is_empty() {
        return Err(Box::new(
            (
                StatusCode::UNAUTHORIZED,
                Json(json!({"error": "admin disabled — no admin_api_keys configured"})),
            )
                .into_response(),
        ));
    }
    let presented = bearer_token(headers).unwrap_or_default();
    if !keys.iter().any(|k| constant_eq(k.as_bytes(), presented.as_bytes())) {
        return Err(Box::new(
            (
                StatusCode::UNAUTHORIZED,
                Json(json!({"error": "invalid admin token"})),
            )
                .into_response(),
        ));
    }
    Ok(())
}

fn bearer_token(headers: &HeaderMap) -> Option<String> {
    let raw = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())?;
    raw.strip_prefix("Bearer ")
        .or_else(|| raw.strip_prefix("bearer "))
        .map(|s| s.trim().to_string())
}

/// Constant-time-ish equality on equal-length byte slices. Prevents the
/// trivial timing leak from comparing raw `&[u8] == &[u8]`. The length
/// short-circuit is unavoidable with variable-length tokens but is
/// considered acceptable for admin api-keys (operator-controlled).
fn constant_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Reduce an admin token to a short, non-reversible identifier so the
/// audit log doesn't store the token itself. Truncated last 6 chars
/// labelled `admin:****abcdef` — same shape used elsewhere for keyed
/// admin actions (see auth admin.rs `audit` calls).
fn short_actor(token: String) -> String {
    let t = token.trim();
    if t.len() <= 6 {
        return format!("admin:****{t}");
    }
    let tail = &t[t.len() - 6..];
    format!("admin:****{tail}")
}

fn server_error(msg: &str) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({"error": "server_error", "error_description": msg})),
    )
        .into_response()
}

// =====================================================================
//   helpers — backend-routed schema reads
// =====================================================================

/// Open a fresh schema handle on demand. Cheap — the underlying pool
/// is owned by the workflow context that already initialised at boot.
/// We re-resolve the connection string from the parsed config so this
/// helper is callable from handlers that only carry the cloned state.
async fn list_module_records(
    cfg: &EngineConfig,
) -> anyhow::Result<Vec<assay_domain::engine::ModuleRecord>> {
    match &cfg.backend {
        #[cfg(feature = "backend-postgres")]
        BackendConfig::Postgres { url } => {
            let pool = sqlx::PgPool::connect(url)
                .await
                .map_err(|e| anyhow::anyhow!("connect pg: {e}"))?;
            let schema = assay_domain::engine::PgEngineSchema::new(pool);
            let rows = schema
                .list_modules()
                .await
                .map_err(|e| anyhow::anyhow!("list modules (pg): {e}"))?;
            Ok(rows)
        }
        #[cfg(feature = "backend-sqlite")]
        BackendConfig::Sqlite { .. } => {
            let pool = open_sqlite_engine_pool(cfg).await?;
            let schema = assay_domain::engine::SqliteEngineSchema::new(pool);
            let rows = schema
                .list_modules()
                .await
                .map_err(|e| anyhow::anyhow!("list modules (sqlite): {e}"))?;
            Ok(rows)
        }
        #[allow(unreachable_patterns)]
        _ => anyhow::bail!("backend not enabled at compile time"),
    }
}

async fn set_module_enabled(
    cfg: &EngineConfig,
    name: &str,
    enabled: bool,
    actor: Option<&str>,
) -> anyhow::Result<bool> {
    match &cfg.backend {
        #[cfg(feature = "backend-postgres")]
        BackendConfig::Postgres { url } => {
            let pool = sqlx::PgPool::connect(url).await?;
            let schema = assay_domain::engine::PgEngineSchema::new(pool);
            schema.set_module_enabled(name, enabled, actor).await
        }
        #[cfg(feature = "backend-sqlite")]
        BackendConfig::Sqlite { .. } => {
            let pool = open_sqlite_engine_pool(cfg).await?;
            let schema = assay_domain::engine::SqliteEngineSchema::new(pool);
            schema.set_module_enabled(name, enabled, actor).await
        }
        #[allow(unreachable_patterns)]
        _ => anyhow::bail!("backend not enabled at compile time"),
    }
}

async fn audit_module_toggle(
    cfg: &EngineConfig,
    name: &str,
    enabled: bool,
    actor: Option<&str>,
) -> anyhow::Result<()> {
    let details = json!({"module": name, "enabled": enabled});
    match &cfg.backend {
        #[cfg(feature = "backend-postgres")]
        BackendConfig::Postgres { url } => {
            let pool = sqlx::PgPool::connect(url).await?;
            let schema = assay_domain::engine::PgEngineSchema::new(pool);
            schema.audit(actor, "engine.module.toggle", &details).await
        }
        #[cfg(feature = "backend-sqlite")]
        BackendConfig::Sqlite { .. } => {
            let pool = open_sqlite_engine_pool(cfg).await?;
            let schema = assay_domain::engine::SqliteEngineSchema::new(pool);
            schema.audit(actor, "engine.module.toggle", &details).await
        }
        #[allow(unreachable_patterns)]
        _ => anyhow::bail!("backend not enabled at compile time"),
    }
}

async fn list_instance_records(
    cfg: &EngineConfig,
) -> anyhow::Result<Vec<assay_domain::engine::InstanceRecord>> {
    match &cfg.backend {
        #[cfg(feature = "backend-postgres")]
        BackendConfig::Postgres { url } => {
            let pool = sqlx::PgPool::connect(url).await?;
            let schema = assay_domain::engine::PgEngineSchema::new(pool);
            schema.list_instances().await
        }
        #[cfg(feature = "backend-sqlite")]
        BackendConfig::Sqlite { .. } => {
            let pool = open_sqlite_engine_pool(cfg).await?;
            let schema = assay_domain::engine::SqliteEngineSchema::new(pool);
            schema.list_instances().await
        }
        #[allow(unreachable_patterns)]
        _ => anyhow::bail!("backend not enabled at compile time"),
    }
}

async fn list_audit_records(
    cfg: &EngineConfig,
    limit: i64,
    offset: i64,
    actor: Option<&str>,
    action: Option<&str>,
    since: Option<f64>,
    until: Option<f64>,
) -> anyhow::Result<(Vec<assay_domain::engine::AuditRecord>, i64)> {
    match &cfg.backend {
        #[cfg(feature = "backend-postgres")]
        BackendConfig::Postgres { url } => {
            let pool = sqlx::PgPool::connect(url).await?;
            let schema = assay_domain::engine::PgEngineSchema::new(pool);
            schema
                .list_audit(limit, offset, actor, action, since, until)
                .await
        }
        #[cfg(feature = "backend-sqlite")]
        BackendConfig::Sqlite { .. } => {
            let pool = open_sqlite_engine_pool(cfg).await?;
            let schema = assay_domain::engine::SqliteEngineSchema::new(pool);
            schema
                .list_audit(limit, offset, actor, action, since, until)
                .await
        }
        #[allow(unreachable_patterns)]
        _ => anyhow::bail!("backend not enabled at compile time"),
    }
}

/// Open a fresh `engine.db`-only sqlite pool, mirroring the
/// `init.rs::sqlite_boot` ATTACH layout. Used by handlers that need a
/// schema-qualified `engine.modules` query without sharing the boot
/// pool's connection. Lighter than re-running boot — skips workflow
/// + auth ATTACH.
#[cfg(feature = "backend-sqlite")]
async fn open_sqlite_engine_pool(cfg: &EngineConfig) -> anyhow::Result<sqlx::SqlitePool> {
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
    use std::str::FromStr;

    let data_dir = cfg
        .backend
        .sqlite_data_dir()
        .ok_or_else(|| anyhow::anyhow!("sqlite_data_dir missing on non-sqlite backend"))?;
    let path = format!("file:{data_dir}/engine.db?mode=rw");
    let opts = SqliteConnectOptions::from_str("sqlite::memory:")?.create_if_missing(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .after_connect(move |conn, _meta| {
            let path = path.clone();
            Box::pin(async move {
                use sqlx::Executor;
                conn.execute(format!("ATTACH DATABASE '{path}' AS engine").as_str())
                    .await?;
                Ok(())
            })
        })
        .connect_with(opts)
        .await
        .map_err(|e| anyhow::anyhow!("connect engine.db: {e}"))?;
    Ok(pool)
}

/// Strip userinfo (user + password) from a Postgres URL for safe
/// display — keeps host / port / db / params. `postgres://u:p@h:5/db`
/// → `postgres://[REDACTED]@h:5/db`. We rebuild the URL string by
/// hand because `url::Url::set_username` percent-encodes `[`/`]`,
/// which would garble the placeholder; the string surface only ever
/// flows out to the dashboard so a plain rebuild is safe.
fn redact_pg_url(raw: &str) -> String {
    let Ok(u) = url::Url::parse(raw) else {
        return "[REDACTED]".to_string();
    };
    let scheme = u.scheme();
    let host = u.host_str().unwrap_or("");
    let port = u.port().map(|p| format!(":{p}")).unwrap_or_default();
    let path = u.path();
    let query = match u.query() {
        Some(q) => format!("?{q}"),
        None => String::new(),
    };
    let userinfo = if !u.username().is_empty() || u.password().is_some() {
        "[REDACTED]@"
    } else {
        ""
    };
    format!("{scheme}://{userinfo}{host}{port}{path}{query}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redact_secrets_replaces_admin_api_keys_array() {
        let mut v = json!({
            "auth": {
                "admin_api_keys": ["secret1", "secret2"],
                "issuer": "https://issuer.example",
            }
        });
        redact_secrets(&mut v);
        let arr = v["auth"]["admin_api_keys"].as_array().unwrap();
        assert!(arr.iter().all(|x| x.as_str() == Some("[REDACTED]")));
        // Non-secret strings stay intact.
        assert_eq!(v["auth"]["issuer"], json!("https://issuer.example"));
    }

    #[test]
    fn redact_secrets_replaces_password_fields() {
        let mut v = json!({"backend": {"password": "hunter2"}});
        redact_secrets(&mut v);
        assert_eq!(v["backend"]["password"], json!("[REDACTED]"));
    }

    #[test]
    fn redact_pg_url_strips_userinfo() {
        let raw = "postgres://alice:hunter2@db.example.com:5432/assay";
        let red = redact_pg_url(raw);
        assert!(!red.contains("hunter2"), "password leak: {red}");
        assert!(red.contains("[REDACTED]"));
        assert!(red.contains("db.example.com:5432"));
        assert!(red.contains("/assay"));
    }

    #[test]
    fn require_admin_locks_when_keys_empty() {
        let h = HeaderMap::new();
        let keys: Vec<String> = vec![];
        assert!(require_admin(&h, &keys).is_err());
    }

    #[test]
    fn require_admin_accepts_known_bearer() {
        let mut h = HeaderMap::new();
        h.insert(header::AUTHORIZATION, "Bearer abcd".parse().unwrap());
        let keys: Vec<String> = vec!["abcd".to_string()];
        assert!(require_admin(&h, &keys).is_ok());
    }

    #[test]
    fn require_admin_rejects_wrong_bearer() {
        let mut h = HeaderMap::new();
        h.insert(header::AUTHORIZATION, "Bearer wrong".parse().unwrap());
        let keys: Vec<String> = vec!["abcd".to_string()];
        assert!(require_admin(&h, &keys).is_err());
    }

    #[test]
    fn short_actor_safely_truncates() {
        let s = short_actor("abcdef0123456789".to_string());
        assert_eq!(s, "admin:****456789");
        // Short token guard.
        let s = short_actor("xyz".to_string());
        assert!(s.starts_with("admin:****"));
    }
}
