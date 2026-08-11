use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::process::ExitCode;
use std::time::Duration;

use mlua::LuaSerdeExt;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use tracing::info;

use crate::lua;
use crate::{
    APPROVAL_REQUEST_PREFIX, DEFAULT_RESUME_TTL_SECS, TOOL_STDOUT_CAP_BYTES, build_http_client,
    install_script_args,
};

pub(crate) fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Serialize)]
pub(crate) struct ToolSuccessEnvelope {
    ok: bool,
    status: &'static str,
    output: JsonValue,
    #[serde(rename = "requiresApproval")]
    requires_approval: Option<JsonValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    truncated: Option<bool>,
    #[serde(skip_serializing_if = "is_false")]
    readonly: bool,
}

#[derive(Serialize)]
pub(crate) struct ToolErrorEnvelope {
    ok: bool,
    status: &'static str,
    error: String,
    #[serde(skip_serializing_if = "is_false")]
    readonly: bool,
}

#[derive(Deserialize)]
pub(crate) struct ApprovalRequestPayload {
    prompt: String,
    #[serde(default)]
    context: JsonValue,
    #[serde(default)]
    op: Option<String>,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    index: Option<u64>,
    #[serde(default)]
    digest: Option<String>,
    #[serde(default)]
    headers: Vec<String>,
}

#[derive(Deserialize, Serialize)]
pub(crate) struct ResumeState {
    script_path: PathBuf,
    approval_prompt: String,
    approval_context: JsonValue,
    created_at: u64,
    ttl_secs: u64,
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    approved_indices: Vec<u64>,
    #[serde(default)]
    pending_index: Option<u64>,
    #[serde(default)]
    op: Option<String>,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    digest: Option<String>,
    #[serde(default)]
    headers: Vec<String>,
    #[serde(default)]
    script_args: Vec<String>,
    /// Every grant issued so far in this run's approval chain: the index,
    /// the op it was approved for, and (when supplied) who approved it.
    #[serde(default)]
    approved_ops: Vec<lua::ApprovedOp>,
}

/// One tool-mode invocation. Bundled rather than passed as six positional
/// arguments so the CLI entry point and the MCP path cannot drift apart.
pub(crate) struct ToolModeRequest<'a> {
    pub path: &'a std::path::Path,
    pub script: &'a str,
    pub timeout_secs: u64,
    pub script_args: Vec<String>,
    pub exec_mode: lua::ExecMode,
    pub approval: &'a lua::ApprovalConfig,
}

pub(crate) async fn run_lua_tool_mode(req: ToolModeRequest<'_>) -> ExitCode {
    let outcome = execute_tool_mode(req).await;
    print!("{}", outcome.envelope);
    ExitCode::SUCCESS
}

/// Result of a tool-mode run: the serialized JSON envelope plus its
/// `status` field. Shared by the CLI (prints `envelope`) and the MCP
/// server (surfaces `envelope` as tool content, maps `status` to isError).
pub struct ToolModeOutcome {
    pub envelope: String,
    pub status: &'static str,
}

/// Execute a Lua script in tool mode and return the JSON envelope without
/// emitting it. `status` mirrors the envelope's own field: "ok",
/// "needs_approval", "error", or "timeout".
pub(crate) async fn execute_tool_mode(req: ToolModeRequest<'_>) -> ToolModeOutcome {
    let readonly = req.exec_mode.is_readonly();
    info!(
        script = %req.path.display(),
        timeout_secs = req.timeout_secs,
        readonly,
        "starting assay (tool mode)"
    );

    let (vm, tool_script) = match prepare_tool_vm(&req) {
        Ok(prepared) => prepared,
        Err(message) => {
            return ToolModeOutcome {
                envelope: build_tool_error("error", message, readonly),
                status: "error",
            };
        }
    };

    let local = tokio::task::LocalSet::new();
    let execution = local.run_until(async {
        vm.load(&tool_script)
            .set_name(format!("@{}", req.path.display()))
            .eval_async::<mlua::Value>()
            .await
    });
    let result = tokio::time::timeout(Duration::from_secs(req.timeout_secs), execution).await;

    classify_tool_result(result, &vm, &req, readonly)
}

