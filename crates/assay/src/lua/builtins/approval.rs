//! Approval-mode guards, applied after `register_all` when the VM is
//! created with `ExecMode::Approval`. Mutating builtins stay registered
//! but each call is gated by its sequence index: an operation whose index
//! is in the approved set runs against the original inner function;
//! otherwise the call raises an approval request carrying the operation
//! descriptor so the tool-mode resume machinery can suspend and collect a
//! decision. A per-VM counter assigns the index and advances on every
//! gated call, so successive resumes admit one more operation each.

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use mlua::{Lua, MultiValue, Table, Value};

use super::gated::{BLOCKED_FUNCTIONS, BLOCKED_TABLES, is_gated_http_verb};
use crate::lua::{APPROVAL_REQUEST_PREFIX, ApprovalConfig};

const SUMMARY_CAP: usize = 200;

struct GateState {
    counter: AtomicU64,
    approved: HashSet<u64>,
    denied: Option<u64>,
}

pub fn apply(lua: &Lua, config: &ApprovalConfig) -> mlua::Result<()> {
    let state = Arc::new(GateState {
        counter: AtomicU64::new(0),
        approved: config.approved_indices.iter().copied().collect(),
        denied: config.denied_index,
    });
    for path in BLOCKED_FUNCTIONS {
        gate_function(lua, path, &state)?;
    }
    for name in BLOCKED_TABLES {
        gate_table(lua, name, &state)?;
    }
    gate_http_client_request(lua, &state)?;
    gate_io_open(lua, &state)?;
    gate_io_output(lua, &state)?;
    Ok(())
}

/// Advance the counter and decide the fate of this call. `Ok(())` admits
/// the operation; an error either denies it terminally or raises the
/// approval request that suspends the run.
fn gate_decision(state: &GateState, op: &str, summary: &str) -> mlua::Result<()> {
    let index = state.counter.fetch_add(1, Ordering::SeqCst);
    if state.denied == Some(index) {
        return Err(mlua::Error::runtime(format!("approval: {op} denied")));
    }
    if state.approved.contains(&index) {
        return Ok(());
    }
    Err(approval_request(op, summary, index))
}

fn approval_request(op: &str, summary: &str, index: u64) -> mlua::Error {
    let payload = serde_json::json!({
        "prompt": format!("Approve {op}?"),
        "op": op,
        "summary": summary,
        "index": index,
    });
    mlua::Error::runtime(format!("{APPROVAL_REQUEST_PREFIX}{payload}"))
}

fn truncate(value: &str) -> String {
    if value.chars().count() > SUMMARY_CAP {
        let head: String = value.chars().take(SUMMARY_CAP).collect();
        format!("{head}...")
    } else {
        value.to_string()
    }
}

/// The salient argument for the descriptor: the first string argument
/// (url for http, path for fs, command for shell, sql for db.execute).
fn first_string_arg(args: &MultiValue) -> String {
    for value in args.iter() {
        if let Value::String(s) = value
            && let Ok(text) = s.to_str()
        {
            return truncate(&text);
        }
    }
    String::new()
}

fn gate_function(lua: &Lua, path: &str, state: &Arc<GateState>) -> mlua::Result<()> {
    let Some((table_name, fn_name)) = path.split_once('.') else {
        return Ok(());
    };
    let Some(table) = lua.globals().get::<Option<Table>>(table_name)? else {
        return Ok(());
    };
    let Value::Function(inner) = table.get::<Value>(fn_name)? else {
        return Ok(());
    };
    let op = path.to_string();
    let state = Arc::clone(state);
    let wrapper = lua.create_async_function(move |_, args: MultiValue| {
        let inner = inner.clone();
        let state = Arc::clone(&state);
        let op = op.clone();
        async move {
            let summary = first_string_arg(&args);
            gate_decision(&state, &op, &summary)?;
            inner.call_async::<MultiValue>(args).await
        }
    })?;
    table.set(fn_name, wrapper)?;
    Ok(())
}

fn gate_table(lua: &Lua, name: &str, state: &Arc<GateState>) -> mlua::Result<()> {
    let Some(table) = lua.globals().get::<Option<Table>>(name)? else {
        return Ok(());
    };
    for pair in table.clone().pairs::<Value, Value>() {
        let (key, value) = pair?;
        let (Value::String(key_str), Value::Function(inner)) = (&key, &value) else {
            continue;
        };
        let op = format!("{name}.{}", key_str.to_str()?);
        let inner = inner.clone();
        let state = Arc::clone(state);
        let wrapper = lua.create_async_function(move |_, args: MultiValue| {
            let inner = inner.clone();
            let state = Arc::clone(&state);
            let op = op.clone();
            async move {
                let summary = first_string_arg(&args);
                gate_decision(&state, &op, &summary)?;
                inner.call_async::<MultiValue>(args).await
            }
        })?;
        table.set(key.clone(), wrapper)?;
    }
    Ok(())
}

fn gate_http_client_request(lua: &Lua, state: &Arc<GateState>) -> mlua::Result<()> {
    let Some(http) = lua.globals().get::<Option<Table>>("http")? else {
        return Ok(());
    };
    let Some(inner) = http.get::<Option<mlua::Function>>("_client_request")? else {
        return Ok(());
    };
    let state = Arc::clone(state);
    let wrapper = lua.create_async_function(move |_, args: MultiValue| {
        let inner = inner.clone();
        let state = Arc::clone(&state);
        async move {
            let method = match args.iter().nth(1) {
                Some(Value::String(s)) => Some(s.to_str()?.to_string()),
                _ => None,
            };
            if let Some(method) = method
                && is_gated_http_verb(&method)
            {
                let op = format!("http.{method}");
                let summary = match args.iter().nth(2) {
                    Some(Value::String(s)) => truncate(&s.to_str()?),
                    _ => String::new(),
                };
                gate_decision(&state, &op, &summary)?;
            }
            inner.call_async::<MultiValue>(args).await
        }
    })?;
    http.set("_client_request", wrapper)?;
    Ok(())
}

fn gate_io_open(lua: &Lua, state: &Arc<GateState>) -> mlua::Result<()> {
    let Some(io_table) = lua.globals().get::<Option<Table>>("io")? else {
        return Ok(());
    };
    let Some(inner) = io_table.get::<Option<mlua::Function>>("open")? else {
        return Ok(());
    };
    let state = Arc::clone(state);
    let wrapper = lua.create_function(move |_, args: MultiValue| {
        let mode = match args.iter().nth(1) {
            Some(Value::String(s)) => s.to_str()?.to_string(),
            _ => "r".to_string(),
        };
        if mode.contains('w') || mode.contains('a') || mode.contains('+') {
            let summary = first_string_arg(&args);
            gate_decision(&state, "io.open", &summary)?;
        }
        inner.call::<MultiValue>(args)
    })?;
    io_table.set("open", wrapper)?;
    Ok(())
}

fn gate_io_output(lua: &Lua, state: &Arc<GateState>) -> mlua::Result<()> {
    let Some(io_table) = lua.globals().get::<Option<Table>>("io")? else {
        return Ok(());
    };
    let Some(inner) = io_table.get::<Option<mlua::Function>>("output")? else {
        return Ok(());
    };
    let state = Arc::clone(state);
    let wrapper = lua.create_function(move |_, args: MultiValue| {
        if !args.is_empty() {
            let summary = first_string_arg(&args);
            gate_decision(&state, "io.output", &summary)?;
        }
        inner.call::<MultiValue>(args)
    })?;
    io_table.set("output", wrapper)?;
    Ok(())
}
