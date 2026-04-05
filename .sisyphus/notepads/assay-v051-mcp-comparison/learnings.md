# Learnings — assay-v051-mcp-comparison

## [2026-02-23] Session: ses_3786d3a07ffeJguiDn8Lotl6IG

### From v0.5.0 Inherited Wisdom

- FTS5 bm25() returns NEGATIVE scores — negate when converting to SearchResult
- BM25 tokenizer: `fn tokenize(text: &str)` splits on non-alphanumeric EXCEPT underscore,
  lowercases, filters len ≤ 1. So "jwt_sign" is ONE token, "jwt" alone won't match it.
- BUILTINS constant current type: `&[(&str, &str)]` (name, description) — MUST change to
  `&[(&str, &str, &[&str])]` for T5
- `discover_rust_builtins()` at src/discovery.rs:234-248 sets keywords to `vec![name.to_string()]`
- `cargo clippy -- -D warnings` is mandatory — warnings are errors
- HTML/CSS/JS not covered by existing dprint config — known limitation, don't add to dprint includes

### Key Files

- `src/discovery.rs:40-73` — BUILTINS constant
- `src/discovery.rs:234-248` — `discover_rust_builtins()`
- `src/search.rs:73-78` — BM25 tokenizer
- `stdlib/*.lua` — line 3 is `--- @keywords` (ONLY this line changes in T4)
- `tests/discovery.rs` — existing test patterns

### Known Gotchas for TDD

- "hash" already matches crypto via description → don't use as TDD RED term
- "websocket" matches ws description → don't use
- "logging" matches log description → don't use
- Use "jwt", "letsencrypt", "backup", "toggle", "crd", "rollout", "encryption", "pipeline", "seal"
- Before writing each RED test — verify the term genuinely fails by checking tokenizer rules against
  existing description/name/keywords

## [2026-02-23] Task 2: Site Scaffold

### Completed

- Created `site/` directory with 7 files:
  - `index.html`, `mcp-comparison.html`, `agent-guides.html`, `modules.html` (4 placeholder pages)
  - `style.css` (responsive, dark/light theme, code/table styling)
  - `_headers` (Cloudflare security headers + llms.txt Content-Type)
  - `_redirects` (2 shortcuts: /github, /crates)
- Created `wrangler.toml` at repo root
- All HTML files have consistent structure: DOCTYPE, head with meta/link, nav, main, footer
- CSS features: prefers-color-scheme dark/light, max-width 900px, code/pre/table styling, responsive
- Commit: `313d051 chore(site): scaffold static website directory structure`

### Key Decisions

- Color scheme: dark #0d1117, accent #e6662a (orange) matching Assay branding
- No frameworks, no npm, no JavaScript — pure HTML/CSS
- Placeholder content "Content coming in next task" for all 4 pages
- All pages link to single `style.css` (verified: 4/4 HTML files)
- Cloudflare Pages config ready for deployment

### Files Created

- `/root/code/assay/site/style.css` (283 lines, 4228 bytes)
- `/root/code/assay/site/_headers` (12 lines, 299 bytes)
- `/root/code/assay/site/_redirects` (2 lines, 106 bytes)
- `/root/code/assay/site/index.html` (30 lines, 1057 bytes)
- `/root/code/assay/site/mcp-comparison.html` (30 lines, 982 bytes)
- `/root/code/assay/site/agent-guides.html` (30 lines, 1029 bytes)
- `/root/code/assay/site/modules.html` (30 lines, 992 bytes)
- `/root/code/assay/wrangler.toml` (3 lines, 86 bytes)

### Evidence

- Saved to `.sisyphus/evidence/task-2-site-structure.txt`
- All 8 files verified with `ls -la`
- Stylesheet link verification: `grep -l 'style.css' site/*.html | wc -l` → 4

## [2026-02-23] Task 3: llms.txt Spec Implementation

### Completed

- Created `site/llms.txt` following Jeremy Howard spec exactly
- Copied to repo root as `llms.txt` (identical files)
- 9 H2 sections (Getting Started, Built-in Globals, Monitoring & Observability, Kubernetes & GitOps,
  Security & Identity, Infrastructure, Data & Storage, Feature Flags & Utilities, Optional)
- 6-line blockquote with LLM agent instructions (assay context, client pattern, error handling,
  pcall)
- All 17 builtins referenced: http, json, yaml, toml, fs, crypto, base64, regex, db, ws, template,
  async, assert, log, env, sleep, time