/// Build the VM for one tool-mode run. Under a gate (read-only or approval)
/// `env.set` is itself gated, so the mode marker goes through the check-env
/// table rather than a script prefix that would trip the gate.
fn prepare_tool_vm(req: &ToolModeRequest<'_>) -> Result<(mlua::Lua, String), String> {
    let gated = req.exec_mode.is_readonly() || req.exec_mode.is_approval();
    let tool_script = if gated {
        req.script.to_string()
    } else {
        format!("env.set(\"ASSAY_MODE\", \"tool\")\n{}", req.script)
    };

    let vm = lua::create_vm_with_options(
        build_http_client(),
        lua::VmOptions {
            global_modules_path: None,
            mode: req.exec_mode,
            approval: req.approval.clone(),
        },
    )
    .map_err(|e| format!("creating Lua VM: {e:#}"))?;

    if gated {
        let mode_env =
            std::collections::HashMap::from([("ASSAY_MODE".to_string(), "tool".to_string())]);
        lua::inject_env(&vm, &mode_env).map_err(|e| format!("injecting ASSAY_MODE: {e:#}"))?;
    }

    install_script_args(&vm, req.path, &req.script_args)
        .map_err(|e| format!("installing arg global: {e}"))?;

    Ok((vm, tool_script))
}

fn classify_tool_result(
    result: Result<mlua::Result<mlua::Value>, tokio::time::error::Elapsed>,
    vm: &mlua::Lua,
    req: &ToolModeRequest<'_>,
    readonly: bool,
) -> ToolModeOutcome {
    let err = |message: String| ToolModeOutcome {
        envelope: build_tool_error("error", message, readonly),
        status: "error",
    };

    match result {
        Ok(Ok(value)) => match lua_value_to_json(vm, value) {
            Ok(output) => ToolModeOutcome {
                envelope: build_tool_success(output, readonly),
                status: "ok",
            },
            Err(e) => err(format!("serializing Lua result: {e}")),
        },
        Ok(Err(e)) => {
            let Some(request) = extract_approval_request(&e) else {
                return err(format_lua_error(&e));
            };
            match persist_resume_state(
                req.path,
                request,
                req.exec_mode,
                &req.approval.approved_indices,
                &lua::approved_ops_from_env(),
                &req.script_args,
            ) {
                Ok(requires_approval) => ToolModeOutcome {
                    envelope: build_tool_needs_approval(requires_approval, readonly),
                    status: "needs_approval",
                },
                Err(message) => err(message),
            }
        }
        Err(_) => ToolModeOutcome {
            envelope: build_tool_error(
                "timeout",
                format!("execution timed out after {}s", req.timeout_secs),
                readonly,
            ),
            status: "timeout",
        },
    }
}

/// Resume a suspended tool-mode run and RETURN its envelope without emitting
/// it. Shared by the CLI `resume` command (which prints the envelope) and the
/// MCP `assay_resume` tool (which surfaces it as tool content). `approve` is
/// "yes" to approve the pending operation, anything else to deny it. The
/// child's stderr is forwarded as diagnostics; its stdout is the resumed run's
/// JSON envelope, returned here with its parsed `status`.
pub(crate) async fn resume_tool_outcome(
    token: &str,
    approve: &str,
    resume_ttl: Option<u64>,
    readonly: bool,
    approver: Option<&str>,
) -> ToolModeOutcome {
    let err_outcome = |message: String| ToolModeOutcome {
        envelope: build_tool_error("error", message, readonly),
        status: "error",
    };

    let state_dir = match resolve_state_dir() {
        Ok(dir) => dir,
        Err(err) => return err_outcome(err),
    };
    let state_path = state_dir.join("resume").join(format!("{token}.json"));
    let state = match load_resume_state(&state_path, resume_ttl) {
        Ok(state) => state,
        Err(err) => return err_outcome(err),
    };

    info!(
        decision = approve,
        approver = approver.unwrap_or("-"),
        op = state.op.as_deref().unwrap_or("-"),
        index = ?state.pending_index,
        "resume decision"
    );

    let mut command = match resume_command(&state, &state_dir, approve) {
        Ok(command) => command,
        Err(err) => return err_outcome(err),
    };
    apply_resume_decision(&mut command, &state, approve, readonly, approver);

    let output = match command.output() {
        Ok(output) => output,
        Err(err) => return err_outcome(format!("spawning resume execution: {err}")),
    };
    finish_resume(output, &state_path, approver)
}

