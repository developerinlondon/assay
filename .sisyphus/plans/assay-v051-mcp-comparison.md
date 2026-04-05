# Assay v0.5.1 — MCP Comparison, Keyword Enrichment & Website

## TL;DR

> **Quick Summary**: Enrich search keywords across all 40 modules to fix search gaps, create a
> static website at assay.rs showing how Assay replaces 42 popular MCP servers, add AI agent
> integration guides, and publish llms.txt for agent traversal.
>
> **Deliverables**:
>
> - Enriched `@keywords` on 23 stdlib modules + 17 builtins (fixing search for "jwt", "tls",
>   "backup", etc.)
> - Static website at assay.rs with 4 pages: homepage, MCP comparison, agent guides, module
>   reference
> - `llms.txt` + `llms-full.txt` for LLM agent context traversal
> - GitHub Actions deploy workflow for Cloudflare Pages
> - Updated README.md and SKILL.md with v0.5.1 content
> - Version bump to 0.5.1
>
> **Estimated Effort**: Medium **Parallel Execution**: YES — 4 waves **Critical Path**: T1 (search
> tests) → T4/T5 (keyword enrichment) → T11 (llms-full.txt) → T14 (version bump)

---

## Context

### Original Request

User wants v0.5.1 to transform Assay's positioning by showing how a single 9 MB binary replaces
dozens of MCP servers. v0.5.0 QA revealed search gaps (e.g., "jwt" doesn't find crypto module). The
release includes keyword enrichment, a marketing/documentation website, LLM agent integration
guides, and llms.txt.

### Interview Summary

**Key Discussions**:

- **Website directory**: `site/` — pure static HTML/CSS/JS, no framework
- **MCP-serve vision**: Show future "before/after" config as roadmap teaser alongside current
  integration via `assay context` and SKILL.md
- **Proposed modules**: Show all 42 MCP servers in comparison — 25 "Available Now" + 15+ "Coming
  Soon"
- **Cloudflare**: User will set up DNS + auth manually; plan includes deploy workflow + setup
  instructions
- **Test strategy**: TDD — write failing search tests first, then enrich keywords to make them pass

**Research Findings**:

- **Keyword audit**: All 23 stdlib modules missing 3-9 keywords each. All 17 builtins have only
  their name as keyword.
- **MCP landscape**: 42 servers researched across 4 tiers. Assay covers 25 domains fully. Key
  replacements: mcp-server-kubernetes, mcp-grafana, vault-mcp-server, postgres-mcp,
  mcp-server-fetch/filesystem.
- **llms.txt**: Jeremy Howard spec — H1, blockquote, H2 sections with `[name](url): description`.
  844K+ sites implement it.
- **Agent configs**: Claude Code + Cursor share `.mcp.json` format. Windsurf, Cline, OpenCode each
  have their own.
- **Cloudflare Pages**: Direct upload via `wrangler pages deploy`, free tier, custom domains require
  zone setup.

### Metis Review

**Identified Gaps** (addressed):

- **BUILTINS type change**: Current type is `&[(&str, &str)]` — must change to
  `&[(&str, &str, &[&str])]` and update `discover_rust_builtins()`. This is a Rust struct change,
  not just value updates.
- **TDD RED validation**: Must verify test terms genuinely fail against BM25 tokenizer
  (underscore-preserved, case-insensitive, splits on hyphens). Some terms like "hash" already match
  via description field.
- **Search regression**: Adding "monitoring" to 5+ modules dilutes BM25 IDF. Must capture baseline
  rankings before enrichment.
- **Website scope cap**: Max 4 HTML pages, table format for MCP comparisons (not prose), no JS
  frameworks.
- **llms.txt location**: Dual — `site/llms.txt` (deployed) + `llms.txt` at repo root (GitHub
  access).
- **Deploy workflow isolation**: Separate from release.yml, triggered on push to main.
- **MCP comparison honesty**: Add coverage qualifiers (full/partial/different-approach) per entry.
- **dprint gap**: HTML/CSS/JS not covered by existing dprint config — documented as known
  limitation.

---

## Work Objectives

### Core Objective

Fix search discoverability gaps and position Assay as the MCP ecosystem replacement through enriched
keywords, a comparison website, agent integration guides, and llms.txt.

### Concrete Deliverables

- 23 stdlib `@keywords` lines enriched (line 3 of each `stdlib/*.lua`)
- 17 builtin keywords enriched in `src/discovery.rs` (BUILTINS constant + type change)
- `site/` directory with 4 HTML pages + CSS + _headers + _redirects
- `site/llms.txt` + `site/llms-full.txt` + repo root `llms.txt`
- `.github/workflows/deploy.yml` for Cloudflare Pages
- Updated `README.md` and `SKILL.md`
- Version 0.5.1 in `Cargo.toml`

### Definition of Done

[x] `cargo test` passes (all existing + new search tests) [x] `cargo clippy -- -D warnings` passes
[x] `assay context "jwt"` returns crypto module [x] `assay context "tls"` returns certmanager module
[x] `assay context "backup"` returns velero module [x] `site/index.html` exists and is valid HTML
[x] `site/llms.txt` follows Jeremy Howard spec [x] Version is 0.5.1 in Cargo.toml

### Must Have

- Enriched keywords on ALL 23 stdlib + ALL 17 builtins (no gaps)
- MCP comparison table with 42 servers showing Assay equivalents
- Agent integration guides for 5 agents (Claude Code, Cursor, Windsurf, Cline, OpenCode)
- llms.txt with grouped sections (Monitoring, K8s/GitOps, Security, etc.)
- TDD search tests that verify keyword enrichment

### Must NOT Have (Guardrails)

- NO `assay mcp-serve` subcommand implementation (document as future vision only)
- NO new stdlib modules (no new .lua files)
- NO JavaScript frameworks, CSS preprocessors, or build tools on website
- NO analytics, tracking, or third-party scripts on website
- NO JavaScript-based search feature on website
- NO more than 4 HTML pages (index, mcp-comparison, agent-guides, modules)
- NO changes to `Dockerfile`, `release.yml`, or existing CI workflows
- NO modification to stdlib `.lua` files beyond the `--- @keywords` line (line 3)
- NO modification to `discovery.rs` beyond `BUILTINS` constant and `discover_rust_builtins()`
  function
- NO SEO meta tags beyond basic title/description
- NO `robots.txt` or `sitemap.xml` (site won't be indexed until DNS is configured)
- NO before/after code snippets for ALL 42 MCP comparisons — use table format with status badges
- NO rewriting SKILL.md from scratch — append sections only
- NO adding `site/` files to dprint includes (HTML/CSS not covered)
- NO binary size growth beyond 12 MB

---

## Verification Strategy

> **ZERO HUMAN INTERVENTION** — ALL verification is agent-executed. No exceptions.

### Test Decision

- **Infrastructure exists**: YES (cargo test, 580+ existing tests)
- **Automated tests**: TDD (RED → GREEN → REFACTOR)
- **Framework**: cargo test (Rust)
- **TDD flow**: Write failing search tests in Wave 1 → enrich keywords in Wave 2 → tests pass

### QA Policy

Every task MUST include agent-executed QA scenarios. Evidence saved to
`.sisyphus/evidence/task-{N}-{scenario-slug}.{ext}`.

- **Search/Keyword tasks**: Use Bash — run `cargo test`, run binary `assay context <query>`, verify
  output
- **Website tasks**: Use Bash — verify file existence, validate HTML structure, check content
- **Config/Workflow tasks**: Use Bash — validate YAML/JSON syntax, check file structure

---

## Execution Strategy

### Parallel Execution Waves

```
Wave 1 (Foundation — 3 parallel tasks):
├── Task 1: TDD RED — write failing search tests for keyword gaps [unspecified-high]
├── Task 2: Site scaffold — directory, base CSS, _headers, _redirects, wrangler.toml [quick]
└── Task 3: Create llms.txt following Jeremy Howard spec [quick]

Wave 2 (Content + Keywords — 7 parallel tasks):
├── Task 4: TDD GREEN — enrich @keywords on all 23 stdlib modules (depends: T1) [unspecified-high]
├── Task 5: TDD GREEN — enrich builtin keywords in discovery.rs (depends: T1) [unspecified-high]
├── Task 6: Site homepage — site/index.html (depends: T2) [unspecified-high]
├── Task 7: Site MCP comparison page — 42 servers mapped (depends: T2) [unspecified-high]
├── Task 8: Site agent integration guides — 5 agents (depends: T2) [unspecified-high]
├── Task 9: Site module reference page — all modules listed (depends: T2) [unspecified-high]
└── Task 10: GitHub Actions deploy workflow for CF Pages (depends: T2) [quick]

Wave 3 (Integration — 4 parallel tasks):
├── Task 11: Create llms-full.txt with enriched module docs (depends: T4, T5) [quick]
├── Task 12: Update README.md for v0.5.1 (depends: T4-T9) [quick]
├── Task 13: Update SKILL.md with MCP comparison + agent info (depends: T7, T8) [quick]
└── Task 14: Version bump to 0.5.1 + Cargo.lock update (depends: T4-T13) [quick]

Wave FINAL (After ALL tasks — 4 parallel review agents):
├── Task F1: Plan compliance audit (oracle)
├── Task F2: Code quality review (unspecified-high)
├── Task F3: Real QA — search tests, website content, llms.txt (unspecified-high)
└── Task F4: Scope fidelity check (unspecified-high)

Critical Path: T1 → T4/T5 → T11 → T14 → F1-F4
Parallel Speedup: ~65% faster than sequential
Max Concurrent: 7 (Wave 2)
```

### Dependency Matrix

| Task  | Depends On | Blocks              | Wave  |
| ----- | ---------- | ------------------- | ----- |
| T1    | —          | T4, T5              | 1     |
| T2    | —          | T6, T7, T8, T9, T10 | 1     |
| T3    | —          | T11                 | 1     |
| T4    | T1         | T11, T12, T14       | 2     |
| T5    | T1         | T11, T12, T14       | 2     |
| T6    | T2         | T12                 | 2     |
| T7    | T2         | T12, T13            | 2     |
| T8    | T2         | T12, T13            | 2     |
| T9    | T2         | T12                 | 2     |
| T10   | T2         | —                   | 2     |
| T11   | T3, T4, T5 | T14                 | 3     |
| T12   | T4-T9      | T14                 | 3     |
| T13   | T7, T8     | T14                 | 3     |
| T14   | T4-T13     | F1-F4               | 3     |
| F1-F4 | T14        | —                   | FINAL |

### Agent Dispatch Summary

- **Wave 1**: 3 tasks — T1 → `unspecified-high`, T2 → `quick`, T3 → `quick`
- **Wave 2**: 7 tasks — T4-T5 → `unspecified-high`, T6-T9 → `unspecified-high`, T10 → `quick`
- **Wave 3**: 4 tasks — T11-T14 → `quick`
- **Wave FINAL**: 4 tasks — F1 → `oracle`, F2-F4 → `unspecified-high`

---

## TODOs

- 1. [x] TDD RED — Write Failing Search Tests for Keyword Gaps

  **What to do**:
  - Create new test functions in `tests/discovery.rs` that search for terms currently missing from
    module keywords
  - Before writing each test, verify the search term does NOT already match by checking: (a)
    existing `@keywords` line, (b) module description field, (c) module name, (d) auto_functions —
    using BM25 tokenizer rules: split on non-alphanumeric except underscore, lowercase, filter len ≤
    1
  - Write ~15 tests covering these verified-to-fail search terms:
    - `"jwt"` → should find crypto (currently "jwt_sign" is one token, "jwt" alone won't match)
    - `"request"` → should find http (not in description "HTTP client and server: get, post, put,
      patch, delete, serve")
    - `"endpoint"` → should find http
    - `"password"` → should find crypto
    - `"tls"` → should find certmanager (keyword has "tls" but verify — it does, so use
      `"letsencrypt"` instead)
    - `"letsencrypt"` → should find certmanager
    - `"backup"` → should find velero
    - `"snapshot"` → should find velero
    - `"toggle"` → should find unleash
    - `"crd"` → should find k8s
    - `"rollout"` → should find k8s
    - `"encryption"` → should find vault
    - `"pipeline"` → should find kargo
    - `"observability"` → should find prometheus or grafana
    - `"seal"` → should find vault
  - Also write 3-5 search REGRESSION tests that capture current behavior:
    - `"grafana"` → must return assay.grafana first (capture current ranking)
    - `"http"` → must return http builtin first
    - `"vault"` → must return assay.vault first
    - `"prometheus"` → must return assay.prometheus first
  - Run `cargo test --test discovery` and confirm all new keyword tests FAIL (RED state) while
    regression tests PASS

  **Must NOT do**:
  - Do NOT modify any `@keywords` lines or discovery.rs — only add tests
  - Do NOT use test terms that already match via description tokenization (e.g., "hash" matches
    crypto's description, "websocket" matches ws description, "logging" matches log description)
  - Do NOT add more than 20 new test functions

  **Recommended Agent Profile**:
  - **Category**: `unspecified-high`
    - Reason: Requires careful BM25 tokenizer analysis to verify terms genuinely fail
  - **Skills**: []
  - **Skills Evaluated but Omitted**:
    - `code-quality`: Not needed for test-only changes

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 1 (with Tasks 2, 3)
  - **Blocks**: Tasks 4, 5
  - **Blocked By**: None (can start immediately)

  **References**:

  **Pattern References**:
  - `tests/discovery.rs` — Existing discovery tests, follow same `#[test]` and `#[tokio::test]`
    patterns
  - `src/search.rs:73-78` — BM25 tokenizer function: `fn tokenize(text: &str) -> Vec<String>` —
    splits on non-alphanumeric except underscore, lowercases, filters len > 1

  **API/Type References**:
  - `src/discovery.rs:40-73` — BUILTINS constant with current (name, description) pairs — check
    description field for existing token matches
  - `src/discovery.rs:234-248` — `discover_rust_builtins()` function showing how keywords are set to
    `vec![name.to_string()]`
  - `src/search.rs` — `SearchEngine` trait, `BM25Index` struct

  **Test References**:
  - `tests/discovery.rs` — All existing tests follow pattern: create index, add documents, search,
    assert results
  - `tests/search.rs` — BM25 unit tests showing search behavior

  **WHY Each Reference Matters**:
  - `search.rs:73-78`: CRITICAL — must understand tokenizer rules to verify which terms will
    genuinely fail vs false-green
  - `discovery.rs:40-73`: Must check each builtin's description string for tokens that would match
    search terms
  - `tests/discovery.rs`: Follow existing patterns for consistency

  **Acceptance Criteria**:
  - [x] ~15 new keyword gap tests added to `tests/discovery.rs`
  - [x] ~4 regression tests added capturing current search rankings
  - [x] `cargo test --test discovery` runs — new keyword tests FAIL, regression tests PASS
  - [x] Each test term verified to NOT match via description/name/existing-keywords

  **QA Scenarios:**

  ```
  Scenario: New keyword tests fail (TDD RED)
    Tool: Bash
    Preconditions: No changes to stdlib/*.lua or src/discovery.rs
    Steps:
      1. Run: cargo test --test discovery 2>&1
      2. Count FAILED tests — should be ~15 (keyword gap tests)
      3. Count PASSED tests — should include existing tests + regression tests
    Expected Result: Exit code non-zero, ~15 keyword tests FAILED, all regression tests PASS
    Failure Indicators: Exit code 0 (all tests pass — means terms already match, bad RED state)
    Evidence: .sisyphus/evidence/task-1-tdd-red-results.txt

  Scenario: Regression tests pass with current code
    Tool: Bash
    Preconditions: No modifications to any source files
    Steps:
      1. Run: cargo test --test discovery test_search_regression -- --nocapture 2>&1
      2. Verify all regression tests pass
    Expected Result: Exit code 0 for regression tests
    Failure Indicators: Any regression test fails — means baseline capture is wrong
    Evidence: .sisyphus/evidence/task-1-regression-baseline.txt
  ```

  **Commit**: YES
  - Message: `test(search): add TDD RED tests for keyword enrichment gaps`
  - Files: `tests/discovery.rs`
  - Pre-commit: `cargo test --test discovery 2>&1 | grep -c FAILED` (expect > 0)

- 2. [x] Site Scaffold — Directory Structure, Base CSS, Headers, Wrangler Config

  **What to do**:
  - Create `site/` directory with the following structure:
    ```
    site/
    ├── index.html          (placeholder — filled in T6)
    ├── mcp-comparison.html  (placeholder — filled in T7)
    ├── agent-guides.html    (placeholder — filled in T8)
    ├── modules.html         (placeholder — filled in T9)
    ├── style.css            (shared stylesheet)
    ├── _headers             (Cloudflare Pages headers config)
    └── _redirects           (Cloudflare Pages redirects)
    ```
  - Create `site/style.css` — clean, minimal CSS for a documentation site:
    - Dark/light theme via `prefers-color-scheme` media query
    - Responsive layout (max-width: 900px centered)
    - Code block styling for Lua/JSON snippets
    - Table styling for MCP comparison tables
    - Navigation header + footer
    - NO CSS framework (Tailwind, Bootstrap, etc.) — hand-written only
  - Create `site/_headers`:
    ```
    /*
      X-Frame-Options: DENY
      X-Content-Type-Options: nosniff
    /llms.txt
      Content-Type: text/plain; charset=utf-8
      Cache-Control: public, max-age=3600
    /llms-full.txt
      Content-Type: text/plain; charset=utf-8
      Cache-Control: public, max-age=3600
    ```
  - Create `site/_redirects`:
    ```
    /github  https://github.com/developerinlondon/assay  302
    /crates  https://crates.io/crates/assay-lua  302
    ```
  - Create `wrangler.toml` at repo root:
    ```toml
    name = "assay-rs"
    pages_build_output_dir = "./site"
    compatibility_date = "2026-02-01"
    ```
  - Create placeholder HTML files with consistent structure: DOCTYPE, head with meta
    charset/viewport/title, link to style.css, nav, main, footer

  **Must NOT do**:
  - Do NOT install any npm packages or build tools
  - Do NOT add JavaScript files
  - Do NOT create more than 4 HTML files
  - Do NOT add analytics or tracking scripts

  **Recommended Agent Profile**:
  - **Category**: `quick`
    - Reason: File creation with well-defined content, no complex logic
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 1 (with Tasks 1, 3)
  - **Blocks**: Tasks 6, 7, 8, 9, 10
  - **Blocked By**: None (can start immediately)

  **References**:

  **Pattern References**:
  - `README.md` — Current project styling and branding to match on website

  **External References**:
  - Cloudflare Pages _headers docs: https://developers.cloudflare.com/pages/configuration/headers/
  - Cloudflare Pages _redirects docs:
    https://developers.cloudflare.com/pages/configuration/redirects/
  - Wrangler Pages config:
    https://developers.cloudflare.com/pages/configuration/wrangler-configuration/

  **WHY Each Reference Matters**:
  - `README.md`: Website should visually match project branding and use same terminology
  - CF docs: Must follow exact syntax for _headers and _redirects files

  **Acceptance Criteria**:
  - [x] `site/` directory exists with 4 placeholder HTML files
  - [x] `site/style.css` exists with responsive layout + dark/light theme
  - [x] `site/_headers` exists with Content-Type rules for llms.txt
  - [x] `site/_redirects` exists
  - [x] `wrangler.toml` exists at repo root with correct project name
  - [x] All HTML files valid (DOCTYPE, head, body)

  **QA Scenarios:**

  ```
  Scenario: Site directory structure is correct
    Tool: Bash
    Steps:
      1. Run: ls -la site/index.html site/mcp-comparison.html site/agent-guides.html site/modules.html site/style.css site/_headers site/_redirects
      2. Run: cat wrangler.toml
    Expected Result: All 7 site files exist; wrangler.toml contains name = "assay-rs"
    Failure Indicators: Any file missing, wrangler.toml missing or wrong project name
    Evidence: .sisyphus/evidence/task-2-site-structure.txt

  Scenario: HTML files have valid structure
    Tool: Bash
    Steps:
      1. Run: head -5 site/index.html
      2. Verify contains <!DOCTYPE html> and <html
      3. Run: grep -l 'style.css' site/*.html | wc -l
    Expected Result: All 4 HTML files link to style.css
    Failure Indicators: Missing DOCTYPE, no CSS link
    Evidence: .sisyphus/evidence/task-2-html-validation.txt
  ```

  **Commit**: YES
  - Message: `chore(site): scaffold static website directory structure`
  - Files: `site/*`, `wrangler.toml`
  - Pre-commit: `ls site/index.html site/style.css site/_headers wrangler.toml`

- 3. [x] Create llms.txt Following Jeremy Howard Spec

  **What to do**:
  - Create `site/llms.txt` following the llms.txt specification (https://llmstxt.org/)
  - Structure:
    ```
    # Assay

    > Assay is a ~9 MB static binary Lua runtime for Kubernetes. It replaces 50-250 MB
    > Python/Node/kubectl containers in K8s Jobs. Single binary with built-in HTTP, database,
    > crypto, WebSocket, and 23 Kubernetes-native stdlib modules. No require() for builtins.
    > Stdlib uses require("assay.<name>") with client pattern: M.client(url, opts) → c:method().
    > Use `assay context <query>` to get LLM-ready method signatures.

    ## Getting Started
    - [README](https://github.com/developerinlondon/assay/blob/main/README.md): Installation, usage, examples
    - [SKILL.md](https://github.com/developerinlondon/assay/blob/main/SKILL.md): Agent integration guide

    ## Built-in Globals (no require needed)
    - [HTTP](https://assay.rs/modules.html#http): http.get/post/put/patch/delete/serve
    - [JSON/YAML/TOML](https://assay.rs/modules.html#serialization): json/yaml/toml.parse/encode
    - [Crypto](https://assay.rs/modules.html#crypto): crypto.jwt_sign, hash, hmac, random
    - [Database](https://assay.rs/modules.html#db): db.connect/query/execute/close
    ... (continue for all builtins and grouped stdlib sections)

    ## Monitoring
    - [assay.prometheus](https://assay.rs/modules.html#prometheus): PromQL queries, alerts, targets
    ... (continue for all stdlib grouped by domain)

    ## Optional
    - [Crates.io](https://crates.io/crates/assay-lua): Rust crate for embedding
    - [Changelog](https://github.com/developerinlondon/assay/releases)
    ```
  - Group stdlib sections by domain: Monitoring, Kubernetes & GitOps, Security & Identity,
    Infrastructure, Data & Storage, Feature Flags
  - Include Stripe-style "Important notes for LLM agents" in the blockquote: no require for
    builtins, client pattern, error handling via error()/pcall(), HTTP response format
  - Copy `site/llms.txt` to repo root `llms.txt` (dual location: site/ for website, root for GitHub)
  - Link URLs to assay.rs pages where possible, GitHub raw URLs for .lua/.rs files
  - Use `blob/main/` not `blob/v0.5.1/` in GitHub URLs (avoids stale links)

  **Must NOT do**:
  - Do NOT include full API docs inline (that's llms-full.txt in T11)
  - Do NOT link to non-existent pages — only link to GitHub files and assay.rs pages being created
  - Do NOT add more than 50 link entries (keep concise)

  **Recommended Agent Profile**:
  - **Category**: `quick`
    - Reason: Well-defined content structure, no complex logic — following a spec
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 1 (with Tasks 1, 2)
  - **Blocks**: Task 11
  - **Blocked By**: None (can start immediately)

  **References**:

  **Pattern References**:
  - `AGENTS.md` — Built-in globals table and stdlib module table — use as source for link entries
  - `SKILL.md` — Agent integration guide, use descriptions for llms.txt entries

  **External References**:
  - llms.txt spec: https://llmstxt.org/
  - FastHTML example: https://www.fastht.ml/docs/llms.txt (reference implementation)
  - Stripe example: https://docs.stripe.com/llms.txt (LLM agent instructions pattern)

  **WHY Each Reference Matters**:
  - `AGENTS.md`: Source of truth for module names, descriptions, and function signatures
  - llmstxt.org: Must follow exact spec format (H1, blockquote, H2 sections)
  - FastHTML/Stripe: Real examples of well-structured llms.txt files to emulate

  **Acceptance Criteria**:
  - [x] `site/llms.txt` exists
  - [x] `llms.txt` exists at repo root (copy of site/llms.txt)
  - [x] First line is `# Assay`
  - [x] Blockquote with LLM agent notes present
  - [x] H2 sections for each domain group
  - [x] `## Optional` section present
  - [x] All 23 stdlib + 17 builtins referenced

  **QA Scenarios:**

  ```
  Scenario: llms.txt follows Jeremy Howard spec
    Tool: Bash
    Steps:
      1. Run: head -1 site/llms.txt
      2. Assert: output is "# Assay"
      3. Run: grep -c '^>' site/llms.txt
      4. Assert: blockquote lines > 0
      5. Run: grep -c '^## ' site/llms.txt
      6. Assert: at least 6 H2 sections
      7. Run: grep -c '## Optional' site/llms.txt
      8. Assert: exactly 1 Optional section
    Expected Result: H1 is "# Assay", blockquote present, 6+ sections, Optional section exists
    Failure Indicators: Missing H1, no blockquote, fewer than 6 sections
    Evidence: .sisyphus/evidence/task-3-llms-txt-spec.txt

  Scenario: Repo root copy matches site version
    Tool: Bash
    Steps:
      1. Run: diff site/llms.txt llms.txt
    Expected Result: No differences (exit code 0)
    Failure Indicators: Files differ
    Evidence: .sisyphus/evidence/task-3-llms-txt-sync.txt
  ```

  **Commit**: YES
  - Message: `docs(llms): add llms.txt for LLM agent context traversal`
  - Files: `site/llms.txt`, `llms.txt`
  - Pre-commit: `head -1 site/llms.txt | grep '^# Assay'`

- 4. [x] TDD GREEN — Enrich @keywords on All 23 Stdlib Modules

  **What to do**:
  - For each of the 23 `stdlib/*.lua` files, update ONLY the `--- @keywords` line (line 3) to add
    missing keywords identified in the keyword audit
  - Specific enrichments per module (add to EXISTING keywords, don't replace):
    - `grafana.lua`: add `organization, folders, search`
    - `prometheus.lua`: add `instant-query, range-query, scrape, metadata, reload, observability`
    - `alertmanager.lua`: add `silence, inhibit, grouping, notification, receiver`
    - `loki.lua`: add `push, tail, stream, instant, range`
    - `k8s.lua`: add `crd, custom-resources, rbac, events, logs, rollout, nodes, readiness, wait`
    - `argocd.lua`: add `rollback, manifest, resource-tree, refresh, wait`
    - `kargo.lua`: add `promotion, pipeline, health, wait, status`
    - `flux.lua`: add `helm, oci, image-automation, notification, readiness, sources`
    - `traefik.lua`: add `http, tcp, tls, configuration, dashboard`
    - `vault.lua`: add
      `encryption, decryption, certificate, seal, initialization, authentication, secret-engine`
    - `certmanager.lua`: add `letsencrypt, order, challenge, request, approval, readiness, wait`
    - `eso.lua`: add `sync, store, readiness, wait, cluster`
    - `dex.lua`: add `openid-configuration, key-set, scope, grant-type, response-type, validation`
    - `crossplane.lua`: add
      `configuration, function, composition, managed-resource, health, readiness, established`
    - `velero.lua`: add
      `backup, restore, schedule, storage-location, snapshot, repository, completion, status`
    - `temporal.lua`: add
      `workflow, task-queue, schedule, signal, history, search, namespace, execution`
    - `harbor.lua`: add
      `project, repository, artifact, tag, scan, vulnerability, replication, image`
    - `healthcheck.lua`: add `json-path, body-match, latency, multi-check, wait, endpoint`
    - `s3.lua`: add `bucket, object, copy, metadata, signature-v4, compatible`
    - `postgres.lua`: add `user, database, grant, privilege, vault, connection`
    - `zitadel.lua`: add `domain, app, idp, login-policy, user, password, google, machine-key`
    - `unleash.lua`: add `feature, toggle, strategy, environment, token, api-token, archive`
    - `openbao.lua`: add
      `encryption, decryption, certificate, seal, initialization, authentication, secret-engine`
      (mirrors vault)
  - After updating, run `cargo test --test discovery` — keyword gap tests from T1 should now PASS
    (GREEN)
  - IMPORTANT: Only modify line 3 (`--- @keywords ...`) in each file. Do NOT touch any other lines.

  **Must NOT do**: Do NOT modify any line other than line 3. Do NOT add new stdlib modules. Do NOT
  change function signatures.

  **Recommended Agent Profile**: **Category**: `unspecified-high` | **Skills**: []

  **Parallelization**: Wave 2 (parallel with T5-T10) | **Blocks**: T11, T12, T14 | **Blocked By**:
  T1

  **References**: `stdlib/grafana.lua:1-5` (LDoc header format), `src/search.rs:73-78` (BM25
  tokenizer), `src/metadata.rs` (LDoc parser)

  **Acceptance Criteria**:
  - [x] All 23 stdlib files have enriched @keywords lines
  - [x] `cargo test --test discovery` — all stdlib keyword gap tests PASS
  - [x] Only line 3 modified in each file

  **QA Scenarios:**
  ```
  Scenario: Stdlib keyword tests pass (TDD GREEN)
    Tool: Bash
    Steps: 1. Run: cargo test --test discovery 2>&1  2. Verify stdlib keyword tests pass
    Expected Result: All stdlib keyword tests pass
    Evidence: .sisyphus/evidence/task-4-stdlib-keywords-green.txt
  Scenario: Only @keywords lines modified
    Tool: Bash
    Steps: 1. Run: git diff --unified=0 stdlib/*.lua  2. Verify only line 3 changes
    Expected Result: All diffs show only line 3 changes
    Evidence: .sisyphus/evidence/task-4-stdlib-diff-check.txt
  ```
  **Commit**: YES | `feat(search): enrich @keywords on all 23 stdlib modules` | Files:
  `stdlib/*.lua`

- 5. [x] TDD GREEN — Enrich Builtin Keywords in discovery.rs

  **What to do**:
  - Change `BUILTINS` constant type from `&[(&str, &str)]` to `&[(&str, &str, &[&str])]` (name,
    description, keywords)
  - Update all 17 builtin entries with enriched keyword arrays:
    - `http`: `["http", "client", "server", "request", "response", "headers", "endpoint", "api"]`
    - `json`: `["json", "serialization", "deserialize", "stringify", "parse", "encode"]`
    - `yaml`: `["yaml", "serialization", "deserialize", "parse", "encode"]`
    - `toml`: `["toml", "serialization", "deserialize", "parse", "encode", "configuration"]`
    - `fs`: `["fs", "filesystem", "file", "read", "write", "io"]`
    - `crypto`:
      `["crypto", "jwt", "signature", "hash", "hmac", "encryption", "random", "security", "password", "signing"]`
    - `base64`: `["base64", "encoding", "decode", "encode"]`
    - `regex`: `["regex", "pattern", "match", "find", "replace", "regular-expression"]`
    - `db`: `["db", "database", "sql", "postgres", "mysql", "sqlite", "connection", "query"]`
    - `ws`: `["ws", "websocket", "connection", "message", "streaming", "realtime"]`
    - `template`: `["template", "jinja2", "rendering", "string-template", "mustache"]`
    - `async`: `["async", "asynchronous", "task", "coroutine", "concurrent", "spawn", "interval"]`
    - `assert`: `["assert", "assertion", "test", "validation", "comparison", "check"]`
    - `log`: `["log", "logging", "output", "debug", "error", "warning", "info"]`
    - `env`: `["env", "environment", "variable", "configuration"]`
    - `sleep`: `["sleep", "delay", "pause", "wait", "time"]`
    - `time`: `["time", "timestamp", "unix", "epoch", "clock"]`
  - Update `discover_rust_builtins()` function to use 3-tuple:
    `for (name, desc, kw) in BUILTINS { ... keywords: kw.iter().map(|k| k.to_string()).collect() ... }`
  - Run `cargo test --test discovery` + `cargo clippy -- -D warnings`

  **Must NOT do**: Do NOT modify code outside BUILTINS + `discover_rust_builtins()`. Do NOT change
  descriptions. Do NOT add new builtins.

  **Recommended Agent Profile**: **Category**: `unspecified-high` | **Skills**: []

  **Parallelization**: Wave 2 (parallel with T4, T6-T10) | **Blocks**: T11, T12, T14 | **Blocked
  By**: T1

  **References**: `src/discovery.rs:40-73` (BUILTINS constant), `src/discovery.rs:234-248`
  (`discover_rust_builtins()`), `src/metadata.rs` (ModuleMetadata struct)

  **Acceptance Criteria**:
  - [x] BUILTINS type changed to `&[(&str, &str, &[&str])]`
  - [x] All 17 builtins have 3+ keywords each
  - [x] `cargo test --test discovery` — ALL tests pass
  - [x] `cargo clippy -- -D warnings` passes

  **QA Scenarios:**
  ```
  Scenario: All keyword tests pass (TDD GREEN complete)
    Tool: Bash
    Steps: 1. Run: cargo test --test discovery 2>&1  2. Verify 0 failures
    Expected Result: Exit code 0, all tests pass
    Evidence: .sisyphus/evidence/task-5-builtin-keywords-green.txt
  Scenario: Clippy clean
    Tool: Bash
    Steps: 1. Run: cargo clippy -- -D warnings 2>&1
    Expected Result: Exit code 0, no warnings
    Evidence: .sisyphus/evidence/task-5-clippy-clean.txt
  ```
  **Commit**: YES | `feat(search): enrich builtin keywords in discovery.rs` | Files:
  `src/discovery.rs`

- 6. [x] Site Homepage — site/index.html

  **What to do**:
  - Replace placeholder `site/index.html` with full homepage content:
    - **Hero**: "Assay — One binary to replace your MCP servers" + 9 MB replaces 500 MB tagline
    - **Key Stats**: 9 MB binary, 40+ modules, 15ms query latency, 5ms cold start
    - **Container size comparison table**: Same as README
    - **Two Modes**: Lua scripting + YAML check orchestration with code examples
    - **Quick Start**: Install commands (curl binary, Docker, cargo install)
    - **Navigation**: Links to MCP Comparison, Agent Guides, Module Reference, GitHub, Crates.io
    - **Footer**: MIT license, links to repo/issues/crates
  - Use semantic HTML5. NO JavaScript. Static HTML + CSS only.

  **Must NOT do**: No JavaScript, no analytics, no CSS framework, no additional HTML pages.

  **Recommended Agent Profile**: **Category**: `unspecified-high` | **Skills**: []

  **Parallelization**: Wave 2 (parallel with T4-T5, T7-T10) | **Blocks**: T12 | **Blocked By**: T2

  **References**: `README.md` (content source), `site/style.css` (CSS classes from T2)

  **Acceptance Criteria**:
  - [x] `site/index.html` is full homepage (not placeholder)
  - [x] Contains hero, features, comparison table, install guide
  - [x] Navigation links to all 3 other pages
  - [x] No JavaScript

  **QA Scenarios:**
  ```
  Scenario: Homepage has required content
    Tool: Bash
    Steps: 1. grep -c 'MCP' site/index.html (expect 3+)  2. grep 'mcp-comparison\|agent-guides\|modules' site/index.html (expect all 3 links)
    Expected Result: MCP mentioned, all navigation links present
    Evidence: .sisyphus/evidence/task-6-homepage-content.txt
  ```
  **Commit**: YES | `docs(site): add homepage with features and install guide` | Files:
  `site/index.html`

- 7. [x] Site MCP Comparison Page — 42 Servers Mapped to Assay

  **What to do**:
  - Replace placeholder `site/mcp-comparison.html` with MCP comparison content:
    - **Intro**: "Assay replaces dozens of MCP servers with a single 9 MB binary"
    - **Before/After visual**: `.mcp.json` with 6 servers (70 lines) vs 1 Assay entry (6 lines) —
      MCP-serve vision teaser
    - **"Coming Soon" note**: `assay mcp-serve` under development; currently use `assay context` for
      LLM integration
    - **Comparison Table**: 42 rows with columns: MCP Server name (GitHub link), Stars, Description,
      Assay Equivalent, Coverage (✅ Full | 🟡 Partial | 🟠 Coming Soon | ❌ Out of Scope)
    - **Tier grouping**: Tier 1 (10K+), Tier 2 (2K-10K), Tier 3 (500-2K), Tier 4 (Anthropic
      Reference)
    - **Coverage summary**: "25 domains fully covered, 15+ coming soon, 2 out of scope"
    - **Size comparison**: "10 MCP servers = ~500 MB npm deps + 10 processes. Assay = 9 MB + 1
      process."
  - Use `<table>` elements for comparison (NOT prose per server). Honest coverage qualifiers.

  **Must NOT do**: No prose per server (table only), no false claims about browser/search
  replacement, single page only.

  **Recommended Agent Profile**: **Category**: `unspecified-high` | **Skills**: []

  **Parallelization**: Wave 2 (parallel with T4-T6, T8-T10) | **Blocks**: T12, T13 | **Blocked By**:
  T2

  **References**: This plan's Context section (42 MCP servers with data), `AGENTS.md` (module
  descriptions)

  **Acceptance Criteria**:
  - [x] Table with 42 MCP server entries
  - [x] Coverage qualifiers: ✅ Full, 🟡 Partial, 🟠 Coming Soon, ❌ Out of Scope
  - [x] Before/after `.mcp.json` visual
  - [x] No false claims about browser/search replacement

  **QA Scenarios:**
  ```
  Scenario: Comparison table complete
    Tool: Bash
    Steps: 1. grep -c '<tr>' site/mcp-comparison.html (expect 43+)  2. grep -c 'Coming Soon' site/mcp-comparison.html (expect 10+)
    Expected Result: 42+ entries, 10+ coming soon
    Evidence: .sisyphus/evidence/task-7-mcp-comparison.txt
  ```
  **Commit**: YES | `docs(site): add MCP comparison page mapping 42 servers` | Files:
  `site/mcp-comparison.html`

- 8. [x] Site Agent Integration Guides Page — 5 AI Coding Agents

  **What to do**:
  - Create `site/agent-guides.html` with integration guides for 5 AI coding agents:
    - **Claude Code**: `.mcp.json` in project root — show before (10 npx MCP servers) and after (1
      assay entry)
    - **Cursor**: `.cursor/mcp.json` — identical format to Claude Code (note: shared format)
    - **Windsurf**: `~/.codeium/windsurf/mcp_config.json` — uses `serverUrl` not `url`
    - **Cline**: VS Code extension config — has `autoApprove` field
    - **OpenCode**: `opencode.json` with `mcp` key (not `mcpServers`), uses `{env:VAR}` syntax
  - Each agent section includes:
    - Config file path and format
    - "Today" integration: `assay context <query>` usage with SKILL.md
    - "Coming Soon" teaser: `assay mcp-serve` vision (v0.6.0) showing config snippet
    - Copy-pasteable code blocks
  - Navigation header linking to all 4 pages
  - Consistent styling with `site/styles.css`

  **Must NOT do**: No actual `assay mcp-serve` implementation, no Continue.dev or Aider guides (they
  lack native MCP support), no JS framework.

  **Recommended Agent Profile**: **Category**: `unspecified-high` | **Skills**: []

  **Parallelization**: Wave 2 (parallel with T4-T7, T9-T10) | **Blocks**: T12, T13 | **Blocked By**:
  T2

  **References**:
  - `site/index.html` (from T6) — navigation pattern and styling to match
  - `site/styles.css` (from T2) — shared stylesheet
  - Draft research section "AI Coding Agent Integration" — config formats for all 5 agents
  - Claude Code MCP docs: https://docs.anthropic.com/en/docs/claude-code/mcp
  - `SKILL.md` — current integration pattern for oh-my-opencode/SKILL files
  - `AGENTS.md` — built-in globals and stdlib tables (content reference for "what Assay provides")

  **Acceptance Criteria**:
  - [x] `site/agent-guides.html` exists with all 5 agent sections
  - [x] Each agent has config file path, format, and code block
  - [x] "Today" section shows `assay context` usage
  - [x] "Coming Soon" section shows `assay mcp-serve` vision
  - [x] Navigation links to all 4 site pages

  **QA Scenarios:**
  ```
  Scenario: All 5 agents covered
    Tool: Bash
    Steps: 1. grep -c 'Claude Code\|Cursor\|Windsurf\|Cline\|OpenCode' site/agent-guides.html (expect 5+)
           2. grep -c 'mcp-serve' site/agent-guides.html (expect 2+)
           3. grep -c 'assay context' site/agent-guides.html (expect 3+)
    Expected Result: All 5 agents documented, mcp-serve vision teased, current integration shown
    Evidence: .sisyphus/evidence/task-8-agent-guides.txt
  ```
  **Commit**: YES | `docs(site): add AI agent integration guides for 5 agents` | Files:
  `site/agent-guides.html`

- 9. [x] Site Module Reference Page — All Modules Listed

  **What to do**:
  - Create `site/modules.html` listing ALL 40+ Assay modules:
    - **Rust Builtins section** (17 modules): Table with columns: Module, Functions, Description
      - Group by domain: HTTP/Networking, Serialization, Filesystem, Crypto, Database, WebSocket,
        Templates, Async, Assertions, Logging, Utilities
    - **Stdlib Modules section** (23 modules): Table with columns: Module, Description, Methods
      count
      - Group by domain: Monitoring (prometheus, alertmanager, loki, grafana), K8s/GitOps (k8s,
        argocd, kargo, flux, traefik), Security (vault, openbao, certmanager, eso, dex), Infra
        (crossplane, velero, temporal, harbor), Data (postgres, s3), Identity (zitadel), Utilities
        (healthcheck, unleash)
    - **Custom Modules section**: Brief note about `./modules/` and `~/.assay/modules/` paths
    - Link to `llms.txt` and `llms-full.txt` for AI agent consumption
  - Navigation header linking to all 4 pages
  - Consistent styling with `site/styles.css`

  **Must NOT do**: No method-level API docs (that's what `assay context` is for), no framework.

  **Recommended Agent Profile**: **Category**: `unspecified-high` | **Skills**: []

  **Parallelization**: Wave 2 (parallel with T4-T8, T10) | **Blocks**: T12 | **Blocked By**: T2

  **References**:
  - `AGENTS.md` — canonical list of builtins and stdlib modules with descriptions
  - `src/discovery.rs:40-73` — BUILTINS constant (after T5 enrichment, has keywords)
  - `stdlib/*.lua` — each module's LDoc header (after T4 enrichment, has keywords)
  - `site/index.html` (from T6) — navigation and styling pattern

  **Acceptance Criteria**:
  - [x] `site/modules.html` exists with builtins table (17 entries) and stdlib table (23 entries)
  - [x] Modules grouped by domain
  - [x] Navigation links to all 4 site pages
  - [x] Mentions `assay context` and `assay modules` commands

  **QA Scenarios:**
  ```
  Scenario: All modules listed
    Tool: Bash
    Steps: 1. grep -c 'assay\.' site/modules.html (expect 23+)
           2. grep -c '<tr>' site/modules.html (expect 40+)
           3. grep 'assay context' site/modules.html (expect match)
    Expected Result: 40+ modules listed in tables with domain grouping
    Evidence: .sisyphus/evidence/task-9-modules-page.txt
  ```
  **Commit**: YES | `docs(site): add module reference page listing all modules` | Files:
  `site/modules.html`

- 10. [x] GitHub Actions Deploy Workflow for Cloudflare Pages

  **What to do**:
  - Create `.github/workflows/deploy.yml` with:
    - Trigger: `push` to `main` branch, only when `site/**` files change
    - Uses: `cloudflare/wrangler-action@v3`
    - Secrets needed: `CLOUDFLARE_API_TOKEN`, `CLOUDFLARE_ACCOUNT_ID`
    - Command: `wrangler pages deploy site/ --project-name=assay-docs`
    - Concurrency group to prevent simultaneous deploys
  - Add comment block at top explaining prerequisites:
    - Cloudflare account with `assay.rs` domain
    - Pages project `assay-docs` created
    - GitHub secrets configured
    - Custom domain linked in Cloudflare Pages dashboard
  - Verify `wrangler.toml` (from T2) is compatible with deploy command

  **Must NOT do**: No `wrangler login` (CI uses API token), no DNS changes, no Cloudflare account
  creation steps.

  **Recommended Agent Profile**: **Category**: `quick` | **Skills**: []

  **Parallelization**: Wave 2 (parallel with T4-T9) | **Blocks**: None | **Blocked By**: T2

  **References**:
  - `wrangler.toml` (from T2) — project config
  - `.github/workflows/release.yml` — existing workflow pattern (DO NOT MODIFY, reference style
    only)
  - Cloudflare wrangler-action docs: https://github.com/cloudflare/wrangler-action
  - Draft research section "Cloudflare Pages" — deploy commands and config

  **Acceptance Criteria**:
  - [x] `.github/workflows/deploy.yml` exists
  - [x] Triggers on push to main when `site/**` changes
  - [x] Uses `cloudflare/wrangler-action@v3`
  - [x] References `CLOUDFLARE_API_TOKEN` and `CLOUDFLARE_ACCOUNT_ID` secrets
  - [x] Has prerequisite comment block

  **QA Scenarios:**
  ```
  Scenario: Workflow file valid
    Tool: Bash
    Steps: 1. python3 -c "import yaml; yaml.safe_load(open('.github/workflows/deploy.yml'))" (expect no error)
           2. grep 'cloudflare/wrangler-action' .github/workflows/deploy.yml (expect match)
           3. grep 'CLOUDFLARE_API_TOKEN' .github/workflows/deploy.yml (expect match)
           4. grep 'site/' .github/workflows/deploy.yml (expect match for both trigger path and deploy command)
    Expected Result: Valid YAML, correct action, correct secrets, correct paths
    Evidence: .sisyphus/evidence/task-10-deploy-workflow.txt
  ```
  **Commit**: YES | `ci(deploy): add GitHub Actions workflow for Cloudflare Pages` | Files:
  `.github/workflows/deploy.yml`

- 11. [x] Create llms-full.txt with Enriched Module Documentation

  **What to do**:
  - Create `site/llms-full.txt` — expanded version of `llms.txt` with ALL module docs inlined
  - For each stdlib module:
    - Include full LDoc header (description, keywords)
    - List ALL client methods with signatures and return types
    - Include auth pattern (`client(url, opts)` with auth options)
    - Include error handling pattern
  - For each builtin:
    - List all functions with signatures
    - Include brief usage examples (1-2 lines each)
  - Structure: Same H1/blockquote/H2 as llms.txt, but with content inlined under each entry instead
    of just links
  - Target: ~2000-3000 lines (comprehensive but not excessive)
  - Purpose: RAG bulk ingestion for AI agents that want ALL context at once

  **Must NOT do**: No links to external pages (everything is inlined), no framework.

  **Recommended Agent Profile**: **Category**: `quick` | **Skills**: []

  **Parallelization**: Wave 3 (parallel with T12-T14) | **Blocks**: T14 | **Blocked By**: T3, T4, T5

  **References**:
  - `site/llms.txt` (from T3) — base structure to expand
  - `AGENTS.md` — built-in globals table with function signatures
  - `stdlib/*.lua` (after T4 enrichment) — LDoc headers with enriched keywords and method docs
  - `src/discovery.rs` (after T5 enrichment) — builtin keywords and descriptions
  - LangGraph llms-full.txt example: https://langchain-ai.github.io/langgraph/llms-full.txt

  **Acceptance Criteria**:
  - [x] `site/llms-full.txt` exists
  - [x] First line is `# Assay`
  - [x] All 23 stdlib modules have methods inlined
  - [x] All 17 builtins have function signatures listed
  - [x] File is 1000+ lines (comprehensive)

  **QA Scenarios:**
  ```
  Scenario: llms-full.txt is comprehensive
    Tool: Bash
    Steps: 1. wc -l site/llms-full.txt (expect 1000+)
           2. head -1 site/llms-full.txt (expect "# Assay")
           3. grep -c 'c:health' site/llms-full.txt (expect 5+ — many modules have health methods)
           4. grep -c 'http.get' site/llms-full.txt (expect 1+)
    Expected Result: 1000+ lines, proper H1, method signatures inlined
    Evidence: .sisyphus/evidence/task-11-llms-full.txt
  ```
  **Commit**: YES | `docs(llms): add llms-full.txt with inlined module documentation` | Files:
  `site/llms-full.txt`

- 12. [x] Update README.md for v0.5.1 with Website Links

  **What to do**:
  - Add links section at bottom of README.md (additive only, do NOT rewrite existing content):
    - Website: `https://assay.rs`
    - MCP Comparison: `https://assay.rs/mcp-comparison.html`
    - Agent Integration: `https://assay.rs/agent-guides.html`
    - Module Reference: `https://assay.rs/modules.html`
    - llms.txt: `https://assay.rs/llms.txt`
  - Add brief "v0.5.1: MCP Comparison" section to existing content (2-3 lines max):
    - "See how Assay replaces 42 popular MCP servers:
      [MCP Comparison](https://assay.rs/mcp-comparison.html)"
    - "Integration guides for Claude Code, Cursor, Windsurf, Cline, and OpenCode:
      [Agent Guides](https://assay.rs/agent-guides.html)"

  **Must NOT do**: No rewriting existing README sections, no removing existing content, max 15 new
  lines.

  **Recommended Agent Profile**: **Category**: `quick` | **Skills**: []

  **Parallelization**: Wave 3 (parallel with T11, T13, T14) | **Blocks**: T14 | **Blocked By**:
  T4-T9

  **References**:
  - `README.md` — current file (DO NOT rewrite, only append/insert)
  - `site/mcp-comparison.html` (from T7) — page title and URL to link
  - `site/agent-guides.html` (from T8) — page title and URL to link

  **Acceptance Criteria**:
  - [x] README.md contains `assay.rs` link
  - [x] README.md contains `mcp-comparison.html` link
  - [x] README.md contains `agent-guides.html` link
  - [x] No existing content removed (diff shows only additions)

  **QA Scenarios:**
  ```
  Scenario: README updated with website links
    Tool: Bash
    Steps: 1. grep 'assay.rs' README.md (expect match)
           2. grep 'mcp-comparison' README.md (expect match)
           3. grep 'agent-guides' README.md (expect match)
           4. git diff README.md | grep '^-' | grep -v '^---' | wc -l (expect 0 — no deletions)
    Expected Result: All links present, no content removed
    Evidence: .sisyphus/evidence/task-12-readme-update.txt
  ```
  **Commit**: YES | `docs(readme): update README for v0.5.1 with website links` | Files: `README.md`

- 13. [x] Update SKILL.md with MCP Comparison and Agent Integration

  **What to do**:
  - Append (do NOT rewrite) new sections to `SKILL.md`:
    - **"MCP Replacement" section** (~30 lines): Brief table of top 10 MCP servers Assay replaces,
      with before/after comparison (10 npx servers → 1 assay binary)
    - **"AI Agent Integration" section** (~40 lines): Config snippets for Claude Code, Cursor,
      Windsurf, Cline, OpenCode showing how to add Assay as skill/MCP source
    - **"MCP-Serve Vision" section** (~20 lines): Document the future `assay mcp-serve` subcommand
      and what it will do — serve as MCP server for direct agent integration
  - Max 100 new lines total
  - Use existing SKILL.md formatting conventions (check current file first)

  **Must NOT do**: No rewriting existing SKILL.md content, no removing sections, max 100 new lines,
  no claiming mcp-serve exists yet.

  **Recommended Agent Profile**: **Category**: `quick` | **Skills**: []

  **Parallelization**: Wave 3 (parallel with T11, T12, T14) | **Blocks**: T14 | **Blocked By**: T7,
  T8

  **References**:
  - `SKILL.md` — current file (DO NOT rewrite, only append)
  - `site/mcp-comparison.html` (from T7) — MCP comparison data source
  - `site/agent-guides.html` (from T8) — agent config data source
  - `AGENTS.md` — module descriptions for compact MCP replacement table

  **Acceptance Criteria**:
  - [x] SKILL.md contains "MCP Replacement" section
  - [x] SKILL.md contains "AI Agent Integration" section
  - [x] SKILL.md contains "MCP-Serve Vision" section
  - [x] No existing content removed
  - [x] Max 100 new lines added

  **QA Scenarios:**
  ```
  Scenario: SKILL.md updated with new sections
    Tool: Bash
    Steps: 1. grep 'MCP Replacement' SKILL.md (expect match)
           2. grep 'AI Agent Integration' SKILL.md (expect match)
           3. grep 'MCP-Serve' SKILL.md (expect match)
           4. grep -c 'mcp-serve' SKILL.md (expect 2+ — mentioned in vision section)
           5. git diff SKILL.md | grep '^-' | grep -v '^---' | wc -l (expect 0 — no deletions)
    Expected Result: All 3 sections present, no deletions
    Evidence: .sisyphus/evidence/task-13-skill-update.txt
  ```
  **Commit**: YES | `docs(skill): append MCP comparison and agent integration to SKILL.md` | Files:
  `SKILL.md`

- 14. [x] Version Bump to 0.5.1

  **What to do**:
  - Update `Cargo.toml`: `version = "0.5.0"` → `version = "0.5.1"`
  - Run `cargo check` to regenerate `Cargo.lock`
  - Run full verification suite:
    - `cargo clippy --tests -- -D warnings` (must pass)
    - `cargo test` (must pass — all existing + new keyword tests)
    - `cargo build --release` (must produce binary < 12 MB)
  - Verify binary works: `./target/release/assay --version` should show 0.5.1

  **Must NOT do**: No other Cargo.toml changes (no new dependencies, no feature changes), no
  modifying any other source files.

  **Recommended Agent Profile**: **Category**: `quick` | **Skills**: []

  **Parallelization**: Wave 3 (sequential — last task before final wave) | **Blocks**: F1-F4 |
  **Blocked By**: T4-T13 (all implementation tasks)

  **References**:
  - `Cargo.toml` — current version field
  - `Cargo.lock` — will be auto-updated by cargo check

  **Acceptance Criteria**:
  - [x] `Cargo.toml` shows `version = "0.5.1"`
  - [x] `Cargo.lock` updated
  - [x] `cargo clippy --tests -- -D warnings` passes
  - [x] `cargo test` passes (all tests including new keyword tests)
  - [x] `cargo build --release` produces binary < 12 MB
  - [x] `./target/release/assay --version` shows 0.5.1

  **QA Scenarios:**
  ```
  Scenario: Version bumped and all checks pass
    Tool: Bash
    Steps: 1. grep '^version = "0.5.1"' Cargo.toml (expect match)
           2. cargo clippy --tests -- -D warnings 2>&1 (expect exit code 0)
           3. cargo test 2>&1 (expect exit code 0)
           4. cargo build --release 2>&1 (expect exit code 0)
           5. ls -la target/release/assay | awk '{print $5}' (expect < 12582912 = 12MB)
           6. ./target/release/assay --version (expect contains "0.5.1")
    Expected Result: Version 0.5.1, all checks pass, binary under 12 MB
    Evidence: .sisyphus/evidence/task-14-version-bump.txt

  Scenario: New keyword search tests pass (TDD GREEN verification)
    Tool: Bash
    Steps: 1. cargo test --test discovery 2>&1 (expect all pass)
           2. Count total tests (expect 580+ existing + ~15 new)
    Expected Result: Exit code 0, all tests pass including keyword gap tests from T1
    Failure Indicators: Any test failure — means keyword enrichment in T4/T5 was incomplete
    Evidence: .sisyphus/evidence/task-14-tdd-green-verification.txt
  ```
  **Commit**: YES | `chore(release): bump version to 0.5.1` | Files: `Cargo.toml`, `Cargo.lock`

---

## Final Verification Wave (MANDATORY — after ALL implementation tasks)

> 4 review agents run in PARALLEL. ALL must APPROVE. Rejection → fix → re-run.

- [x] F1. **Plan Compliance Audit** — `oracle` Read the plan end-to-end. For each "Must Have":
      verify implementation exists (read file, run command). For each "Must NOT Have": search
      codebase for forbidden patterns — reject with file:line if found. Check evidence files exist
      in .sisyphus/evidence/. Compare deliverables against plan. Output:
      `Must Have [N/N] | Must NOT Have [N/N] | Tasks [N/N] | VERDICT: APPROVE/REJECT`

- [x] F2. **Code Quality Review** — `unspecified-high` Run `cargo clippy --tests -- -D warnings` +
      `cargo test`. Review changed files in `src/discovery.rs` for: `as any`/`@ts-ignore`
      equivalents, empty error handling, commented-out code, unused imports. Check `stdlib/*.lua`
      changes are ONLY on `@keywords` line. Verify no AI slop: excessive comments, over-abstraction,
      generic names. Output:
      `Build [PASS/FAIL] | Lint [PASS/FAIL] | Tests [N pass/N fail] | Files [N clean/N issues] | VERDICT`

- [x] F3. **Real Manual QA** — `unspecified-high` Build the binary (`cargo build --release`). Run
      these searches and verify results:
  - `./target/release/assay context "jwt"` → must find crypto
  - `./target/release/assay context "tls certificate"` → must find certmanager
  - `./target/release/assay context "backup disaster recovery"` → must find velero
  - `./target/release/assay context "feature flags toggle"` → must find unleash
  - `./target/release/assay context "oidc identity sso"` → must find dex, zitadel
  - `./target/release/assay context "grafana"` → must still find grafana first (regression) Verify
    `site/llms.txt` follows spec (H1, blockquote, H2 sections). Verify all 4 HTML pages exist and
    contain expected content. Save to `.sisyphus/evidence/final-qa/`. Output:
    `Search [N/N pass] | Website [N/N] | llms.txt [PASS/FAIL] | VERDICT`

- [x] F4. **Scope Fidelity Check** — `unspecified-high` For each task: read "What to do", read
      actual diff (git log/diff). Verify 1:1 — everything in spec was built, nothing beyond spec was
      built. Check "Must NOT do" compliance: no mcp-serve code, no new stdlib modules, no JS
      frameworks, max 4 HTML pages. Flag unaccounted changes. Output:
      `Tasks [N/N compliant] | Contamination [CLEAN/N issues] | Unaccounted [CLEAN/N files] | VERDICT`

---

## Commit Strategy

| Task(s) | Commit Message                                                         | Key Files                      |
| ------- | ---------------------------------------------------------------------- | ------------------------------ |
| T1      | `test(search): add TDD RED tests for keyword enrichment gaps`          | `tests/discovery.rs`           |
| T2      | `chore(site): scaffold static website directory structure`             | `site/*`, `wrangler.toml`      |
| T3      | `docs(llms): add llms.txt for LLM agent context traversal`             | `site/llms.txt`, `llms.txt`    |
| T4      | `feat(search): enrich @keywords on all 23 stdlib modules`              | `stdlib/*.lua`                 |
| T5      | `feat(search): enrich builtin keywords in discovery.rs`                | `src/discovery.rs`             |
| T6      | `docs(site): add homepage with features and install guide`             | `site/index.html`              |
| T7      | `docs(site): add MCP comparison page mapping 42 servers`               | `site/mcp-comparison.html`     |
| T8      | `docs(site): add AI agent integration guides for 5 agents`             | `site/agent-guides.html`       |
| T9      | `docs(site): add module reference page listing all modules`            | `site/modules.html`            |
| T10     | `ci(deploy): add GitHub Actions workflow for Cloudflare Pages`         | `.github/workflows/deploy.yml` |
| T11     | `docs(llms): add llms-full.txt with inlined module documentation`      | `site/llms-full.txt`           |
| T12     | `docs(readme): update README for v0.5.1 with website links`            | `README.md`                    |
| T13     | `docs(skill): append MCP comparison and agent integration to SKILL.md` | `SKILL.md`                     |
| T14     | `chore(release): bump version to 0.5.1`                                | `Cargo.toml`, `Cargo.lock`     |

---

## Success Criteria

### Verification Commands

```bash
cargo clippy --tests -- -D warnings   # Expected: no warnings
cargo test                              # Expected: all pass (580+ existing + ~15 new)
cargo build --release                   # Expected: binary < 12 MB
./target/release/assay context "jwt"    # Expected: finds crypto module
./target/release/assay context "tls"    # Expected: finds certmanager module
./target/release/assay context "backup" # Expected: finds velero module
ls site/index.html site/llms.txt        # Expected: files exist
head -1 site/llms.txt                   # Expected: "# Assay"
grep '^version = "0.5.1"' Cargo.toml   # Expected: match
```

### Final Checklist

- [x] All "Must Have" present
- [x] All "Must NOT Have" absent
- [x] All tests pass (existing + new keyword tests)
- [x] Binary size < 12 MB
- [x] Website has exactly 4 HTML pages
- [x] llms.txt follows Jeremy Howard spec
- [x] Version is 0.5.1
