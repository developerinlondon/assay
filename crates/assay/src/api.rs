//! Authenticated HTTP surface for gated Lua runs.
//!
//! `mcp-serve` speaks stdio, so a host that wants the runtime in a separate
//! trust domain has had to invent a protocol over the CLI. This serves the
//! two operations such a host actually needs — run a gated script, resume a
//! suspended one — over the network, behind a bearer token.

use std::process::ExitCode;
use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::{HeaderMap, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{Value as JsonValue, json};

use crate::lua;
use crate::mcp::{TempScript, parse_string_array, resolve_mode, write_temp_script};
use crate::tool_mode::{ToolModeRequest, execute_tool_mode, resume_tool_outcome};

/// Comma-separated bearer tokens accepted by this server. The process
/// refuses to start without at least one: a runtime reachable over the
/// network with no credential is never what an operator meant.
pub const API_TOKENS_ENV: &str = "ASSAY_API_TOKENS";

const DEFAULT_TIMEOUT_SECS: u64 = 20;
const MAX_TIMEOUT_SECS: u64 = 600;

#[derive(Clone)]
struct ApiState {
    tokens: Arc<Vec<String>>,
}

pub async fn serve(bind: &str) -> ExitCode {
    let tokens = match configured_tokens() {
        Ok(tokens) => tokens,
        Err(message) => {
            eprintln!("api: {message}");
            return ExitCode::from(2);
        }
    };

    let state = ApiState {
        tokens: Arc::new(tokens),
    };
    let guarded = Router::new()
        .route("/v1/run", post(run_handler))
        .route("/v1/resume", post(resume_handler))
        .layer(middleware::from_fn_with_state(state.clone(), require_token))
        .with_state(state);
    let app = Router::new()
        .route("/healthz", get(|| async { Json(json!({ "ok": true })) }))
        .merge(guarded);

    let listener = match tokio::net::TcpListener::bind(bind).await {
        Ok(listener) => listener,
        Err(e) => {
            eprintln!("api: binding {bind}: {e}");
            return ExitCode::from(1);
        }
    };
    tracing::info!(bind, "assay api listening");

    match axum::serve(listener, app).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("api: serving: {e}");
            ExitCode::from(1)
        }
    }
}

fn configured_tokens() -> Result<Vec<String>, String> {
    let raw = std::env::var(API_TOKENS_ENV).unwrap_or_default();
    let tokens: Vec<String> = raw
        .split(',')
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(str::to_owned)
        .collect();
    if tokens.is_empty() {
        return Err(format!(
            "{API_TOKENS_ENV} is empty — refusing to serve an ungated runtime"
        ));
    }
    Ok(tokens)
}

/// Compares against every configured token without short-circuiting, so
/// response time does not narrow the search for an attacker.
fn authorized(headers: &HeaderMap, tokens: &[String]) -> bool {
    let Some(presented) = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
    else {
        return false;
    };
    let mut matched = false;
    for token in tokens {
        matched |= constant_time_eq(token.as_bytes(), presented.as_bytes());
    }
    matched
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

fn envelope_response(envelope: String) -> Response {
    (
        StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        envelope,
    )
        .into_response()
}

fn error_response(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({ "ok": false, "error": message }))).into_response()
}

fn resolve_timeout(body: &JsonValue) -> u64 {
    body.get("timeout_secs")
        .and_then(JsonValue::as_u64)
        .unwrap_or(DEFAULT_TIMEOUT_SECS)
        .clamp(1, MAX_TIMEOUT_SECS)
}

async fn require_token(State(state): State<ApiState>, request: Request, next: Next) -> Response {
    if !authorized(request.headers(), &state.tokens) {
        return error_response(StatusCode::UNAUTHORIZED, "missing or invalid bearer token");
    }
    next.run(request).await
}