fn load_resume_state(
    state_path: &std::path::Path,
    resume_ttl: Option<u64>,
) -> Result<ResumeState, String> {
    if !state_path.exists() {
        return Err("invalid resume token".to_string());
    }
    let content =
        fs::read_to_string(state_path).map_err(|err| format!("reading resume state: {err}"))?;
    let state: ResumeState =
        serde_json::from_str(&content).map_err(|err| format!("parsing resume state: {err}"))?;

    let ttl_secs = resume_ttl.unwrap_or(state.ttl_secs);
    if state.created_at.saturating_add(ttl_secs) < unix_timestamp_now() {
        return Err("resume token expired".to_string());
    }
    Ok(state)
}

fn resume_command(
    state: &ResumeState,
    state_dir: &std::path::Path,
    approve: &str,
) -> Result<Command, String> {
    let current_exe =
        std::env::current_exe().map_err(|err| format!("locating assay binary: {err}"))?;
    let mut command = Command::new(current_exe);
    command.args([
        "run",
        "--mode",
        "tool",
        state.script_path.to_string_lossy().as_ref(),
    ]);
    // Re-run with the original positional arguments so the `arg` global —
    // and any control flow that depends on it — is identical across runs.
    if !state.script_args.is_empty() {
        command.arg("--");
        command.args(&state.script_args);
    }
    command
        .env("ASSAY_MODE", "tool")
        .env("ASSAY_APPROVAL_RESULT", approve)
        .env("ASSAY_STATE_DIR", state_dir);
    Ok(command)
}

fn apply_resume_decision(
    command: &mut Command,
    state: &ResumeState,
    approve: &str,
    readonly: bool,
    approver: Option<&str>,
) {
    if state.mode.as_deref() != Some("approval") {
        if readonly {
            command.env(lua::READONLY_ENV, "1");
        }
        return;
    }

    command.env(lua::APPROVAL_ENV, "1");
    let mut approved = state.approved_indices.clone();
    let mut approved_ops = state.approved_ops.clone();
    match (approve, state.pending_index) {
        ("yes", Some(idx)) => {
            if !approved.contains(&idx) {
                approved.push(idx);
            }
            // Bind the grant to the op it was issued for, so the re-run
            // cannot spend it on a different operation.
            if let Some(op) = state.op.clone()
                && !approved_ops.iter().any(|entry| entry.index == idx)
            {
                approved_ops.push(lua::ApprovedOp {
                    index: idx,
                    op,
                    digest: state.digest.clone(),
                    approver: approver.map(str::to_owned),
                });
            }
        }
        (_, Some(idx)) => {
            command.env(lua::DENIED_INDEX_ENV, idx.to_string());
        }
        _ => {}
    }

    let joined = approved
        .iter()
        .map(u64::to_string)
        .collect::<Vec<_>>()
        .join(",");
    command.env(lua::APPROVED_INDICES_ENV, joined);
    if !approved_ops.is_empty()
        && let Ok(serialized) = serde_json::to_string(&approved_ops)
    {
        command.env(lua::APPROVED_OPS_ENV, serialized);
    }
}

fn finish_resume(
    output: std::process::Output,
    state_path: &std::path::Path,
    approver: Option<&str>,
) -> ToolModeOutcome {
    // The child's stderr is diagnostic (logs) — forward it. Safe on the CLI
    // and on the MCP stdout channel alike, which only ever carries the envelope.
    if !output.stderr.is_empty() {
        eprint!("{}", String::from_utf8_lossy(&output.stderr));
    }

    let raw = String::from_utf8_lossy(&output.stdout).into_owned();
    let resumed_status = serde_json::from_slice::<JsonValue>(&output.stdout)
        .ok()
        .and_then(|json| json.get("status").cloned())
        .and_then(|status| status.as_str().map(str::to_owned));

    // Echo the audit identity in the envelope the caller sees. Best-effort:
    // a non-object envelope is passed through untouched.
    let envelope = match approver {
        Some(who) => match serde_json::from_str::<JsonValue>(&raw) {
            Ok(JsonValue::Object(mut map)) => {
                map.insert("approver".to_string(), JsonValue::String(who.to_string()));
                serde_json::to_string(&JsonValue::Object(map)).unwrap_or(raw)
            }
            _ => raw,
        },
        None => raw,
    };

    // Drop the token once the run reaches a terminal state; a still-pending
    // approval keeps it for the next resume. Best-effort: a cleanup hiccup
    // must not turn a successful resume into an error.
    if output.status.success() && resumed_status.as_deref() != Some("needs_approval") {
        let _ = fs::remove_file(state_path);
    }

    let status: &'static str = match resumed_status.as_deref() {
        Some("ok") => "ok",
        Some("needs_approval") => "needs_approval",
        Some("timeout") => "timeout",
        _ => "error",
    };
    ToolModeOutcome { envelope, status }
}

