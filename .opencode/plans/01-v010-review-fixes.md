# 01 - v0.1.0 Review Fixes

**Status**: COMPLETED **Created**: 2026-02-09 **Completed**: 2026-02-10

---

## Context

Post-build code review of assay v0.1.0 identified 3 bugs, 5 security issues, and 5 design issues.
This plan addresses all of them in priority order before tagging v0.1.0.

Sources: Oracle review, librarian (mlua/reqwest research), explore (code quality scan).

---

## Bugs

### B1: Timeout loses ALL partial results [HIGH]

**File**: `src/runner.rs:15-42`

**Problem**: `run()` creates an empty `results` Vec on line 15. `run_all_checks()` builds its own
internal Vec. When `tokio::timeout` fires, the internal Vec is dropped -- partial results from
completed checks are lost. The `Err` branch marks ALL checks as timed out.

**Fix**: Refactor `run_all_checks` to take `&mut Vec<CheckResult>` and push results in-place. On
timeout, only remaining (unattempted) checks get marked as timed out.

```rust
// Before (broken):
let mut results: Vec<CheckResult> = Vec::with_capacity(config.checks.len());
let run_future = run_all_checks(config, &client);  // builds own Vec
match timeout(config.timeout, run_future).await {
    Ok(check_results) => { results = check_results; }
    Err(_) => { /* results is empty here */ }
}

// After (fixed):
let results = Arc::new(Mutex::new(Vec::with_capacity(config.checks.len())));
let run_future = run_all_checks(config, &client, results.clone());
match timeout(config.timeout, run_future).await {
    Ok(()) => {}
    Err(_) => { /* results has partial data */ }
}
let mut results = Arc::try_unwrap(results).unwrap().into_inner();
// fill remaining checks as timed out
```

**Files changed**: `src/runner.rs`

---

### B2: `http.post` rejects Lua tables [MEDIUM]

**File**: `src/lua/builtins.rs:79-83`

**Problem**: `http.post(url, body, opts)` only accepts string or nil body. Lua tables (the natural
way to pass structured data) are rejected with a runtime error. The plan and loki example expect
table auto-serialization to JSON.

**Fix**: Add `Value::Table` match arm that converts via a new `lua_table_to_json()` helper (inverse
of existing `json_value_to_lua()`). Also set `Content-Type: application/json` automatically when
body is a table.

**Files changed**: `src/lua/builtins.rs`

---

### B3: `loki-test.lua` uses sandboxed `os.clock()` [MEDIUM]

**File**: `examples/loki-test.lua:5`

**Problem**: `os` is removed by sandbox, so `os.clock()` crashes at runtime with nil index error.

**Fix**: Replace with a new `time()` builtin that returns epoch seconds as a float (via
`std::time::SystemTime`). Update example to use `time()` instead of `os.clock()`.

**Files changed**: `src/lua/builtins.rs`, `examples/loki-test.lua`

---

## Security Hardening

### S1: Remove `load()` from sandbox [MEDIUM-HIGH]

**File**: `src/lua/mod.rs:7`

**Problem**: `load()` can execute arbitrary bytecode strings, bypassing textual sandboxing.

**Fix**: Add `"load"` to `DANGEROUS_GLOBALS` list. Alternatively, use
`Lua::new_with(StdLib::ALL_SAFE, LuaOptions::default())` instead of `Lua::new()` (mlua's `ALL_SAFE`
already excludes `debug`, `io`, `os`, `package`). Then only additionally remove `load`,
`string.dump`, `collectgarbage`, `print`.

**Recommendation**: Switch to `Lua::new_with(StdLib::ALL_SAFE, ...)` as the base, then layer
additional removals on top. This is the mlua-blessed approach and more maintainable than manually
tracking every dangerous global.

**Files changed**: `src/lua/mod.rs`

---

### S2: Remove `string.dump` [MEDIUM]

**File**: `src/lua/mod.rs`

**Problem**: `string.dump()` serializes function bytecode. Combined with `load()`, enables sandbox
escape via bytecode roundtripping.

**Fix**: After loading string lib, remove dump:

```rust
let string_lib: mlua::Table = lua.globals().get("string")?;
string_lib.set("dump", mlua::Value::Nil)?;
```

**Files changed**: `src/lua/mod.rs`

---

### S3: Add VM memory limit [MEDIUM]

**File**: `src/lua/mod.rs`

**Problem**: No memory limit on Lua VM. A buggy or malicious script can OOM the process.

**Fix**: Add `lua.set_memory_limit(64 * 1024 * 1024)?;` (64MB) in `create_vm()`. mlua supports this
natively for Lua 5.4 via custom allocator. Returns `Error::MemoryError` when exceeded.

**Files changed**: `src/lua/mod.rs`

---

### S4: Remove `collectgarbage` [LOW]

**File**: `src/lua/mod.rs`

**Problem**: Can force GC pauses (DoS vector in adversarial contexts).

**Fix**: Add to removal list. Not critical for our use case (trusted scripts), but good hygiene.

**Files changed**: `src/lua/mod.rs`

---

### S5: Remove `print` [LOW]

**File**: `src/lua/mod.rs`

**Problem**: `print()` writes to stdout, which corrupts the structured JSON output. Scripts should
use `log.info()` instead (writes to stderr via tracing).

**Fix**: Add `"print"` to removal list.

**Files changed**: `src/lua/mod.rs`

---

## Design Improvements

### D1: Add per-request HTTP timeout [MEDIUM-HIGH]