async fn run_handler(Json(body): Json<JsonValue>) -> Response {
    let Some(script) = body
        .get("script")
        .and_then(JsonValue::as_str)
        .map(str::to_owned)
    else {
        return error_response(StatusCode::BAD_REQUEST, "script must be a string");
    };
    let exec_mode = match resolve_mode(body.get("mode")) {
        Ok(mode) => mode,
        Err(message) => return error_response(StatusCode::BAD_REQUEST, &message),
    };
    let timeout_secs = resolve_timeout(&body);
    let script_args = parse_string_array(body.get("args"));

    match run_on_own_thread(script, script_args, exec_mode, timeout_secs).await {
        Ok(envelope) => envelope_response(envelope),
        Err(message) => error_response(StatusCode::INTERNAL_SERVER_ERROR, &message),
    }
}

/// The Lua VM is `!Send`, so it cannot live across an await inside an axum
/// handler. Each run gets its own thread and current-thread runtime, and
/// only the finished envelope crosses back.
async fn run_on_own_thread(
    script: String,
    script_args: Vec<String>,
    exec_mode: lua::ExecMode,
    timeout_secs: u64,
) -> Result<String, String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    std::thread::spawn(move || {
        let result = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| format!("building runtime: {e}"))
            .and_then(|runtime| {
                let script_file =
                    write_temp_script(&script).map_err(|e| format!("writing temp script: {e}"))?;
                let outcome = runtime.block_on(execute_tool_mode(ToolModeRequest {
                    path: &script_file.path,
                    script: lua::async_bridge::strip_shebang(&script),
                    timeout_secs,
                    script_args,
                    exec_mode,
                    approval: &lua::approval_config_from_env(),
                }));
                cleanup_unless_suspended(&script_file, outcome.status);
                Ok(outcome.envelope)
            });
        let _ = tx.send(result);
    });
    rx.await
        .map_err(|_| "run thread ended without a result".to_string())?
}

/// A suspended run's resume token points back at this file, so it may only
/// be removed once the run has reached a terminal state.
fn cleanup_unless_suspended(script_file: &TempScript, status: &str) {
    if status != "needs_approval" {
        script_file.cleanup();
    }
}

async fn resume_handler(Json(body): Json<JsonValue>) -> Response {
    let Some(token) = body.get("token").and_then(JsonValue::as_str) else {
        return error_response(StatusCode::BAD_REQUEST, "token must be a string");
    };
    let Some(approve) = body.get("approve").and_then(JsonValue::as_bool) else {
        return error_response(StatusCode::BAD_REQUEST, "approve must be a boolean");
    };
    let approver = body.get("approver").and_then(JsonValue::as_str);

    let outcome = resume_tool_outcome(
        token,
        if approve { "yes" } else { "no" },
        None,
        false,
        approver,
    )
    .await;
    envelope_response(outcome.envelope)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers_with(value: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            value.parse().expect("header value"),
        );
        headers
    }

    #[test]
    fn a_matching_bearer_token_is_accepted() {
        let tokens = vec!["alpha".to_string(), "beta".to_string()];
        assert!(authorized(&headers_with("Bearer beta"), &tokens));
    }

    #[test]
    fn a_wrong_or_absent_token_is_refused() {
        let tokens = vec!["alpha".to_string()];
        assert!(!authorized(&headers_with("Bearer nope"), &tokens));
        assert!(!authorized(&headers_with("alpha"), &tokens));
        assert!(!authorized(&HeaderMap::new(), &tokens));
    }

    #[test]
    fn a_token_that_is_a_prefix_of_a_real_one_is_refused() {
        let tokens = vec!["alphabet".to_string()];
        assert!(!authorized(&headers_with("Bearer alpha"), &tokens));
    }

    #[test]
    fn timeouts_are_clamped_rather_than_trusted() {
        assert_eq!(resolve_timeout(&json!({})), DEFAULT_TIMEOUT_SECS);
        assert_eq!(resolve_timeout(&json!({ "timeout_secs": 0 })), 1);
        assert_eq!(
            resolve_timeout(&json!({ "timeout_secs": 999_999 })),
            MAX_TIMEOUT_SECS
        );
    }
}
