//! Plain column-aligned table printer for CLI output.
//!
//! Two-pass: compute column widths across header + every row, then print
//! each row padded to those widths. No colors, no unicode borders — keeps
//! output copy-pasteable and script-consumable via `awk` / `cut`.

/// Print a header row plus `rows` aligned so every column is at least
/// the width of its longest value.
pub fn print_table(headers: &[&str], rows: &[Vec<String>]) {
    let mut widths: Vec<usize> = headers.iter().map(|h| h.len()).collect();
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            if i < widths.len() && cell.len() > widths[i] {
                widths[i] = cell.len();
            }
        }
    }
    print_row(headers.iter().map(|s| s.to_string()).collect(), &widths);
    // No separator line — the alignment alone is enough.
    for row in rows {
        print_row(row.clone(), &widths);
    }
}

fn print_row(cells: Vec<String>, widths: &[usize]) {
    let n = cells.len().min(widths.len());
    let parts: Vec<String> = (0..n)
        .map(|i| {
            if i + 1 == n {
                // Last cell: no trailing padding, easier to pipe.
                cells[i].clone()
            } else {
                format!("{:<width$}", cells[i], width = widths[i])
            }
        })
        .collect();
    println!("{}", parts.join("  "));
}

/// Common helpers for serde_json::Value formatting.
pub fn value_as_str(v: &serde_json::Value, key: &str) -> String {
    match v.get(key) {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(serde_json::Value::Null) | None => "-".to_string(),
        Some(other) => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn value_as_str_handles_missing_and_null() {
        let v = serde_json::json!({ "a": "x", "b": null });
        assert_eq!(value_as_str(&v, "a"), "x");
        assert_eq!(value_as_str(&v, "b"), "-");
        assert_eq!(value_as_str(&v, "missing"), "-");
    }

    #[test]
    fn value_as_str_renders_numbers() {
        let v = serde_json::json!({ "n": 42 });
        assert_eq!(value_as_str(&v, "n"), "42");
    }
}
