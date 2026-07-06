//! Model Context Protocol server over stdio (JSON-RPC 2.0).
//!
//! Exposes two tools — `assay_run` and `assay_context` — so an MCP client
//! composes every embedded module through a single gated Lua entry point
//! instead of one tool per module. Transport is newline-delimited JSON per
//! the MCP stdio spec: one JSON-RPC message per line, no embedded newlines.

use serde_json::{Value as JsonValue, json};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::lua;

const SERVER_NAME: &str = "assay";
const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");
const DEFAULT_PROTOCOL_VERSION: &str = "2024-11-05";
const DEFAULT_CONTEXT_LIMIT: usize = 5;

/// Opt-in gate for the third execution mode over MCP. Off by default: the
/// server advertises and accepts only `readonly` + `approval`, so any MCP
/// client is safe by default and can never fall through to unrestricted
/// execution. A deployment that gates access itself — resolving the caller's
/// allowed mode from its own policy before ever passing `unrestricted` — sets
/// this to `1`/`true` to enable the full three-mode ladder.
const UNRESTRICTED_ENV: &str = "ASSAY_MCP_UNRESTRICTED";

fn unrestricted_allowed() -> bool {
    matches!(
        std::env::var(UNRESTRICTED_ENV)
            .ok()
            .as_deref()
            .map(str::trim),
        Some("1") | Some("true")
    )
}

static CALL_COUNTER: AtomicU64 = AtomicU64::new(0);

// JSON-RPC 2.0 error codes.
const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;

/// Run the MCP server loop until stdin reaches EOF (clean shutdown).
pub async fn serve() -> ExitCode {
    let mut reader = BufReader::new(tokio::io::stdin());
    let mut stdout = tokio::io::stdout();
    let mut line = String::new();

    loop {
        line.clear();
        match reader.read_line(&mut line).await {
            Ok(0) => break, // Client closed stdin.
            Ok(_) => {}
            Err(e) => {
                eprintln!("mcp: reading stdin: {e}");
                return ExitCode::from(1);
            }
        }

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let response = match serde_json::from_str::<JsonValue>(trimmed) {
            Ok(request) => handle_message(request).await,
            Err(_) => Some(error_response(JsonValue::Null, PARSE_ERROR, "parse error")),
        };

        if let Some(response) = response
            && let Err(e) = write_message(&mut stdout, &response).await
        {
            eprintln!("mcp: writing stdout: {e}");
            return ExitCode::from(1);
        }
    }

    ExitCode::SUCCESS
}

/// Dispatch a single parsed JSON-RPC message. Returns `None` for
/// notifications (requests without an `id`), which never get a reply.
async fn handle_message(request: JsonValue) -> Option<JsonValue> {
    let Some(id) = request.get("id").cloned() else {
        return None; // Notification: fire-and-forget, acted on by none.
    };

    let method = match request.get("method").and_then(JsonValue::as_str) {
        Some(m) => m,
        None => return Some(error_response(id, INVALID_REQUEST, "missing method")),
    };

    let params = request.get("params").cloned().unwrap_or(JsonValue::Null);

    let response = match method {
        "initialize" => success_response(id, initialize_result(&params)),
        "ping" => success_response(id, json!({})),
        "tools/list" => success_response(id, tools_list_result()),
        "tools/call" => handle_tools_call(id, params).await,
        other => error_response(id, METHOD_NOT_FOUND, &format!("method not found: {other}")),
    };
    Some(response)
}

fn initialize_result(params: &JsonValue) -> JsonValue {
    let protocol_version = params
        .get("protocolVersion")
        .and_then(JsonValue::as_str)
        .filter(|v| !v.is_empty())
        .unwrap_or(DEFAULT_PROTOCOL_VERSION);

    json!({
        "protocolVersion": protocol_version,
        "capabilities": { "tools": {} },
        "serverInfo": { "name": SERVER_NAME, "version": SERVER_VERSION },
    })
}

fn tools_list_result() -> JsonValue {
    json!({ "tools": [assay_run_tool(), assay_context_tool()] })
}

