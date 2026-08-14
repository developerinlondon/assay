// Exercises `assay mcp-serve`: spawn the binary, speak newline-delimited
// JSON-RPC 2.0 over its stdio, and assert the MCP handshake, tool listing,
// and both tools (assay_run + assay_context) behave as specified.

use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

struct McpServer {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: i64,
}

impl McpServer {
    fn start() -> Self {
        Self::start_with(&[])
    }

    // A server that opted in to the third mode (ASSAY_MCP_UNRESTRICTED=1).
    fn start_unrestricted() -> Self {
        Self::start_with(&[("ASSAY_MCP_UNRESTRICTED", "1")])
    }

    fn start_with(envs: &[(&str, &str)]) -> Self {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_assay"));
        cmd.arg("mcp-serve")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            // Per-call modes drive the gate; never inherit a process-wide one,
            // and start from a clean unrestricted-gate state each time.
            .env_remove("ASSAY_READONLY")
            .env_remove("ASSAY_APPROVAL")
            .env_remove("ASSAY_MODE")
            .env_remove("ASSAY_MCP_UNRESTRICTED");
        for (k, v) in envs {
            cmd.env(k, v);
        }
        let mut child = cmd.spawn().unwrap();
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        McpServer {
            child,
            stdin,
            stdout,
            next_id: 0,
        }
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        let msg = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        writeln!(self.stdin, "{}", serde_json::to_string(&msg).unwrap()).unwrap();
        self.stdin.flush().unwrap();

        let mut line = String::new();
        let n = self.stdout.read_line(&mut line).unwrap();
        assert!(n > 0, "server closed stdout without answering {method}");
        let resp: Value = serde_json::from_str(line.trim())
            .unwrap_or_else(|e| panic!("invalid JSON-RPC response to {method}: {e}: {line}"));
        assert_eq!(resp["jsonrpc"], "2.0", "response: {resp}");
        assert_eq!(resp["id"], json!(id), "response id mismatch: {resp}");
        resp
    }

    fn initialize(&mut self) -> Value {
        self.request(
            "initialize",
            json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": { "name": "test-client", "version": "0.0.0" },
            }),
        )
    }

    fn call_tool(&mut self, name: &str, arguments: Value) -> Value {
        self.request(
            "tools/call",
            json!({ "name": name, "arguments": arguments }),
        )
    }
}