**File**: `src/runner.rs` or `src/lua/mod.rs`

**Problem**: Individual HTTP requests have no timeout. A single `http.get()` to a slow server
consumes the entire global timeout window. Combined with B1, this means all checks get reported as
timed out.

**Fix**: Configure `reqwest::Client` with `.timeout(Duration::from_secs(30))` at construction time
in `runner.rs:14`. This applies to both YAML-mode checks and Lua builtins (same client). Also accept
an optional `timeout` field in opts tables for Lua builtins to override per-request.

**Files changed**: `src/runner.rs`, `src/lua/builtins.rs` (opts parsing)

---

### D2: Unify `env.get` and `ENV` table [LOW-MEDIUM]

**File**: `src/lua/builtins.rs`, `src/lua/mod.rs`

**Problem**: Two env mechanisms with confusingly similar names:

- `env.get("FOO")` reads process environment (`std::env::var`)
- `ENV.FOO` reads check-config env map (injected via `inject_env`)

Script authors will use the wrong one.

**Fix**: Make `env.get()` check the injected `ENV` table first, fall back to process env. Remove the
separate `ENV` global.

**Files changed**: `src/lua/builtins.rs`, `src/lua/mod.rs`

---

### D3: `json.encode` builtin [LOW]

**File**: `src/lua/builtins.rs`

**Problem**: No way to serialize Lua tables to JSON strings. Scripts must manually construct JSON
via string concatenation (error-prone, no escaping).

**Fix**: Add `json.encode(table) -> string` using the `lua_table_to_json()` helper (same one needed
for B2). Register alongside `json.parse`.

**Files changed**: `src/lua/builtins.rs`

---

### D4: `time()` builtin [LOW]

**File**: `src/lua/builtins.rs`

**Problem**: No way to get current time since `os` is sandboxed. Needed for timestamps in
verification scripts (e.g., Loki push).

**Fix**: Register a `time()` global that returns epoch seconds as float via
`SystemTime::now().duration_since(UNIX_EPOCH)`. Simple, no external deps.

**Files changed**: `src/lua/builtins.rs`

---

### D5: Return `ExitCode` instead of `process::exit()` [LOW]

**File**: `src/output.rs`, `src/main.rs`

**Problem**: `process::exit()` skips destructors. May lose final tracing log lines.

**Fix**: Change `main()` to return `ExitCode`. `RunResult::print_and_exit()` becomes
`RunResult::print()` returning `ExitCode`.

```rust
// main.rs
fn main() -> ExitCode {
    // ...
    result.print()
}

// output.rs
pub fn print(self) -> ExitCode {
    let json = serde_json::to_string_pretty(&self).expect("serialize");
    println!("{json}");
    if self.passed { ExitCode::SUCCESS } else { ExitCode::from(1) }
}
```

**Files changed**: `src/output.rs`, `src/main.rs`

---

## Not Fixing (Assessed, Accepted)

| Issue                                            | Reason                                                                                           |
| ------------------------------------------------ | ------------------------------------------------------------------------------------------------ |
| `assert.matches` uses `lua.load()`               | Oracle confirmed safe: `lua_string_literal` escapes properly, `string.find` never evaluates code |
| `reqwest::Client` cloned in closures             | `Client` is `Arc`-based, clone is cheap. Correct pattern for async closures                      |
| `config.name.clone()` in check results           | Strings are small, happens once per check. Not worth refactoring                                 |
| `prometheus.query` inconsistent types            | Defer to post-v0.1 -- current behavior is usable with `type()` check                             |
| `extract_string_arg` compiles Lua chunk per call | Minor perf, only fires for non-string assert messages (rare path)                                |

---

## Implementation Order

| Step      | Items                                                      | Files                                       | Estimate    |
| --------- | ---------------------------------------------------------- | ------------------------------------------- | ----------- |
| 1         | B1 (timeout) + D1 (per-request timeout)                    | `runner.rs`                                 | ~10 min     |
| 2         | S1+S2+S3+S4+S5 (sandbox hardening)                         | `lua/mod.rs`                                | ~10 min     |
| 3         | B2 (table body) + D3 (json.encode)                         | `lua/builtins.rs`                           | ~10 min     |
| 4         | B3 (loki example) + D4 (time builtin)                      | `lua/builtins.rs`, `examples/loki-test.lua` | ~5 min      |
| 5         | D2 (env unification)                                       | `lua/builtins.rs`, `lua/mod.rs`             | ~5 min      |
| 6         | D5 (ExitCode)                                              | `output.rs`, `main.rs`                      | ~5 min      |
| 7         | `cargo check && cargo clippy -- -D warnings && cargo test` | --                                          | ~5 min      |
| 8         | Re-run integration tests                                   | --                                          | ~2 min      |
| **Total** |                                                            |                                             | **~50 min** |

---

## Verification

After all fixes:

- [ ] `cargo check` clean
- [ ] `cargo clippy -- -D warnings` clean
- [ ] `cargo test` all pass
- [ ] Integration tests: `assay -c tests/pass.yaml` exits 0
- [ ] Integration tests: `assay -c tests/fail.yaml` exits 1
- [ ] Integration tests: `assay -c tests/script-test.yaml` exits 0
- [ ] Sandbox test: script using `load()`, `os`, `io`, `print` all error properly
- [ ] Timeout test: verify partial results preserved on global timeout
- [ ] Binary size still < 10MB

---

_Last Updated_: 2026-02-09
