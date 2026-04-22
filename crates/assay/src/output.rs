use serde::Serialize;
use std::process::ExitCode;

#[derive(Debug, Serialize)]
pub struct RunResult {
    pub passed: bool,
    pub checks: Vec<CheckResult>,
    pub duration_ms: u64,
}

#[derive(Debug, Serialize)]
pub struct CheckResult {
    pub name: String,
    pub passed: bool,
    pub duration_ms: u64,
    pub message: Option<String>,
}

impl RunResult {
    pub fn print(self) -> ExitCode {
        let json = serde_json::to_string_pretty(&self).expect("failed to serialize results");
        println!("{json}");
        if self.passed {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        }
    }
}