- All 23 stdlib modules referenced: prometheus, alertmanager, loki, grafana, k8s, argocd, kargo,
  flux, traefik, vault, openbao, certmanager, eso, dex, crossplane, velero, temporal, harbor,
  healthcheck, s3, postgres, zitadel, unleash
- 48 total links (under limit of 60)
- Commit: `docs(llms): add llms.txt for LLM agent context traversal`

### Key Patterns

- Jeremy Howard spec: H1 first, blockquote optional but important, H2 sections with
  `- [Name](url): description` entries
- `## Optional` section at bottom — LLM tools may skip for shorter context
- Blockquote emphasizes: `assay context <query>` as key integration point for LLM agents
- All links use `blob/main/` (not versioned) and `assay.rs/modules.html` anchors
- Both files must be IDENTICAL — site/llms.txt for Cloudflare Pages, llms.txt for GitHub raw access

### Evidence

- `.sisyphus/evidence/task-3-llms-txt-spec.txt` — verification output

## [2026-02-23] Task 1: TDD RED Tests for Keyword Enrichment

### Completed

15 new `test_search_keyword_*` tests — all FAIL (RED) as expected 4 new `test_search_regression_*`
tests — all PASS (GREEN) as expected 9 existing tests unchanged and passing Commit:
`test(search): add TDD RED tests for keyword enrichment gaps`

### Key Discoveries

Auto-functions from stdlib modules ARE indexed and match terms unexpectedly:

- `jwt` matched crypto (auto_functions likely include jwt-related funcs)
- `crd` matched k8s, `rollout` matched k8s, `seal` matched vault
- `snapshot` matched velero, `toggle` matched unleash, `helm` matched flux Original task terms that
  pass (NOT suitable for RED tests):
- backup (in velero description), pipeline (in kargo description)
- oidc (in dex keywords), disaster (disaster-recovery splits to disaster+recovery) `assay.openbao`
  ranks HIGHER than `assay.vault` for query "vault" — openbao is a vault alias and references vault
  extensively, inflating its score Vault regression test changed to `contains` check (not
  first-result) because of openbao

### Replacement RED Test Terms (verified to FAIL)

webhook, request, endpoint → http letsencrypt, ssl → certmanager password, encryption, rotation →
vault/certmanager cicd → argocd/flux metric → prometheus observability → prometheus/grafana
terraform → crossplane deploy → k8s failover → velero docker → harbor

### Tokenizer Verification Pattern

1. Check module's `@keywords` line (line 3 of stdlib/*.lua)
2. Check module's `@description` line (line 2)
3. Check BUILTINS constant descriptions (src/discovery.rs:40-73)
4. Check auto_functions from `function c:name()` patterns in .lua files
5. Apply tokenizer: split on non-alphanumeric-except-underscore, lowercase, len>1
6. BM25 uses EXACT token matching — "metric" ≠ "metrics", "jwt" ≠ "jwt_sign"

## [2026-02-23] Task 6: Homepage Content

### Completed

- Replaced placeholder `site/index.html` with 190-line production homepage 8 sections: header/nav,
  hero, stats (4 cards), what-is-assay (2 modes), container size table, MCP replacement pitch,
  install guide, footer 10 MCP references (requirement was 3+), 9 nav link matches, 0 script tags
  All inline styles use CSS custom properties (var(--accent), var(--border), etc.) for theme
  consistency GitHub link added to nav (5th item) per spec Commit:
  `e78ff10 docs(site): add homepage with features and install guide`

### Key Patterns

- CSS has no button/card/hero classes — used inline styles with CSS custom properties Grid layout
  `repeat(auto-fit, minmax(180px, 1fr))` works well for stat cards on mobile Accent row in table via
  inline `style="font-weight: 600; color: var(--accent);"` HTML entities for arrows: `&rarr;`,
  `&darr;`; dashes: `&mdash;`

## Task 5: Builtin Keywords (2026-02-23)

BUILTINS changed from `&[(&str, &str)]` to `&[(&str, &str, &[&str])]` — 3rd element is keyword slice
`discover_rust_builtins()` unpacks via `for &(name, description, kw) in BUILTINS` Keywords
converted: `kw.iter().map(|k| k.to_string()).collect()` 3 target tests fixed: webhook→http,
request→http, endpoint→http all now PASS 8 remaining failures are stdlib .lua keyword issues (vault,
certmanager, velero, harbor, crossplane) — separate task Clippy clean, commit: `40c8e56`

