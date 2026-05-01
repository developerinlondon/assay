//! CLI subcommand implementations — one async fn per subcommand.
//!
//! Each fn returns `ExitCode`:
//!   - 0 (SUCCESS) on the happy path
//!   - 1 on HTTP error, not-found, unreachable engine
//!   - 2 on `workflow wait` timeout before the target status is reached
//!   - 64 on usage errors (bad JSON input) — matches sysexits EX_USAGE
//!
//! Error messages go to stderr; results go to stdout. Keeps scripts
//! composable: `assay workflow list > wfs.json 2> errors.log`.

use std::io::IsTerminal;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::cli::client::EngineClient;
use crate::cli::input::resolve_json;
use crate::cli::output::{print_list, print_record};
use crate::cli::table::value_as_str;
use crate::cli::GlobalOpts;

fn eprint_err(e: anyhow::Error) -> ExitCode {
    eprintln!("error: {e:#}");
    ExitCode::from(1)
}

fn usage_error(msg: impl std::fmt::Display) -> ExitCode {
    eprintln!("error: {msg}");
    ExitCode::from(64)
}

// ── Workflows ──────────────────────────────────────────────

pub async fn workflow_start(
    opts: &GlobalOpts,
    workflow_type: &str,
    id: Option<String>,
    input: Option<String>,
    queue: Option<String>,
    search_attrs: Option<String>,
) -> ExitCode {
    let client = EngineClient::new(opts);
    let resolved_input = match input.as_deref() {
        None => None,
        Some(raw) => match resolve_json(raw, "--input") {
            Ok(v) => Some(v),
            Err(e) => return usage_error(e),
        },
    };
    let resolved_attrs = match search_attrs.as_deref() {
        None => None,
        Some(raw) => match resolve_json(raw, "--search-attrs") {
            Ok(v) => Some(v),
            Err(e) => return usage_error(e),
        },
    };
    let generated_id;
    let workflow_id = match id.as_deref() {
        Some(v) => v,
        None => {
            generated_id = format!(
                "wf-{}-{}",
                workflow_type.to_ascii_lowercase(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis())
                    .unwrap_or(0)
            );
            &generated_id
        }
    };
    match client
        .workflow_start(
            workflow_type,
            workflow_id,
            resolved_input.as_ref(),
            queue.as_deref(),
            resolved_attrs.as_ref(),
        )
        .await
    {
        Ok(v) => {
            print_record(opts.output, &v);
            ExitCode::SUCCESS
        }
        Err(e) => eprint_err(e),
    }
}

pub async fn workflow_list(
    opts: &GlobalOpts,
    status: Option<String>,
    workflow_type: Option<String>,
    search_attrs: Option<String>,
    limit: i64,
) -> ExitCode {
    let client = EngineClient::new(opts);
    let resolved_attrs = match search_attrs.as_deref() {
        None => None,
        Some(raw) => match resolve_json(raw, "--search-attrs") {
            Ok(v) => Some(v),
            Err(e) => return usage_error(e),
        },
    };
    let result = client
        .workflow_list(
            status.as_deref(),
            workflow_type.as_deref(),
            resolved_attrs.as_ref(),
            Some(limit),
        )
        .await;
    let workflows = match result {
        Ok(Value::Array(v)) => v,
        Ok(_) => {
            eprintln!("error: unexpected response shape (expected array)");
            return ExitCode::from(1);
        }
        Err(e) => return eprint_err(e),
    };

    print_list(
        opts.output,
        &workflows,
        &["ID", "TYPE", "STATUS", "QUEUE"],
        |w| {
            vec![
                value_as_str(w, "id"),
                value_as_str(w, "workflow_type"),
                value_as_str(w, "status"),
                value_as_str(w, "task_queue"),
            ]
        },
    );
    ExitCode::SUCCESS
}