fn assay_run_tool() -> JsonValue {
    // The advertised mode ladder and its prose track whether this server was
    // started with unrestricted execution enabled, so the client only ever
    // sees a mode it can actually request.
    let (modes, description, mode_description): (Vec<&str>, &str, &str) = if unrestricted_allowed()
    {
        (
            vec!["readonly", "approval", "unrestricted"],
            "Run a Lua script through the assay runtime and return the tool-mode JSON envelope (status ok | needs_approval | error). Every embedded assay module is available via require(\"assay.<module>\") alongside the builtins (http, json, fs, crypto, ...). Execution is gated by mode: 'readonly' blocks mutating builtins, 'approval' suspends each mutating operation and returns a resume token, 'unrestricted' runs everything ungated. This server was started with unrestricted execution enabled; the caller is responsible for deciding who may request it.",
            "Execution gate. 'readonly' (default) blocks mutating builtins; 'approval' suspends each mutating operation for per-operation approval; 'unrestricted' runs everything with no gate.",
        )
    } else {
        (
            vec!["readonly", "approval"],
            "Run a Lua script through the assay runtime and return the tool-mode JSON envelope (status ok | needs_approval | error). Every embedded assay module is available via require(\"assay.<module>\") alongside the builtins (http, json, fs, crypto, ...). Execution is always gated: 'readonly' blocks mutating builtins, 'approval' suspends each mutating operation and returns a resume token. Unrestricted execution is not exposed by this server.",
            "Execution gate. 'readonly' (default) blocks mutating builtins; 'approval' suspends each mutating operation for per-operation approval. 'unrestricted' is not accepted.",
        )
    };
    json!({
        "name": "assay_run",
        "description": description,
        "inputSchema": {
            "type": "object",
            "properties": {
                "script": {
                    "type": "string",
                    "description": "Lua source to execute. Return a table or value to surface it as the tool output.",
                },
                "mode": {
                    "type": "string",
                    "enum": modes,
                    "default": "readonly",
                    "description": mode_description,
                },
                "timeout_secs": {
                    "type": "number",
                    "description": "Maximum execution time in seconds (default 20).",
                },
                "args": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Positional arguments exposed to the script as the 1-indexed `arg` global.",
                },
            },
            "required": ["script", "mode"],
        },
    })
}

fn assay_context_tool() -> JsonValue {
    json!({
        "name": "assay_context",
        "description": "Search assay's embedded modules and return prompt-ready Markdown docs (method signatures, env vars, builtins) for the matches. Call this before writing an assay_run script to discover the correct module APIs.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Module search query, e.g. 'grafana', 'kubernetes', 'oauth'.",
                },
                "limit": {
                    "type": "number",
                    "description": "Maximum number of modules to return (default 5).",
                },
            },
            "required": ["query"],
        },
    })
}

async fn handle_tools_call(id: JsonValue, params: JsonValue) -> JsonValue {
    let arguments = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));

    match params.get("name").and_then(JsonValue::as_str) {
        Some("assay_run") => match call_assay_run(&arguments).await {
            Ok(result) => success_response(id, result),
            Err(message) => success_response(id, tool_error_content(message)),
        },
        Some("assay_context") => match call_assay_context(&arguments) {
            Ok(result) => success_response(id, result),
            Err(message) => success_response(id, tool_error_content(message)),
        },
        Some(other) => error_response(id, INVALID_PARAMS, &format!("unknown tool: {other}")),
        None => error_response(id, INVALID_PARAMS, "missing tool name"),
    }
}