/// CLI `resume` command: resume a suspended run and print its envelope.
pub(crate) async fn resume_tool_execution(
    token: &str,
    approve: &str,
    resume_ttl: Option<u64>,
    readonly: bool,
    approver: Option<&str>,
) -> ExitCode {
    let outcome = resume_tool_outcome(token, approve, resume_ttl, readonly, approver).await;
    if !outcome.envelope.is_empty() {
        print!("{}", outcome.envelope);
    }
    ExitCode::SUCCESS
}

pub(crate) fn format_lua_error(err: &mlua::Error) -> String {
    match err {
        mlua::Error::RuntimeError(msg) => msg.clone(),
        mlua::Error::CallbackError { traceback, cause } => {
            let cause_msg = format_lua_error(cause);
            if traceback.is_empty() {
                cause_msg
            } else {
                format!("{cause_msg}\n{traceback}")
            }
        }
        other => format!("{other}"),
    }
}

pub(crate) fn lua_value_to_json(
    lua: &mlua::Lua,
    value: mlua::Value,
) -> Result<JsonValue, mlua::Error> {
    lua.from_value(value)
}

pub(crate) fn extract_approval_request(err: &mlua::Error) -> Option<ApprovalRequestPayload> {
    let message = format_lua_error(err);
    let start = message.find(APPROVAL_REQUEST_PREFIX)?;
    let payload = &message[start + APPROVAL_REQUEST_PREFIX.len()..];
    let json_payload = payload
        .split_once('\n')
        .map(|(json, _)| json)
        .unwrap_or(payload);
    serde_json::from_str(json_payload).ok()
}

pub(crate) fn persist_resume_state(
    script_path: &std::path::Path,
    request: ApprovalRequestPayload,
    mode: lua::ExecMode,
    approved_indices: &[u64],
    approved_ops: &[lua::ApprovedOp],
    script_args: &[String],
) -> Result<JsonValue, String> {
    let state_dir = resolve_state_dir()?;
    let resume_dir = state_dir.join("resume");
    fs::create_dir_all(&resume_dir)
        .map_err(|err| format!("creating resume state directory: {err}"))?;

    let token = format!("{:032x}", rand::random::<u128>());
    let resolved_script_path = if script_path.is_absolute() {
        script_path.to_path_buf()
    } else {
        match script_path.canonicalize() {
            Ok(path) => path,
            Err(_) => script_path.to_path_buf(),
        }
    };
    let mode_label = mode.is_approval().then(|| "approval".to_string());
    let state = ResumeState {
        script_path: resolved_script_path,
        approval_prompt: request.prompt.clone(),
        approval_context: request.context.clone(),
        created_at: unix_timestamp_now(),
        ttl_secs: DEFAULT_RESUME_TTL_SECS,
        mode: mode_label,
        approved_indices: approved_indices.to_vec(),
        pending_index: request.index,
        op: request.op.clone(),
        summary: request.summary.clone(),
        digest: request.digest.clone(),
        headers: request.headers.clone(),
        script_args: script_args.to_vec(),
        approved_ops: approved_ops.to_vec(),
    };

    let serialized =
        serde_json::to_vec(&state).map_err(|err| format!("serializing resume state: {err}"))?;
    fs::write(resume_dir.join(format!("{token}.json")), serialized)
        .map_err(|err| format!("writing resume state: {err}"))?;

    let mut requires = serde_json::Map::new();
    requires.insert("prompt".to_string(), JsonValue::String(request.prompt));
    requires.insert("context".to_string(), request.context);
    requires.insert("resumeToken".to_string(), JsonValue::String(token));
    if let Some(op) = request.op {
        requires.insert("op".to_string(), JsonValue::String(op));
    }
    if let Some(summary) = request.summary {
        requires.insert("summary".to_string(), JsonValue::String(summary));
    }
    if let Some(index) = request.index {
        requires.insert("index".to_string(), JsonValue::from(index));
    }
    // The approver sees which exact request they are authorising: the
    // digest binds the grant, and header names identify the credentials in
    // play without printing their values.
    if let Some(digest) = request.digest {
        requires.insert("digest".to_string(), JsonValue::String(digest));
    }
    if !request.headers.is_empty() {
        requires.insert(
            "headers".to_string(),
            JsonValue::from(request.headers.clone()),
        );
    }
    Ok(JsonValue::Object(requires))
}

