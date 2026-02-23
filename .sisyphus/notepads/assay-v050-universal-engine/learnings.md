# Learnings

## 2026-02-23 Session Start

### Codebase State (v0.4.5 baseline)

- `src/main.rs`: 145 lines. Single `Cli` struct with positional `file: PathBuf`. Extension-based
  dispatch (yaml→YAML checks, lua→script). `run_yaml_checks()` and `run_lua_script()` are standalone
  async fns.
- `src/lib.rs`: 2 lines — only `pub mod lua;`
- `stdlib/`: 23 Lua files. None have LDoc headers yet. Pattern: `local M = {}`, then
  `M.client(url, opts)`, then `local function api_get/post/put/delete(self, path)`, then
  `function c:method()` client methods.
- `grafana.lua`: 110 lines, 10 client methods: health, datasources, datasource, search, dashboard,
  annotations, create_annotation, org, alert_rules, folders. Auth: api_key OR username+password.
- Cargo.toml: features `default = ["db", "server", "cli"]`. sqlx with sqlite feature already
  present.

### Key Conventions

- Error format: `"<module>: <METHOD> <path> HTTP <status>: <body>"`
- Strip trailing slashes: `url:gsub("/+$", "")`
- 404 = nil pattern: check `resp.status == 404`, return `nil`
- Tests use wiremock: `MockServer::start().await`,
  `Mock::given(method("GET")).and(path("/...")).respond_with(...).mount(&server).await`
- Test helper: `common::run_lua(script)` from `tests/common/mod.rs`

### LDoc Header Format

```lua
--- @module assay.grafana
--- @description Grafana monitoring and dashboards. Health, datasources, annotations, alerts, folders.
--- @keywords grafana, monitoring, dashboards, datasources, annotations, alerts, health
--- @env GRAFANA_URL, GRAFANA_API_KEY
--- @quickref c:health() -> {database, version, commit} | Check Grafana health
--- @quickref c:datasources() -> [{id, name, type, url}] | List all datasources
```

Rules: triple-dash `--- @`, header at TOP before `local M = {}`, @quickref only for
`function c:method()` (not local helpers).

## 2026-02-23 Discovery Module

### Implementation Notes

`include_dir!` crate's `Dir::files()` iterates all embedded files — used for stdlib discovery
`STDLIB_DIR` static must be duplicated (not shared) between `lua/mod.rs` and `discovery.rs` since
`include_dir!` is a macro invocation tied to a specific static FTS5Index `add_document` field name
mapping: "name" maps to name column, anything unknown falls to functions column catch-all. Passing
"module_name" as field name goes to functions column in FTS5 but works correctly in BM25Index
`#[cfg(feature = "db")]` / `#[cfg(not(feature = "db"))]` blocks: clippy flags `return` in cfg-gated
blocks as needless when only one branch compiles. Use expression-style (no `return`) for both
branches `Box<dyn SearchEngine>` — trait methods are callable without importing the trait (vtable
dispatch), so `use SearchEngine` import is unused in tests 17 Rust builtins + 23 stdlib .lua files =
40 total modules discovered

## 2026-02-23 Context Subcommand Wiring

### Tokio Runtime Nesting Gotcha

FTS5Index creates its own `tokio::Runtime` internally (`Runtime::new()` + `block_on()`). Calling
discovery/search from within `#[tokio::main]` causes panic: "Cannot start a runtime from within a
runtime". Fix: spawn a `std::thread` to run discovery outside the tokio runtime context. This
pattern will apply to any sync function that calls `search_modules()` or `build_index()` from async
context.

### Crate Access Pattern

- Binary crate (`src/main.rs`) uses `mod checks; mod config; mod lua;` for binary-only modules
- Library crate name is `assay` (from `[lib] name = "assay"` in Cargo.toml)
- Library modules accessed via `use assay::context::...` and `use assay::discovery::...`
- SearchResult fields: `id: String, score: f64`

## 2026-02-23 Modules Subcommand

### Implementation Notes

`run_modules()` is sync (no `std::thread::spawn` needed) because `discover_modules()` doesn't use
FTS5Index or any async runtime. Only `search_modules()` triggers the tokio nesting issue. Clippy
`print_literal` lint: `println!("{:<30} {:<10} {}", "A", "B", "C")` triggers it for the last literal
arg with plain `{}` format. Fix: inline the literal into the format string directly.
`discover_modules()` returns 40 modules (23 stdlib + 17 Rust builtins). Dedup by name with
`HashSet::insert` preserving priority order (Project > Global > BuiltIn).