pub async fn workflow_describe(opts: &GlobalOpts, id: &str) -> ExitCode {
    let client = EngineClient::new(opts);
    match client.workflow_describe(id).await {
        Ok(v) => {
            print_record(opts.output, &v);
            ExitCode::SUCCESS
        }
        Err(e) => eprint_err(e),
    }
}

pub async fn workflow_state(opts: &GlobalOpts, id: &str, name: Option<&str>) -> ExitCode {
    let client = EngineClient::new(opts);
    match client.workflow_state(id, name).await {
        Ok(v) => {
            print_record(opts.output, &v);
            ExitCode::SUCCESS
        }
        Err(e) => eprint_err(e),
    }
}

pub async fn workflow_events(opts: &GlobalOpts, id: &str, follow: bool) -> ExitCode {
    let client = EngineClient::new(opts);
    if follow {
        return workflow_events_follow(opts, &client, id).await;
    }
    let result = client.workflow_events(id).await;
    let events = match result {
        Ok(Value::Array(v)) => v,
        Ok(_) => {
            eprintln!("error: unexpected response shape (expected array)");
            return ExitCode::from(1);
        }
        Err(e) => return eprint_err(e),
    };
    print_list(
        opts.output,
        &events,
        &["SEQ", "TYPE", "TIMESTAMP"],
        |e| {
            vec![
                value_as_str(e, "seq"),
                value_as_str(e, "event_type"),
                value_as_str(e, "timestamp"),
            ]
        },
    );
    ExitCode::SUCCESS
}