async fn call_assay_run(arguments: &JsonValue) -> Result<JsonValue, String> {
    let script = arguments
        .get("script")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| "assay_run requires a 'script' string argument".to_string())?;

    let exec_mode = resolve_mode(arguments.get("mode"))?;

    let timeout_secs = arguments
        .get("timeout_secs")
        .and_then(JsonValue::as_u64)
        .unwrap_or(crate::DEFAULT_TOOL_TIMEOUT_SECS);

    let script_args = parse_string_array(arguments.get("args"));

    let approval = if exec_mode.is_approval() {
        lua::approval_config_from_env()
    } else {
        lua::ApprovalConfig::default()
    };

    let script_file = write_temp_script(script).map_err(|e| format!("writing temp script: {e}"))?;
    let stripped = lua::async_bridge::strip_shebang(script);

    let outcome = crate::execute_tool_mode(
        &script_file.path,
        stripped,
        timeout_secs,
        script_args,
        exec_mode,
        &approval,
    )
    .await;

    // Keep the script on disk only while an approval gate still references
    // it through a resume token; otherwise clean it up eagerly.
    if outcome.status != "needs_approval" {
        script_file.cleanup();
    }

    let is_error = !matches!(outcome.status, "ok" | "needs_approval");
    Ok(tool_text_content(outcome.envelope, is_error))
}

fn call_assay_context(arguments: &JsonValue) -> Result<JsonValue, String> {
    let query = arguments
        .get("query")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| "assay_context requires a 'query' string argument".to_string())?;

    let limit = arguments
        .get("limit")
        .and_then(JsonValue::as_u64)
        .map(|n| n as usize)
        .filter(|n| *n > 0)
        .unwrap_or(DEFAULT_CONTEXT_LIMIT);

    let markdown = crate::render_context(query, limit)?;
    Ok(tool_text_content(markdown, false))
}

/// Map the requested mode to an `ExecMode`. An absent mode defaults to
/// read-only so a caller can never fall through to a less restricted mode.
/// `unrestricted` is accepted only when the server opted in via
/// `ASSAY_MCP_UNRESTRICTED` (see `unrestricted_allowed`); otherwise it — and
/// any other value — is rejected.
fn resolve_mode(value: Option<&JsonValue>) -> Result<lua::ExecMode, String> {
    let accepted = if unrestricted_allowed() {
        "\"readonly\", \"approval\", or \"unrestricted\""
    } else {
        "\"readonly\" or \"approval\""
    };
    match value {
        None | Some(JsonValue::Null) => Ok(lua::ExecMode::ReadOnly),
        Some(JsonValue::String(mode)) => match mode.as_str() {
            "readonly" => Ok(lua::ExecMode::ReadOnly),
            "approval" => Ok(lua::ExecMode::Approval),
            "unrestricted" if unrestricted_allowed() => Ok(lua::ExecMode::Unrestricted),
            "unrestricted" => Err(
                "mode \"unrestricted\" is not enabled on this server (start it with \
                 ASSAY_MCP_UNRESTRICTED=1 to allow it)"
                    .to_string(),
            ),
            other => Err(format!(
                "mode must be {accepted}; \"{other}\" is not accepted"
            )),
        },
        Some(_) => Err(format!("mode must be a string: {accepted}")),
    }
}

fn parse_string_array(value: Option<&JsonValue>) -> Vec<String> {
    value
        .and_then(JsonValue::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

fn tool_text_content(text: String, is_error: bool) -> JsonValue {
    json!({
        "content": [{ "type": "text", "text": text }],
        "isError": is_error,
    })
}

fn tool_error_content(message: String) -> JsonValue {
    tool_text_content(message, true)
}

fn success_response(id: JsonValue, result: JsonValue) -> JsonValue {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn error_response(id: JsonValue, code: i64, message: &str) -> JsonValue {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

async fn write_message(stdout: &mut tokio::io::Stdout, message: &JsonValue) -> std::io::Result<()> {
    let mut bytes = serde_json::to_vec(message).unwrap_or_else(|_| b"{}".to_vec());
    bytes.push(b'\n');
    stdout.write_all(&bytes).await?;
    stdout.flush().await
}

/// A Lua script materialised on disk for a single `assay_run` call. The
/// path feeds the tool-mode runner and any resume token it persists.
struct TempScript {
    path: PathBuf,
    dir: PathBuf,
}

impl TempScript {
    fn cleanup(&self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn write_temp_script(script: &str) -> std::io::Result<TempScript> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let seq = CALL_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("assay-mcp-{nonce}-{seq}"));
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("script.lua");
    std::fs::write(&path, script)?;
    Ok(TempScript { path, dir })
}
