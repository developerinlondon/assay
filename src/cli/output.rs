//! Output format plumbing for CLI subcommands.
//!
//! Four formats:
//!   - `Table`  — column-aligned human-readable, default on TTY
//!   - `Json`   — single pretty-printed JSON document, default when piped
//!   - `Jsonl`  — one compact JSON doc per line (streaming-friendly)
//!   - `Yaml`   — single YAML document
//!
//! Subcommands describe their data with `RenderList` (tabular) or
//! `RenderRecord` (one object) and the `Printer` handles format
//! selection.

use std::io::IsTerminal;
use std::str::FromStr;

use serde_json::Value;

use crate::cli::table::print_table;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Output {
    Table,
    Json,
    Jsonl,
    Yaml,
}

impl Output {
    /// Default when the user hasn't specified `--output` or set
    /// `ASSAY_OUTPUT` / `output:` in the config file. `table` on a TTY,
    /// `json` when stdout is redirected (shell pipe, `> file`).
    pub fn tty_adaptive_default() -> Self {
        if std::io::stdout().is_terminal() {
            Self::Table
        } else {
            Self::Json
        }
    }

    /// Resolve a format from a user-provided string, ignoring case.
    /// Returns None if the value doesn't match any known format.
    pub fn from_user_string(s: &str) -> Option<Self> {
        Self::from_str(s).ok()
    }
}

impl FromStr for Output {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, String> {
        match s.to_ascii_lowercase().as_str() {
            "table" => Ok(Self::Table),
            "json" => Ok(Self::Json),
            "jsonl" => Ok(Self::Jsonl),
            "yaml" | "yml" => Ok(Self::Yaml),
            other => Err(format!(
                "unknown output format '{other}' (expected table, json, jsonl, yaml)"
            )),
        }
    }
}

/// Print a list-of-records in the configured format.
///
/// `headers` + `row_for` are only used for `Output::Table`; other formats
/// render the raw JSON value. Keeps table formatting isolated from JSON
/// shape so the REST response structure stays the source of truth.
pub fn print_list(
    format: Output,
    items: &[Value],
    headers: &[&str],
    row_for: impl Fn(&Value) -> Vec<String>,
) {
    match format {
        Output::Table => {
            let rows: Vec<Vec<String>> = items.iter().map(&row_for).collect();
            print_table(headers, &rows);
        }
        Output::Json => {
            let v = Value::Array(items.to_vec());
            println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
        }
        Output::Jsonl => {
            for item in items {
                println!("{}", serde_json::to_string(item).unwrap_or_default());
            }
        }
        Output::Yaml => {
            let v = Value::Array(items.to_vec());
            match serde_yml::to_string(&v) {
                Ok(s) => print!("{s}"),
                Err(e) => eprintln!("error: serialising yaml: {e}"),
            }
        }
    }
}

/// Print a single record. Table format falls back to pretty JSON —
/// records don't have natural column shape, and tabular display of a
/// single object is noisier than JSON.
pub fn print_record(format: Output, value: &Value) {
    match format {
        Output::Table | Output::Json => {
            println!(
                "{}",
                serde_json::to_string_pretty(value).unwrap_or_default()
            );
        }
        Output::Jsonl => {
            println!("{}", serde_json::to_string(value).unwrap_or_default());
        }
        Output::Yaml => match serde_yml::to_string(value) {
            Ok(s) => print!("{s}"),
            Err(e) => eprintln!("error: serialising yaml: {e}"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_output_formats() {
        assert_eq!(Output::from_str("table").unwrap(), Output::Table);
        assert_eq!(Output::from_str("JSON").unwrap(), Output::Json);
        assert_eq!(Output::from_str("jsonl").unwrap(), Output::Jsonl);
        assert_eq!(Output::from_str("yaml").unwrap(), Output::Yaml);
        assert_eq!(Output::from_str("yml").unwrap(), Output::Yaml);
        assert!(Output::from_str("junk").is_err());
    }
}
