/// Context output formatter for module metadata.
/// Renders prompt-ready Markdown text from module metadata.

#[derive(Debug, Clone)]
pub struct ModuleContextEntry {
    pub module_name: String,
    pub description: String,
    pub env_vars: Vec<String>,
    pub quickrefs: Vec<QuickRefEntry>,
}

#[derive(Debug, Clone)]
pub struct QuickRefEntry {
    pub signature: String,
    pub return_hint: String,
    pub description: String,
}

/// Format module context entries into prompt-ready Markdown.
///
/// Output includes:
/// - Module list with descriptions, env vars, and method signatures
/// - Built-in functions section (always present)
///
/// Lines are kept under 120 chars where practical.
pub fn format_context(entries: &[ModuleContextEntry]) -> String {
    let mut output = String::new();

    output.push_str("# Assay Module Context\n\n");

    if entries.is_empty() {
        output.push_str("No matching modules found.\n\n");
    } else {
        output.push_str("## Matching Modules\n\n");

        for entry in entries {
            output.push_str(&format!("### {}\n", entry.module_name));
            output.push_str(&format!("{}\n", entry.description));

            if !entry.env_vars.is_empty() {
                output.push_str(&format!("Env: {}\n", entry.env_vars.join(", ")));
            }

            if !entry.quickrefs.is_empty() {
                output.push_str("Methods:\n");
                for qr in &entry.quickrefs {
                    output.push_str(&format!(
                        "  {} -> {} | {}\n",
                        qr.signature, qr.return_hint, qr.description
                    ));
                }
            }

            output.push('\n');
        }
    }

    output.push_str("## Built-in Functions (always available, no require needed)\n");
    output.push_str("http.get(url, opts?) -> {status, body, headers}\n");
    output.push_str("json.parse(str) -> table | json.encode(tbl) -> str\n");
    output.push_str("yaml.parse(str) -> table | yaml.encode(tbl) -> str\n");
    output.push_str("toml.parse(str) -> table | toml.encode(tbl) -> str\n");
    output.push_str("base64.encode(str) -> str | base64.decode(str) -> str\n");
    output.push_str("crypto.jwt_sign(claims, key, alg) -> token\n");
    output.push_str("crypto.hash(str, alg) -> str | crypto.hmac(key, data, alg?) -> str\n");
    output.push_str("crypto.random(len) -> str\n");
    output.push_str("regex.match(pat, str) -> bool | regex.find(pat, str) -> str\n");
    output.push_str("regex.find_all(pat, str) -> [str] | regex.replace(pat, str, repl) -> str\n");
    output.push_str("fs.read(path) -> str | fs.write(path, str)\n");
    output.push_str("db.connect(url) -> conn | db.query(conn, sql, params?) -> [row]\n");
    output.push_str("db.execute(conn, sql, params?) -> count | db.close(conn)\n");
    output.push_str("ws.connect(url) -> conn | ws.send(conn, msg)\n");
    output.push_str("ws.recv(conn) -> msg | ws.close(conn)\n");
    output.push_str("template.render(path, vars) -> str\n");
    output.push_str("template.render_string(tmpl, vars) -> str\n");
    output.push_str("async.spawn(fn) -> handle | async.spawn_interval(fn, ms) -> handle\n");
    output.push_str("handle:await() | handle:cancel()\n");
    output.push_str("assert.eq(a, b, msg?) | assert.gt(a, b, msg?) | assert.lt(a, b, msg?)\n");
    output.push_str("assert.contains(str, sub, msg?) | assert.not_nil(val, msg?)\n");
    output.push_str("assert.matches(str, pat, msg?)\n");
    output.push_str("log.info(msg) | log.warn(msg) | log.error(msg)\n");
    output.push_str("env.get(key) -> str | sleep(secs) | time() -> int\n");

    output
}