pub(crate) fn resolve_state_dir() -> Result<PathBuf, String> {
    if let Ok(dir) = std::env::var("ASSAY_STATE_DIR") {
        return Ok(PathBuf::from(dir));
    }
    if let Ok(dir) = std::env::var("OPENCLAW_STATE_DIR") {
        return Ok(PathBuf::from(dir));
    }

    match std::env::var("HOME") {
        Ok(home) => Ok(PathBuf::from(home).join(".assay").join("state")),
        Err(_) => Err("resolving state directory: HOME is not set".to_string()),
    }
}

pub(crate) fn unix_timestamp_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub(crate) fn build_tool_success(output: JsonValue, readonly: bool) -> String {
    let mut envelope = ToolSuccessEnvelope {
        ok: true,
        status: "ok",
        output,
        requires_approval: None,
        truncated: None,
        readonly,
    };

    if let Ok(serialized) = serde_json::to_vec(&envelope)
        && serialized.len() > TOOL_STDOUT_CAP_BYTES
    {
        envelope = truncate_tool_envelope(envelope);
    }

    match serde_json::to_string(&envelope) {
        Ok(serialized) => serialized,
        Err(e) => build_tool_error("error", format!("serializing tool envelope: {e}"), readonly),
    }
}

pub(crate) fn build_tool_needs_approval(requires_approval: JsonValue, readonly: bool) -> String {
    let envelope = ToolSuccessEnvelope {
        ok: true,
        status: "needs_approval",
        output: JsonValue::Null,
        requires_approval: Some(requires_approval),
        truncated: None,
        readonly,
    };

    match serde_json::to_string(&envelope) {
        Ok(serialized) => serialized,
        Err(err) => build_tool_error(
            "error",
            format!("serializing tool envelope: {err}"),
            readonly,
        ),
    }
}

pub(crate) fn build_tool_error(
    status: &'static str,
    error_message: String,
    readonly: bool,
) -> String {
    let envelope = ToolErrorEnvelope {
        ok: false,
        status,
        error: error_message,
        readonly,
    };

    serde_json::to_string(&envelope).unwrap_or_else(|e| {
        format!(
            "{{\"ok\":false,\"status\":\"error\",\"error\":\"serializing tool envelope: {e}\"}}"
        )
    })
}

pub(crate) fn truncate_tool_envelope(mut envelope: ToolSuccessEnvelope) -> ToolSuccessEnvelope {
    let serialized_output =
        serde_json::to_string(&envelope.output).unwrap_or_else(|_| "null".to_string());
    let boundaries: Vec<usize> = serialized_output
        .char_indices()
        .map(|(idx, _)| idx)
        .chain(std::iter::once(serialized_output.len()))
        .collect();

    let suffix = if serialized_output.is_empty() {
        ""
    } else {
        "..."
    };
    let mut low = 0usize;
    let mut high = boundaries.len().saturating_sub(1);
    let mut best = JsonValue::String(suffix.to_string());

    while low <= high {
        let mid = low + (high - low) / 2;
        let candidate = format!("{}{}", &serialized_output[..boundaries[mid]], suffix);
        envelope.output = JsonValue::String(candidate.clone());
        envelope.truncated = Some(true);

        match serde_json::to_vec(&envelope) {
            Ok(serialized) if serialized.len() <= TOOL_STDOUT_CAP_BYTES => {
                best = JsonValue::String(candidate);
                low = mid.saturating_add(1);
            }
            _ => {
                if mid == 0 {
                    break;
                }
                high = mid - 1;
            }
        }
    }

    envelope.output = best;
    envelope.truncated = Some(true);
    envelope
}