/// Poll the event log every 500ms, printing new events as they arrive.
/// Simpler + more predictable than a raw SSE subscription, and it
/// works against any engine that implements `GET /workflows/{id}/events`
/// — no extra server code, no reconnection dance. Stops when a terminal
/// workflow event is seen.
async fn workflow_events_follow(
    opts: &GlobalOpts,
    client: &EngineClient,
    id: &str,
) -> ExitCode {
    let mut last_seq: i64 = -1;
    loop {
        let events = match client.workflow_events(id).await {
            Ok(Value::Array(v)) => v,
            Ok(_) => {
                eprintln!("error: unexpected response shape (expected array)");
                return ExitCode::from(1);
            }
            Err(e) => return eprint_err(e),
        };
        let mut terminal = false;
        for ev in &events {
            let seq = ev.get("seq").and_then(|v| v.as_i64()).unwrap_or(-1);
            if seq <= last_seq {
                continue;
            }
            last_seq = seq;
            match opts.output {
                crate::cli::Output::Table => println!(
                    "{:>4}  {:<30}  {}",
                    value_as_str(ev, "seq"),
                    value_as_str(ev, "event_type"),
                    value_as_str(ev, "timestamp"),
                ),
                _ => println!("{}", serde_json::to_string(ev).unwrap_or_default()),
            }
            let event_type = ev.get("event_type").and_then(|v| v.as_str()).unwrap_or("");
            if matches!(
                event_type,
                "WorkflowCompleted" | "WorkflowFailed" | "WorkflowCancelled"
            ) {
                terminal = true;
            }
        }
        if terminal {
            return ExitCode::SUCCESS;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

pub async fn workflow_children(opts: &GlobalOpts, id: &str) -> ExitCode {
    let client = EngineClient::new(opts);
    let result = client.workflow_children(id).await;
    let children = match result {
        Ok(Value::Array(v)) => v,
        Ok(_) => {
            eprintln!("error: unexpected response shape (expected array)");
            return ExitCode::from(1);
        }
        Err(e) => return eprint_err(e),
    };
    print_list(
        opts.output,
        &children,
        &["ID", "TYPE", "STATUS"],
        |c| {
            vec![
                value_as_str(c, "id"),
                value_as_str(c, "workflow_type"),
                value_as_str(c, "status"),
            ]
        },
    );
    ExitCode::SUCCESS
}

pub async fn workflow_signal(
    opts: &GlobalOpts,
    id: &str,
    name: &str,
    payload: Option<String>,
) -> ExitCode {
    let client = EngineClient::new(opts);
    let parsed = match payload.as_deref() {
        None => None,
        Some(raw) => match resolve_json(raw, "signal payload") {
            Ok(v) => Some(v),
            Err(e) => return usage_error(e),
        },
    };
    match client.workflow_signal(id, name, parsed.as_ref()).await {
        Ok(()) => {
            print_action_result(opts, &format!("signal '{name}' sent to {id}"));
            ExitCode::SUCCESS
        }
        Err(e) => eprint_err(e),
    }
}

pub async fn workflow_cancel(opts: &GlobalOpts, id: &str) -> ExitCode {
    let client = EngineClient::new(opts);
    match client.workflow_cancel(id).await {
        Ok(()) => {
            print_action_result(opts, &format!("cancel requested for {id}"));
            ExitCode::SUCCESS
        }
        Err(e) => eprint_err(e),
    }
}

pub async fn workflow_terminate(
    opts: &GlobalOpts,
    id: &str,
    reason: Option<String>,
) -> ExitCode {
    let client = EngineClient::new(opts);
    match client.workflow_terminate(id, reason.as_deref()).await {
        Ok(()) => {
            print_action_result(opts, &format!("{id} terminated"));
            ExitCode::SUCCESS
        }
        Err(e) => eprint_err(e),
    }
}

pub async fn workflow_continue_as_new(
    opts: &GlobalOpts,
    id: &str,
    input: Option<String>,
) -> ExitCode {
    let client = EngineClient::new(opts);
    let resolved_input = match input.as_deref() {
        None => None,
        Some(raw) => match resolve_json(raw, "--input") {
            Ok(v) => Some(v),
            Err(e) => return usage_error(e),
        },
    };
    match client
        .workflow_continue_as_new(id, resolved_input.as_ref())
        .await
    {
        Ok(v) => {
            print_record(opts.output, &v);
            ExitCode::SUCCESS
        }
        Err(e) => eprint_err(e),
    }
}

pub async fn workflow_wait(
    opts: &GlobalOpts,
    id: &str,
    timeout_secs: u64,
    target: Option<String>,
) -> ExitCode {
    let client = EngineClient::new(opts);
    let deadline = Instant::now() + Duration::from_secs(timeout_secs);
    let target = target.as_deref();
    loop {
        let v = match client.workflow_describe(id).await {
            Ok(v) => v,
            Err(e) => return eprint_err(e),
        };
        let status = v
            .get("status")
            .and_then(|s| s.as_str())
            .unwrap_or("")
            .to_string();

        let matches_target = match target {
            Some(t) => status.eq_ignore_ascii_case(t),
            None => matches!(
                status.as_str(),
                "COMPLETED" | "FAILED" | "CANCELLED" | "TIMED_OUT"
            ),
        };

        if matches_target {
            // Human formats get a final describe; JSON-family prints the record.
            match opts.output {
                crate::cli::Output::Table => {
                    if std::io::stdout().is_terminal() {
                        println!("{id} {status}");
                    }
                }
                _ => print_record(opts.output, &v),
            }
            return match status.as_str() {
                "COMPLETED" => ExitCode::SUCCESS,
                _ if target.is_some() => ExitCode::SUCCESS,
                _ => ExitCode::from(1),
            };
        }

        if Instant::now() >= deadline {
            eprintln!(
                "error: timeout after {timeout_secs}s waiting for {id} (last status: {status})"
            );
            return ExitCode::from(2);
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

// ── Schedules ──────────────────────────────────────────────

pub async fn schedule_list(opts: &GlobalOpts) -> ExitCode {
    let client = EngineClient::new(opts);
    let result = client.schedule_list().await;
    let schedules = match result {
        Ok(Value::Array(v)) => v,
        Ok(_) => {
            eprintln!("error: unexpected response shape (expected array)");
            return ExitCode::from(1);
        }
        Err(e) => return eprint_err(e),
    };

    print_list(
        opts.output,
        &schedules,
        &["NAME", "TYPE", "CRON", "TZ", "PAUSED"],
        |s| {
            vec![
                value_as_str(s, "name"),
                value_as_str(s, "workflow_type"),
                value_as_str(s, "cron_expr"),
                value_as_str(s, "timezone"),
                value_as_str(s, "paused"),
            ]
        },
    );
    ExitCode::SUCCESS
}

pub async fn schedule_describe(opts: &GlobalOpts, name: &str) -> ExitCode {
    let client = EngineClient::new(opts);
    match client.schedule_describe(name).await {
        Ok(v) => {
            print_record(opts.output, &v);
            ExitCode::SUCCESS
        }
        Err(e) => eprint_err(e),
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn schedule_create(
    opts: &GlobalOpts,
    name: &str,
    workflow_type: &str,
    cron: &str,
    timezone: Option<String>,
    input: Option<String>,
    queue: Option<String>,
) -> ExitCode {
    let client = EngineClient::new(opts);
    let parsed_input = match input.as_deref() {
        None => None,
        Some(raw) => match resolve_json(raw, "--input") {
            Ok(v) => Some(v),
            Err(e) => return usage_error(e),
        },
    };
    match client
        .schedule_create(
            name,
            workflow_type,
            cron,
            timezone.as_deref(),
            parsed_input.as_ref(),
            queue.as_deref(),
        )
        .await
    {
        Ok(v) => {
            print_record(opts.output, &v);
            ExitCode::SUCCESS
        }
        Err(e) => eprint_err(e),
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn schedule_patch(
    opts: &GlobalOpts,
    name: &str,
    cron: Option<String>,
    timezone: Option<String>,
    input: Option<String>,
    queue: Option<String>,
    overlap: Option<String>,
) -> ExitCode {
    if cron.is_none()
        && timezone.is_none()
        && input.is_none()
        && queue.is_none()
        && overlap.is_none()
    {
        return usage_error(
            "schedule patch: at least one of \
             --cron/--timezone/--input/--queue/--overlap is required",
        );
    }
    let client = EngineClient::new(opts);
    let parsed_input = match input.as_deref() {
        None => None,
        Some(raw) => match resolve_json(raw, "--input") {
            Ok(v) => Some(v),
            Err(e) => return usage_error(e),
        },
    };
    match client
        .schedule_patch(
            name,
            cron.as_deref(),
            timezone.as_deref(),
            parsed_input.as_ref(),
            queue.as_deref(),
            overlap.as_deref(),
        )
        .await
    {
        Ok(v) => {
            print_record(opts.output, &v);
            ExitCode::SUCCESS
        }
        Err(e) => eprint_err(e),
    }
}

pub async fn schedule_pause(opts: &GlobalOpts, name: &str) -> ExitCode {
    let client = EngineClient::new(opts);
    match client.schedule_pause(name).await {
        Ok(_) => {
            print_action_result(opts, &format!("{name} paused"));
            ExitCode::SUCCESS
        }
        Err(e) => eprint_err(e),
    }
}

pub async fn schedule_resume(opts: &GlobalOpts, name: &str) -> ExitCode {
    let client = EngineClient::new(opts);
    match client.schedule_resume(name).await {
        Ok(_) => {
            print_action_result(opts, &format!("{name} resumed"));
            ExitCode::SUCCESS
        }
        Err(e) => eprint_err(e),
    }
}

pub async fn schedule_delete(opts: &GlobalOpts, name: &str) -> ExitCode {
    let client = EngineClient::new(opts);
    match client.schedule_delete(name).await {
        Ok(()) => {
            print_action_result(opts, &format!("{name} deleted"));
            ExitCode::SUCCESS
        }
        Err(e) => eprint_err(e),
    }
}

// ── Namespaces ─────────────────────────────────────────────

pub async fn namespace_create(opts: &GlobalOpts, name: &str) -> ExitCode {
    let client = EngineClient::new(opts);
    match client.namespace_create(name).await {
        Ok(()) => {
            print_action_result(opts, &format!("namespace '{name}' created"));
            ExitCode::SUCCESS
        }
        Err(e) => eprint_err(e),
    }
}

pub async fn namespace_list(opts: &GlobalOpts) -> ExitCode {
    let client = EngineClient::new(opts);
    let result = client.namespace_list().await;
    let items = match result {
        Ok(Value::Array(v)) => v,
        Ok(_) => {
            eprintln!("error: unexpected response shape (expected array)");
            return ExitCode::from(1);
        }
        Err(e) => return eprint_err(e),
    };
    print_list(opts.output, &items, &["NAME", "CREATED"], |n| {
        vec![value_as_str(n, "name"), value_as_str(n, "created_at")]
    });
    ExitCode::SUCCESS
}

pub async fn namespace_describe(opts: &GlobalOpts, name: &str) -> ExitCode {
    let client = EngineClient::new(opts);
    match client.namespace_stats(name).await {
        Ok(v) => {
            print_record(opts.output, &v);
            ExitCode::SUCCESS
        }
        Err(e) => eprint_err(e),
    }
}

pub async fn namespace_delete(opts: &GlobalOpts, name: &str) -> ExitCode {
    let client = EngineClient::new(opts);
    match client.namespace_delete(name).await {
        Ok(()) => {
            print_action_result(opts, &format!("namespace '{name}' deleted"));
            ExitCode::SUCCESS
        }
        Err(e) => eprint_err(e),
    }
}

// ── Workers ────────────────────────────────────────────────

pub async fn worker_list(opts: &GlobalOpts) -> ExitCode {
    let client = EngineClient::new(opts);
    let result = client.worker_list().await;
    let workers = match result {
        Ok(Value::Array(v)) => v,
        Ok(_) => {
            eprintln!("error: unexpected response shape (expected array)");
            return ExitCode::from(1);
        }
        Err(e) => return eprint_err(e),
    };
    print_list(
        opts.output,
        &workers,
        &["ID", "IDENTITY", "QUEUE", "ACTIVE", "LAST HEARTBEAT"],
        |w| {
            vec![
                value_as_str(w, "id"),
                value_as_str(w, "identity"),
                value_as_str(w, "task_queue"),
                value_as_str(w, "active_tasks"),
                value_as_str(w, "last_heartbeat"),
            ]
        },
    );
    ExitCode::SUCCESS
}

// ── Queues ─────────────────────────────────────────────────

pub async fn queue_stats(opts: &GlobalOpts) -> ExitCode {
    let client = EngineClient::new(opts);
    let result = client.queue_stats().await;
    let queues = match result {
        Ok(Value::Array(v)) => v,
        Ok(_) => {
            eprintln!("error: unexpected response shape (expected array)");
            return ExitCode::from(1);
        }
        Err(e) => return eprint_err(e),
    };
    print_list(
        opts.output,
        &queues,
        &["QUEUE", "PENDING", "RUNNING", "WORKERS"],
        |q| {
            vec![
                value_as_str(q, "queue"),
                value_as_str(q, "pending_activities"),
                value_as_str(q, "running_activities"),
                value_as_str(q, "workers"),
            ]
        },
    );
    ExitCode::SUCCESS
}

// ── Helpers ────────────────────────────────────────────────

/// For side-effect subcommands (signal, cancel, pause, …): print a
/// human confirmation on table output; print `{"ok":true,"message":…}`
/// on JSON-family outputs so scripts can parse the result.
fn print_action_result(opts: &GlobalOpts, message: &str) {
    match opts.output {
        crate::cli::Output::Table => println!("{message}"),
        _ => {
            let v = serde_json::json!({ "ok": true, "message": message });
            print_record(opts.output, &v);
        }
    }
}
