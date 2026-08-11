//! Dispatch for the workflow-engine subcommands.
//!
//! Each family resolves the shared global options once, then routes to the
//! matching `cli::commands` client call.

use std::process::ExitCode;

use crate::cli;
use crate::cli::args::{
    CliEngineOpts, NamespaceCommands, QueueCommands, ScheduleCommands, WorkerCommands,
    WorkflowCommands,
};

pub(crate) async fn workflow(global: CliEngineOpts, command: WorkflowCommands) -> ExitCode {
    let opts = match cli::GlobalOpts::resolve(global.as_flags()) {
        Ok(o) => o,
        Err(code) => return code,
    };
    match command {
        WorkflowCommands::Start {
            workflow_type,
            id,
            input,
            queue,
            search_attrs,
        } => {
            cli::commands::workflow_start(&opts, &workflow_type, id, input, queue, search_attrs)
                .await
        }
        WorkflowCommands::List {
            status,
            workflow_type,
            search_attrs,
            limit,
        } => cli::commands::workflow_list(&opts, status, workflow_type, search_attrs, limit).await,
        WorkflowCommands::Describe { id } => cli::commands::workflow_describe(&opts, &id).await,
        WorkflowCommands::State { id, name } => {
            cli::commands::workflow_state(&opts, &id, name.as_deref()).await
        }
        WorkflowCommands::Events { id, follow } => {
            cli::commands::workflow_events(&opts, &id, follow).await
        }
        WorkflowCommands::Children { id } => cli::commands::workflow_children(&opts, &id).await,
        WorkflowCommands::Signal { id, name, payload } => {
            cli::commands::workflow_signal(&opts, &id, &name, payload).await
        }
        WorkflowCommands::Cancel { id } => cli::commands::workflow_cancel(&opts, &id).await,
        WorkflowCommands::Terminate { id, reason } => {
            cli::commands::workflow_terminate(&opts, &id, reason).await
        }
        WorkflowCommands::Retry {
            id,
            requested_by,
            reason,
        } => cli::commands::workflow_retry(&opts, &id, &requested_by, &reason).await,
        WorkflowCommands::ContinueAsNew { id, input } => {
            cli::commands::workflow_continue_as_new(&opts, &id, input).await
        }
        WorkflowCommands::Wait {
            id,
            timeout,
            target,
        } => cli::commands::workflow_wait(&opts, &id, timeout, target).await,
    }
}

pub(crate) async fn schedule(global: CliEngineOpts, command: ScheduleCommands) -> ExitCode {
    let opts = match cli::GlobalOpts::resolve(global.as_flags()) {
        Ok(o) => o,
        Err(code) => return code,
    };
    match command {
        ScheduleCommands::List => cli::commands::schedule_list(&opts).await,
        ScheduleCommands::Describe { name } => cli::commands::schedule_describe(&opts, &name).await,
        ScheduleCommands::Create {
            name,
            workflow_type,
            cron,
            timezone,
            input,
            queue,
        } => {
            cli::commands::schedule_create(
                &opts,
                &name,
                &workflow_type,
                &cron,
                timezone,
                input,
                Some(queue),
            )
            .await
        }
        ScheduleCommands::Patch {
            name,
            cron,
            timezone,
            input,
            queue,
            overlap,
        } => {
            cli::commands::schedule_patch(&opts, &name, cron, timezone, input, queue, overlap).await
        }
        ScheduleCommands::Pause { name } => cli::commands::schedule_pause(&opts, &name).await,
        ScheduleCommands::Resume { name } => cli::commands::schedule_resume(&opts, &name).await,
        ScheduleCommands::Delete { name } => cli::commands::schedule_delete(&opts, &name).await,
    }
}

pub(crate) async fn namespace(global: CliEngineOpts, command: NamespaceCommands) -> ExitCode {
    let opts = match cli::GlobalOpts::resolve(global.as_flags()) {
        Ok(o) => o,
        Err(code) => return code,
    };
    match command {
        NamespaceCommands::Create { name } => cli::commands::namespace_create(&opts, &name).await,
        NamespaceCommands::List => cli::commands::namespace_list(&opts).await,
        NamespaceCommands::Describe { name } => {
            cli::commands::namespace_describe(&opts, &name).await
        }
        NamespaceCommands::Delete { name } => cli::commands::namespace_delete(&opts, &name).await,
    }
}

pub(crate) async fn worker(global: CliEngineOpts, command: WorkerCommands) -> ExitCode {
    let opts = match cli::GlobalOpts::resolve(global.as_flags()) {
        Ok(o) => o,
        Err(code) => return code,
    };
    match command {
        WorkerCommands::List => cli::commands::worker_list(&opts).await,
    }
}

pub(crate) async fn queue(global: CliEngineOpts, command: QueueCommands) -> ExitCode {
    let opts = match cli::GlobalOpts::resolve(global.as_flags()) {
        Ok(o) => o,
        Err(code) => return code,
    };
    match command {
        QueueCommands::Stats => cli::commands::queue_stats(&opts).await,
    }
}
