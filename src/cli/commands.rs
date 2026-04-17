//! CLI subcommand implementations — one async fn per subcommand.
//!
//! Each fn returns `ExitCode`:
//!   - SUCCESS (0) on the happy path
//!   - 1 on HTTP error, not-found, unreachable engine, or bad JSON input
//!
//! Error messages go to stderr; results go to stdout. Keeps scripts
//! composable — `assay workflow list > workflows.txt 2> errors.log`.

use std::process::ExitCode;

use serde_json::Value;

use crate::cli::client::EngineClient;
use crate::cli::table::{print_table, value_as_str};
use crate::cli::GlobalOpts;

fn eprint_err(e: anyhow::Error) -> ExitCode {
    eprintln!("error: {e:#}");
    ExitCode::from(1)
}

fn parse_json_input(raw: &str, what: &str) -> Result<Value, String> {
    serde_json::from_str(raw).map_err(|e| format!("{what}: invalid JSON: {e}"))
}

// ── Workflows ──────────────────────────────────────────────

pub async fn workflow_list(
    opts: &GlobalOpts,
    status: Option<String>,
    workflow_type: Option<String>,
    limit: i64,
) -> ExitCode {
    let client = EngineClient::new(opts);
    let result = client
        .workflow_list(status.as_deref(), workflow_type.as_deref(), Some(limit))
        .await;
    let Ok(Value::Array(workflows)) = result.as_ref().map(|v| v.clone()) else {
        return match result {
            Ok(_) => {
                eprintln!("error: unexpected response shape (expected array)");
                ExitCode::from(1)
            }
            Err(e) => eprint_err(e),
        };
    };

    let rows: Vec<Vec<String>> = workflows
        .iter()
        .map(|w| {
            vec![
                value_as_str(w, "id"),
                value_as_str(w, "workflow_type"),
                value_as_str(w, "status"),
                value_as_str(w, "task_queue"),
            ]
        })
        .collect();
    print_table(&["ID", "TYPE", "STATUS", "QUEUE"], &rows);
    ExitCode::SUCCESS
}

pub async fn workflow_describe(opts: &GlobalOpts, id: &str) -> ExitCode {
    let client = EngineClient::new(opts);
    match client.workflow_describe(id).await {
        Ok(v) => {
            println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
            ExitCode::SUCCESS
        }
        Err(e) => eprint_err(e),
    }
}

pub async fn workflow_state(opts: &GlobalOpts, id: &str, name: Option<&str>) -> ExitCode {
    let client = EngineClient::new(opts);
    match client.workflow_state(id, name).await {
        Ok(v) => {
            println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
            ExitCode::SUCCESS
        }
        Err(e) => eprint_err(e),
    }
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
        Some(raw) => match parse_json_input(raw, "signal payload") {
            Ok(v) => Some(v),
            Err(e) => {
                eprintln!("error: {e}");
                return ExitCode::from(1);
            }
        },
    };
    match client.workflow_signal(id, name, parsed.as_ref()).await {
        Ok(()) => {
            println!("signal '{name}' sent to {id}");
            ExitCode::SUCCESS
        }
        Err(e) => eprint_err(e),
    }
}

pub async fn workflow_cancel(opts: &GlobalOpts, id: &str) -> ExitCode {
    let client = EngineClient::new(opts);
    match client.workflow_cancel(id).await {
        Ok(()) => {
            println!("cancel requested for {id}");
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
            println!("{id} terminated");
            ExitCode::SUCCESS
        }
        Err(e) => eprint_err(e),
    }
}

// ── Schedules ──────────────────────────────────────────────

pub async fn schedule_list(opts: &GlobalOpts) -> ExitCode {
    let client = EngineClient::new(opts);
    let result = client.schedule_list().await;
    let Ok(Value::Array(schedules)) = result.as_ref().map(|v| v.clone()) else {
        return match result {
            Ok(_) => {
                eprintln!("error: unexpected response shape (expected array)");
                ExitCode::from(1)
            }
            Err(e) => eprint_err(e),
        };
    };

    let rows: Vec<Vec<String>> = schedules
        .iter()
        .map(|s| {
            vec![
                value_as_str(s, "name"),
                value_as_str(s, "workflow_type"),
                value_as_str(s, "cron_expr"),
                value_as_str(s, "timezone"),
                value_as_str(s, "paused"),
            ]
        })
        .collect();
    print_table(&["NAME", "TYPE", "CRON", "TZ", "PAUSED"], &rows);
    ExitCode::SUCCESS
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
        Some(raw) => match parse_json_input(raw, "--input") {
            Ok(v) => Some(v),
            Err(e) => {
                eprintln!("error: {e}");
                return ExitCode::from(1);
            }
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
            println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
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
        eprintln!(
            "error: schedule patch: at least one of \
             --cron/--timezone/--input/--queue/--overlap is required"
        );
        return ExitCode::from(1);
    }
    let client = EngineClient::new(opts);
    let parsed_input = match input.as_deref() {
        None => None,
        Some(raw) => match parse_json_input(raw, "--input") {
            Ok(v) => Some(v),
            Err(e) => {
                eprintln!("error: {e}");
                return ExitCode::from(1);
            }
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
            println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
            ExitCode::SUCCESS
        }
        Err(e) => eprint_err(e),
    }
}

pub async fn schedule_pause(opts: &GlobalOpts, name: &str) -> ExitCode {
    let client = EngineClient::new(opts);
    match client.schedule_pause(name).await {
        Ok(_) => {
            println!("{name} paused");
            ExitCode::SUCCESS
        }
        Err(e) => eprint_err(e),
    }
}

pub async fn schedule_resume(opts: &GlobalOpts, name: &str) -> ExitCode {
    let client = EngineClient::new(opts);
    match client.schedule_resume(name).await {
        Ok(_) => {
            println!("{name} resumed");
            ExitCode::SUCCESS
        }
        Err(e) => eprint_err(e),
    }
}

pub async fn schedule_delete(opts: &GlobalOpts, name: &str) -> ExitCode {
    let client = EngineClient::new(opts);
    match client.schedule_delete(name).await {
        Ok(()) => {
            println!("{name} deleted");
            ExitCode::SUCCESS
        }
        Err(e) => eprint_err(e),
    }
}