## Task 4: Stdlib @keywords Enrichment

All 23 stdlib/*.lua files have `--- @keywords` on line 3 (LDoc format) Keywords are comma-space
separated: `--- @keywords k1, k2, k3` BM25 search indexes keywords from this line; adding
synonyms/related terms dramatically improves recall The edit tool requires fresh LINE#ID refs after
reading — first read returns stale IDs from tool cache All 28 discovery tests pass (15 keyword gap +
4 regression + 9 baseline) Only line 3 changes needed per file — surgical edits confirmed via
`git diff --unified=0`

## [2026-02-23] Task 10: Deploy Workflow

### Completed

- Created `.github/workflows/deploy.yml` (46 lines, 1429 bytes)
- Triggers on push to `main` when `site/**` or `wrangler.toml` changes
- Uses `cloudflare/wrangler-action@v3` with API token and account ID secrets
- Concurrency group prevents simultaneous deploys (`cancel-in-progress: false`)
- Prerequisite comment block documents manual setup steps (Cloudflare account, Pages project,
  secrets, custom domain)
- YAML validation: `python3 -c "import yaml; yaml.safe_load(open('.github/workflows/deploy.yml'))"`
  passes
- Evidence: `.sisyphus/evidence/task-10-deploy-workflow.txt`
- Commit: `da545bc ci(deploy): add GitHub Actions workflow for Cloudflare Pages`

### Key Patterns

- Workflow name: "Deploy to Cloudflare Pages"
- Job name: "Deploy site to Cloudflare Pages"
- Permissions: `contents: read`, `deployments: write`
- Deploy command: `wrangler pages deploy site/ --project-name=assay-docs`
- Project name in wrangler.toml is `assay-rs` but Cloudflare Pages project is `assay-docs` (can
  differ)
- Paths trigger: `site/**` and `wrangler.toml` (both must change to trigger)
- Concurrency: `cancel-in-progress: false` ensures deploys finish even if new push comes in

### Files Created

- `.github/workflows/deploy.yml` (46 lines)

### Evidence

- `.sisyphus/evidence/task-10-deploy-workflow.txt` — YAML validation + grep output

## [2026-02-23] Task 8: Agent Integration Guides

### Completed

- Replaced placeholder `site/agent-guides.html` with full integration guide (292 lines) 5 agent
  sections: Claude Code, Cursor, Windsurf, Cline, OpenCode Each agent has "Today" section (assay
  context usage) + "Coming Soon" section (mcp-serve config) Quick Reference table at bottom with
  config paths and formats Common Workflow section with universal client pattern Commit:
  `7ab0c85 docs(site): add AI agent integration guides for 5 agents`

### Key Agent Config Differences

- Claude Code: `.mcp.json` with `mcpServers` key Cursor: `.cursor/mcp.json` — identical format to
  Claude Code Windsurf: `~/.codeium/windsurf/mcp_config.json` — uses `serverUrl` (HTTP transport),
  global only Cline: VS Code settings — has `autoApprove` field for trusted tool calls OpenCode:
  `opencode.json` — uses `mcp` key (not `mcpServers`)

### Verification Counts

- `mcp-serve` references: 8 (required 5+) `assay context` references: 29 (required 5+) All 5 agent
  sections with both Today and Coming Soon subsections

## [2026-02-23] Task 7: MCP Comparison Page

### Completed

- Replaced placeholder `site/mcp-comparison.html` with full content (11,566 bytes) 42 MCP server
  entries across 4 tiers (10K+, 2K-10K, 500-2K, Anthropic/Model Reference) 4 tier separator rows in
  table for visual grouping Coverage qualifiers: 28 Full, 4 Partial, 12 Coming Soon, 5 Out of Scope
  Before/after `.mcp.json` visual showing 10 servers → 1 Assay entry Commit:
  `ee852a7 docs(site): add MCP comparison page mapping 42 servers`

### Key Patterns

- `grep -c '<tr>'` only matches `<tr>` not `<tr style=...>` — tier separator rows use inline style
  so they DON'T count toward the `<tr>` grep. 42 data + 1 thead = 43 matches. "Coming Soon"
  (case-sensitive) appears in: 8 table cells + h3 heading + code block note + coverage key legend +
  summary callout = 12 total lines Callout boxes use inline styles on `<p>` since style.css has no
  `.callout` class `assay mcp-serve` is v0.6.0 future — clearly marked as Coming Soon throughout No
  JavaScript, no CSS framework — pure HTML/CSS matching site conventions