impl Drop for McpServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Pull the single text content block out of an MCP tool result.
fn text_content(result: &Value) -> &str {
    result["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("tool result missing text content: {result}"))
}

#[test]
fn initialize_handshake() {
    let mut server = McpServer::start();
    let resp = server.initialize();
    let result = &resp["result"];

    assert_eq!(result["serverInfo"]["name"], "assay");
    let version = result["serverInfo"]["version"].as_str().unwrap();
    assert!(!version.is_empty(), "serverInfo.version should be set");
    assert_eq!(result["protocolVersion"], "2024-11-05");
    assert!(
        result["capabilities"]["tools"].is_object(),
        "capabilities.tools should advertise tool support: {result}"
    );
}

#[test]
fn tools_list_advertises_both_tools_with_schemas() {
    let mut server = McpServer::start();
    server.initialize();

    let resp = server.request("tools/list", json!({}));
    let tools = resp["result"]["tools"].as_array().unwrap();

    let names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();
    assert!(names.contains(&"assay_run"), "missing assay_run: {names:?}");
    assert!(
        names.contains(&"assay_context"),
        "missing assay_context: {names:?}"
    );

    let run = tools.iter().find(|t| t["name"] == "assay_run").unwrap();
    let schema = &run["inputSchema"];
    assert_eq!(schema["type"], "object");
    assert!(schema["properties"]["script"].is_object());
    let modes = schema["properties"]["mode"]["enum"].as_array().unwrap();
    let modes: Vec<&str> = modes.iter().filter_map(Value::as_str).collect();
    assert!(modes.contains(&"readonly"), "modes: {modes:?}");
    assert!(modes.contains(&"approval"), "modes: {modes:?}");
    assert!(
        !modes.contains(&"unrestricted"),
        "unrestricted must never be offered: {modes:?}"
    );

    let ctx = tools.iter().find(|t| t["name"] == "assay_context").unwrap();
    assert!(ctx["inputSchema"]["properties"]["query"].is_object());
    assert_eq!(ctx["inputSchema"]["required"][0], "query");
}

#[test]
fn assay_run_readonly_ok_envelope() {
    let mut server = McpServer::start();
    server.initialize();

    let resp = server.call_tool(
        "assay_run",
        json!({ "script": "return 1 + 1", "mode": "readonly" }),
    );
    let result = &resp["result"];
    assert_eq!(result["isError"], json!(false), "result: {result}");

    let envelope: Value = serde_json::from_str(text_content(result)).unwrap();
    assert_eq!(envelope["ok"], json!(true), "envelope: {envelope}");
    assert_eq!(envelope["status"], "ok");
    assert_eq!(envelope["output"], json!(2));
    assert_eq!(envelope["readonly"], json!(true));
}

#[test]
fn assay_run_rejects_unrestricted_mode() {
    let mut server = McpServer::start();
    server.initialize();

    let resp = server.call_tool(
        "assay_run",
        json!({ "script": "return 1", "mode": "unrestricted" }),
    );
    let result = &resp["result"];
    assert_eq!(
        result["isError"],
        json!(true),
        "unrestricted mode must be rejected: {result}"
    );
    assert!(
        text_content(result).contains("unrestricted"),
        "rejection should explain the gate: {result}"
    );
}

#[test]
fn tools_list_offers_unrestricted_when_enabled() {
    let mut server = McpServer::start_unrestricted();
    server.initialize();

    let resp = server.request("tools/list", json!({}));
    let tools = resp["result"]["tools"].as_array().unwrap();
    let run = tools.iter().find(|t| t["name"] == "assay_run").unwrap();
    let modes = run["inputSchema"]["properties"]["mode"]["enum"]
        .as_array()
        .unwrap();
    let modes: Vec<&str> = modes.iter().filter_map(Value::as_str).collect();
    assert!(
        modes.contains(&"unrestricted"),
        "with ASSAY_MCP_UNRESTRICTED=1 the ladder should offer unrestricted: {modes:?}"
    );
    // The opt-in never drops the safer modes.
    assert!(
        modes.contains(&"readonly") && modes.contains(&"approval"),
        "modes: {modes:?}"
    );
}

#[test]
fn assay_run_unrestricted_runs_write_op_when_enabled() {
    let mut server = McpServer::start_unrestricted();
    server.initialize();

    let path = std::env::temp_dir().join(format!("assay-mcp-unrestricted-{}", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let path_lua = path.to_str().unwrap().replace('\\', "\\\\");
    let script = format!(r#"fs.write("{path_lua}", "x"); return "wrote""#);

    let resp = server.call_tool(
        "assay_run",
        json!({ "script": script, "mode": "unrestricted" }),
    );
    let result = &resp["result"];
    assert_eq!(
        result["isError"],
        json!(false),
        "unrestricted mode should run the write that readonly blocks: {result}"
    );
    let envelope: Value = serde_json::from_str(text_content(result)).unwrap();
    assert_eq!(envelope["status"], "ok", "envelope: {envelope}");
    assert!(
        path.exists(),
        "the fs.write should have reached disk in unrestricted mode"
    );
    let _ = std::fs::remove_file(&path);
}

#[test]
fn assay_run_readonly_blocks_write_op() {
    let mut server = McpServer::start();
    server.initialize();

    let resp = server.call_tool(
        "assay_run",
        json!({
            "script": r#"fs.write("/tmp/assay-mcp-readonly-probe", "x")"#,
            "mode": "readonly",
        }),
    );
    let result = &resp["result"];
    assert_eq!(result["isError"], json!(true), "result: {result}");

    let envelope: Value = serde_json::from_str(text_content(result)).unwrap();
    assert_eq!(envelope["ok"], json!(false), "envelope: {envelope}");
    assert_eq!(envelope["status"], "error");
    let error = envelope["error"].as_str().unwrap();
    assert!(
        error.contains("readonly: fs.write blocked"),
        "should surface the read-only block, not crash: {error}"
    );
}

#[test]
fn assay_context_returns_markdown() {
    let mut server = McpServer::start();
    server.initialize();

    let resp = server.call_tool("assay_context", json!({ "query": "grafana" }));
    let result = &resp["result"];
    assert_eq!(result["isError"], json!(false), "result: {result}");

    let markdown = text_content(result);
    assert!(
        markdown.contains("Assay Module Context"),
        "markdown header missing: {markdown}"
    );
    assert!(
        markdown.to_lowercase().contains("grafana"),
        "expected the grafana module in the docs: {markdown}"
    );
}

#[test]
fn assay_context_omits_builtins_unless_asked() {
    let mut server = McpServer::start();
    server.initialize();

    let resp = server.call_tool("assay_context", json!({ "query": "grafana" }));
    let markdown = text_content(&resp["result"]);

    assert!(
        markdown.to_lowercase().contains("grafana"),
        "the module docs are still the point: {markdown}"
    );
    assert!(
        !markdown.contains("Built-in Functions"),
        "the builtins block repeats on every call and must be opt-in over MCP: {markdown}"
    );
    assert!(
        !markdown.contains("http.get(url, opts?)"),
        "builtins body leaked: {markdown}"
    );
}

#[test]
fn assay_context_includes_builtins_on_request() {
    let mut server = McpServer::start();
    server.initialize();

    let resp = server.call_tool(
        "assay_context",
        json!({ "query": "grafana", "include_builtins": true }),
    );
    let markdown = text_content(&resp["result"]);

    assert!(
        markdown.contains("## Built-in Functions"),
        "include_builtins should bring the block back: {markdown}"
    );
    assert!(
        markdown.contains("http.get(url, opts?)"),
        "builtins body missing: {markdown}"
    );
}

#[test]
fn assay_context_schema_advertises_include_builtins() {
    let mut server = McpServer::start();
    server.initialize();

    let resp = server.request("tools/list", json!({}));
    let tools = resp["result"]["tools"].as_array().unwrap();
    let ctx = tools.iter().find(|t| t["name"] == "assay_context").unwrap();
    let flag = &ctx["inputSchema"]["properties"]["include_builtins"];

    assert_eq!(flag["type"], "boolean", "assay_context schema: {ctx}");
    assert_eq!(flag["default"], json!(false), "assay_context schema: {ctx}");
    let required = ctx["inputSchema"]["required"].as_array().unwrap();
    assert!(
        !required.contains(&json!("include_builtins")),
        "the flag is optional: {ctx}"
    );
}

#[test]
fn tools_list_includes_assay_resume() {
    let mut server = McpServer::start();
    server.initialize();

    let resp = server.request("tools/list", json!({}));
    let tools = resp["result"]["tools"].as_array().unwrap();
    let names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();
    assert!(
        names.contains(&"assay_resume"),
        "missing assay_resume: {names:?}"
    );

    let resume = tools.iter().find(|t| t["name"] == "assay_resume").unwrap();
    let props = &resume["inputSchema"]["properties"];
    assert!(
        props["token"].is_object(),
        "resume needs a token arg: {resume}"
    );
    assert!(
        props["approve"].is_object(),
        "resume needs an approve arg: {resume}"
    );
    assert!(
        props["approver"].is_object(),
        "resume offers an approver audit arg: {resume}"
    );
    let required = resume["inputSchema"]["required"].as_array().unwrap();
    assert!(
        required.contains(&json!("approve")),
        "approve must be required: {resume}"
    );
}

#[test]
fn approval_run_suspends_then_resume_completes_the_write() {
    let mut server = McpServer::start();
    server.initialize();

    let path = std::env::temp_dir().join(format!("assay-mcp-approval-{}", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let path_lua = path.to_str().unwrap().replace('\\', "\\\\");
    let script = format!(r#"fs.write("{path_lua}", "x"); return "done""#);

    // 1) approval mode → the mutating fs.write suspends and hands back a token.
    let run = server.call_tool("assay_run", json!({ "script": script, "mode": "approval" }));
    let envelope: Value = serde_json::from_str(text_content(&run["result"])).unwrap();
    assert_eq!(envelope["status"], "needs_approval", "envelope: {envelope}");
    let token = envelope["requiresApproval"]["resumeToken"]
        .as_str()
        .unwrap_or_else(|| panic!("no resumeToken in {envelope}"));
    assert!(!path.exists(), "the write must not land before approval");

    // 2) assay_resume approving the pending op → the run finishes, write lands.
    let resume = server.call_tool("assay_resume", json!({ "token": token, "approve": true }));
    let resume_result = &resume["result"];
    assert_eq!(
        resume_result["isError"],
        json!(false),
        "resume: {resume_result}"
    );
    let done: Value = serde_json::from_str(text_content(resume_result)).unwrap();
    assert_eq!(done["status"], "ok", "resumed envelope: {done}");
    assert!(path.exists(), "the approved write should have reached disk");

    let _ = std::fs::remove_file(&path);
}

#[test]
fn assay_resume_rejects_an_unknown_token() {
    let mut server = McpServer::start();
    server.initialize();

    let resp = server.call_tool(
        "assay_resume",
        json!({ "token": "deadbeefdeadbeefdeadbeefdeadbeef", "approve": true }),
    );
    let result = &resp["result"];
    assert_eq!(
        result["isError"],
        json!(true),
        "unknown token must error: {result}"
    );
    assert!(
        text_content(result).contains("invalid resume token"),
        "should explain the bad token: {result}"
    );
}

#[test]
fn assay_resume_requires_an_explicit_approve() {
    let mut server = McpServer::start();
    server.initialize();

    // No `approve` at all: the decision may never be defaulted — this must
    // fail as an argument error before the token is even looked at.
    let resp = server.call_tool(
        "assay_resume",
        json!({ "token": "deadbeefdeadbeefdeadbeefdeadbeef" }),
    );
    let result = &resp["result"];
    assert_eq!(
        result["isError"],
        json!(true),
        "omitted approve must error: {result}"
    );
    assert!(
        text_content(result).contains("explicit 'approve'"),
        "should demand the explicit decision: {result}"
    );
}

#[test]
fn assay_resume_echoes_the_approver_identity() {
    let mut server = McpServer::start();
    server.initialize();

    let path =
        std::env::temp_dir().join(format!("assay-mcp-approval-audit-{}", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let path_lua = path.to_str().unwrap().replace('\\', "\\\\");
    let script = format!(r#"fs.write("{path_lua}", "x"); return "done""#);

    let run = server.call_tool("assay_run", json!({ "script": script, "mode": "approval" }));
    let envelope: Value = serde_json::from_str(text_content(&run["result"])).unwrap();
    let token = envelope["requiresApproval"]["resumeToken"]
        .as_str()
        .unwrap_or_else(|| panic!("no resumeToken in {envelope}"));

    let resume = server.call_tool(
        "assay_resume",
        json!({ "token": token, "approve": true, "approver": "alice@example.com" }),
    );
    let done: Value = serde_json::from_str(text_content(&resume["result"])).unwrap();
    assert_eq!(done["status"], "ok", "resumed envelope: {done}");
    assert_eq!(
        done["approver"], "alice@example.com",
        "envelope must carry the audit identity: {done}"
    );

    let _ = std::fs::remove_file(&path);
}

#[test]
fn unknown_method_returns_json_rpc_error() {
    let mut server = McpServer::start();
    server.initialize();

    let resp = server.request("does/not/exist", json!({}));
    assert_eq!(resp["error"]["code"], json!(-32601), "response: {resp}");
    assert!(resp.get("result").is_none(), "response: {resp}");
}

#[test]
fn eof_triggers_clean_shutdown() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_assay"))
        .arg("mcp-serve")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();

    // Close stdin immediately; the server should exit 0 on EOF.
    drop(child.stdin.take());
    let status = child.wait().unwrap();
    assert!(
        status.success(),
        "clean shutdown should exit zero: {status}"
    );
}
