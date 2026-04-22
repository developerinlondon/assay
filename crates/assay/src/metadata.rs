/// LDoc-style metadata parsed from `--- @tag value` lines at the top of a Lua module.
#[derive(Debug, Clone, Default)]
pub struct ModuleMetadata {
    /// From `@module` tag
    pub module_name: String,
    /// From `@description` tag
    pub description: String,
    /// From `@keywords` tag, split by comma and trimmed
    pub keywords: Vec<String>,
    /// From `@env` tag, split by comma and trimmed
    pub env_vars: Vec<String>,
    /// From `@quickref` tags (one per tag line)
    pub quickrefs: Vec<QuickRef>,
    /// Auto-extracted function names from `function c:method(` and `function M.method(` patterns
    pub auto_functions: Vec<String>,
}

/// A quick-reference entry parsed from `@quickref signature -> return_hint | description`.
#[derive(Debug, Clone, Default)]
pub struct QuickRef {
    /// e.g. `c:health()`
    pub signature: String,
    /// e.g. `{database, version, commit}`
    pub return_hint: String,
    /// e.g. `Check Grafana health`
    pub description: String,
}

/// Parse LDoc-style metadata from a Lua source string.
///
/// 1. Parses `--- @tag value` lines at the TOP of the file (stops at first non-`---` line).
/// 2. Auto-extracts function names from `function c:method_name(` and `function M.method_name(`
///    patterns across the entire file.
///
/// Never panics — returns a valid [`ModuleMetadata`] even on empty or malformed input.
pub fn parse_metadata(source: &str) -> ModuleMetadata {
    let mut meta = ModuleMetadata::default();

    parse_header_tags(source, &mut meta);
    extract_auto_functions(source, &mut meta);

    meta
}

/// Parse `--- @tag value` lines from the top of the file, stopping at the first non-`---` line.
fn parse_header_tags(source: &str, meta: &mut ModuleMetadata) {
    for line in source.lines() {
        let trimmed = line.trim();

        if !trimmed.starts_with("---") {
            break;
        }

        // Strip the `--- ` prefix and look for `@tag`
        let after_dashes = trimmed.trim_start_matches('-').trim();
        if let Some(rest) = after_dashes.strip_prefix('@')
            && let Some((tag, value)) = rest.split_once(char::is_whitespace)
        {
            let value = value.trim();
            match tag {
                "module" => meta.module_name = value.to_string(),
                "description" => meta.description = value.to_string(),
                "keywords" => {
                    meta.keywords = split_comma_list(value);
                }
                "env" => {
                    meta.env_vars = split_comma_list(value);
                }
                "quickref" => {
                    if let Some(qr) = parse_quickref(value) {
                        meta.quickrefs.push(qr);
                    }
                }
                _ => {} // Unknown tags silently ignored
            }
        }
    }
}

/// Split a comma-separated string into trimmed, non-empty items.
fn split_comma_list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Parse a quickref value: `signature -> return_hint | description`
fn parse_quickref(value: &str) -> Option<QuickRef> {
    // Split on ` -> ` first to get signature and the rest
    let (signature, rest) = value.split_once(" -> ")?;
    // Split the rest on ` | ` to get return_hint and description
    let (return_hint, description) = rest.split_once(" | ")?;

    Some(QuickRef {
        signature: signature.trim().to_string(),
        return_hint: return_hint.trim().to_string(),
        description: description.trim().to_string(),
    })
}

/// Scan the entire source for `function c:method_name(` and `function M.method_name(` patterns,
/// extracting the method/function name.
fn extract_auto_functions(source: &str, meta: &mut ModuleMetadata) {
    for line in source.lines() {
        let trimmed = line.trim();

        // Match `function <ident>:<name>(` or `function <ident>.<name>(`
        if let Some(rest) = trimmed.strip_prefix("function ") {
            // Find the separator (`:` or `.`) after the identifier
            if let Some(name) = extract_function_name(rest)
                && !name.is_empty()
            {
                meta.auto_functions.push(name);
            }
        }
    }
}

/// Extract function name from patterns like `c:health()` or `M.client(url, opts)`.
/// Returns the part after `:` or `.` and before `(`.
fn extract_function_name(rest: &str) -> Option<String> {
    // Find the separator position (first `:` or `.`)
    let sep_pos = rest.find([':', '.'])?;
    let after_sep = &rest[sep_pos + 1..];
    // Take everything up to `(`
    let name = after_sep.split('(').next()?;
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    Some(name.to_string())
}
