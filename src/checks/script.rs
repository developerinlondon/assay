use crate::config::CheckConfig;
use crate::lua;
use crate::output::CheckResult;
use anyhow::{Context, Result};

pub struct ScriptCheck;

impl ScriptCheck {
    pub async fn execute(&self, config: &CheckConfig, client: &reqwest::Client) -> Result<CheckResult> {
        let file_path = config
            .file
            .as_deref()
            .context("script check requires a 'file' field")?;

        // Create a fresh Lua VM for each script check (isolation)
        let vm = lua::create_vm(client.clone()).context("creating Lua VM")?;

        // Inject check-specific environment variables
        lua::inject_env(&vm, &config.env)?;

        // Execute the Lua script with async support
        match lua::async_bridge::exec_lua_file_async(&vm, file_path).await {
            Ok(()) => Ok(CheckResult {
                name: config.name.clone(),
                passed: true,
                duration_ms: 0,
                message: None,
            }),
            Err(e) => {
                // Lua script errors (including assert failures) mean the check failed
                let message = format_lua_error(&e);
                Ok(CheckResult {
                    name: config.name.clone(),
                    passed: false,
                    duration_ms: 0,
                    message: Some(message),
                })
            }
        }
    }
}

fn format_lua_error(err: &mlua::Error) -> String {
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
