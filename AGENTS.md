# AGENTS.md

## Skills & Rules

Key coding practices for this project:

- **autonomous-workflow** — Proposal-first development, decision authority, commit hygiene
- **code-quality** — Warnings-as-errors, no underscore prefixes, test coverage, type safety

## Repository sync hygiene

When feature work has merged, switch back to `main`, fetch upstream, and fast-forward local `main`
to `origin/main` before starting or debugging follow-up work. Do not leave merged feature branches
or locally-applied copies of merged commits as a dirty worktree; that makes later sessions confuse
released upstream changes with unfinished local work.

Use the main checkout for active work. Do not create extra git worktrees for this repo unless the
human operator explicitly asks for parallel checkouts; one workstream in the main worktree is
enough.

## Self-hosted CI runner

The GitHub Actions runner named `assay-runner-eda` runs inside the `assay-runner` systemd-nspawn
machine on the EDA server. Treat its services as persistent, not ephemeral. In particular, its
Postgres service is a long-lived system service used by Linux CI via `TEST_DATABASE_URL`; tests must
drop any `assay_test_*` databases they create, and CI should keep an `always()` cleanup guard for
that prefix. Keep Cargo build output on the runner cache mount by setting `CARGO_TARGET_DIR` to
`/var/cache/runner-work/assay-target` for self-hosted CI; do not put recurring build artifacts under
`/home/runner/actions-runner/_work/.../target`.

## Library hygiene: no application-domain leakage

**Assay is a general-purpose library. It must have zero knowledge of any specific application that
uses it.** When writing or modifying any code, test, comment, doc, changelog entry, commit message,
or PR description in this repo, never reference:

- Specific consumer applications by name (e.g. "Command Center", "hydra-login", or any internal
  product)
- Specific deployments, environments, or company-specific URLs (e.g.
  `*.dev.simons.disw.siemens.com`, internal cluster names)
- Company- or project-specific role names, namespaces, client IDs, or resource names that only make
  sense in one consumer's context
- The user's organisation, team, or internal project naming conventions

Use generic placeholder names instead:

- Client IDs: `example-app`, `demo-client`, `app-1`
- Hostnames: `example.com`, `app.example.com`, `hydra.example.com`
- Role objects: `app:role-a`, `namespace1:role-a`, `app:admin`
- Project IDs: `demo-project`, `project-1`
- Workflow names: `MyWorkflow`, `my-queue`

When motivating a new feature in a CHANGELOG entry, commit message, or PR description, describe
**the OIDC/Kubernetes/HTTP scenario it enables**, not **the specific consumer that asked for it**.
The library should read the same to a stranger who has never heard of any of assay's consumers as it
does to someone who works on one of them every day.

This applies to all files in the repo: `stdlib/`, `src/`, `tests/`, `*.md`, `*.html`,
`CHANGELOG.md`, and any commit/PR text. The exceptions are the copyright holder's name in
`LICENSE`/`NOTICE`/`CLA.md` and deployment material for Assay's own public first-party properties.
First-party deployment details must stay under `service/` or its dedicated deployment workflow and
may name only public `*.assay.rs` surfaces. They must not introduce knowledge of a downstream
consumer, private environment, or organisation into the reusable crates.

## Failing tests: never rewrite, always escalate

**If a test is failing, the default is to fix the code under test — NOT the test.** Modifying a
failing test (relaxing an assertion, changing an expected value, loosening a regex, adding
`#[ignore]`, deleting the test, or narrowing its scope) requires **explicit approval from the human
operator** before committing. No exceptions — not "while debugging", not "just to unblock CI", not
"the test was wrong anyway". Ask, wait for a clear yes, then change.

Why this rule exists: AI agents have a known failure mode where a failing assertion gets "solved" by
weakening the check until it passes, masking the real regression behind green CI. The test is a
specification the code is supposed to meet; weakening it silently changes the specification without
the human realising the behavioural guarantee has shrunk.

When a test fails, the expected agent workflow is:

1. Report the failure verbatim (test name + failure message + relevant file:line).
2. Explain what the test is checking and what the code actually did.
3. Propose a fix — which is usually in the code under test, occasionally in the test (if the test
   itself has a real bug), but never "just delete the assertion".
4. Wait for approval before editing anything test-related.

If the test genuinely has a bug (e.g. wrong assertion, stale fixture, pre-existing flake), that's
also a conversation, not a silent rewrite. Flag it, describe the bug, propose the correction, wait
for confirmation. Tests are the repo's behavioural contract; changes to them need the same care as
changes to published API surface.

## Release docs checklist

Every release (patch, minor, or major) must update **all** of the following before the PR merges —
never ship a release that only touches source + CHANGELOG. The site, llms.txt, and sub-crate
manifests drift silently if you forget them, and agents downstream (including future-you) see stale
information.

### Version bumps

- `crates/assay/Cargo.toml` — bump `[package].version` for the `assay-lua` binary crate. The root
  `Cargo.toml` is a virtual workspace with no version of its own.
- **`crates/*/Cargo.toml`** — each sub-crate follows **independent semver tied to its own public
  Rust API**, not the binary's version. Today that's `crates/assay-workflow/Cargo.toml`. Skip this
  and the crates.io publish step of the release workflow fails at tag push time (the crate version
  already exists on the index).
- When a sub-crate version bumps, the workspace root `Cargo.toml` dependency spec must bump to match
  (e.g. `assay-workflow = { ..., version = "0.2" }`). Cargo's lockfile will regenerate on the next
  build; commit the resulting `Cargo.lock` update.

### Pre-1.0 semver: patch bumps by default

**Every release bumps the patch digit. Never jump the minor without an explicit conversation.**

Rationale: assay is pre-1.0. Both the binary (`assay-lua`) and the engine library (`assay-workflow`)
are young and do not have a stable promised API yet. A minor bump on a 0.x crate conventionally
signals a breaking change to downstream Cargo consumers — which matters once there are downstream
consumers. While there aren't, a patch bump is both sufficient and keeps Cargo dep specs
(`version = "0.1"` = `^0.1`, `version = "0.11"` = `^0.11`) stable across the upgrade.

- `assay-lua` (binary): always patch bump (`0.11.4 → 0.11.5`) unless the user says otherwise.
- `assay-workflow` (engine sub-crate): always patch bump (`0.1.2 → 0.1.3`) unless the user says
  otherwise.
- **Don't pick a minor bump unilaterally.** If a change looks like it justifies one (e.g. a public
  enum becomes a struct, a trait signature changes), surface the tradeoff to the user and ask before
  touching the version field. Discussion first, edit second.

The binary version and sub-crate version are independent. It's normal for `assay-lua 0.11.5` to ship
`assay-workflow 0.1.3`; they have unrelated bump cadences.

### Non-source files

- `CHANGELOG.md` — new section at the top; describe the OIDC/Kubernetes/HTTP scenario enabled, not a
  specific consumer.
- `docs/modules/*.md` — any module whose surface changed.
- `README.md`, `SKILL.md`, `AGENTS.md`, `skills/assay/SKILL.md` — auth / CLI / API tables if
  touched.
- `llms.txt` (root) — one-liner module descriptors.
- `site/pages/index.html` — the release banner (`v0.x.y` tag + copy) and any version badge on
  feature cards (e.g. the Workflow Engine card).
- `site/static/llms.txt` — site's static teaser; keep it reasonably fresh (not auto-generated from
  the root `llms.txt`).
- Any site page that references the previous version in prose or code (e.g. the `mise` / `crates.io`
  install snippets on `index.html`).

### Verification before opening the PR

```sh
# No stale version references in shipping content:
grep -rn "v0.PREVIOUS" . --include="*.md" --include="*.html" --include="*.toml" --include="*.txt"

# Every crate manifest version field matches what you intend to publish:
grep -E '^version = ' Cargo.toml crates/*/Cargo.toml

# Dep spec in the root Cargo.toml tracks the sub-crate bump:
grep -A0 'assay-workflow' Cargo.toml | grep 'version ='
```

Only matches that should remain for the first grep are historical CHANGELOG entries and "introduced
in vX.Y.Z" feature markers. The second and third greps exist because v0.11.4's crates.io publish
failed when the sub-crate version was missed — do not skip them.

### Regenerate README and AGENTS.md when adding/removing modules

Two files carry a generated stdlib table between `<!-- BEGIN STDLIB TABLE -->` and
`<!-- END STDLIB TABLE -->` markers, both written by `assay site/build.lua` — but from different
sources:

- `README.md` — from the `category:` frontmatter of `docs/modules/*.md`.
- `AGENTS.md` (the "Stdlib Modules" section below) — from the `--- @module` / `--- @description` /
  `--- @category` doc blocks at the top of each `crates/assay/stdlib/**/*.lua`, which is the only
  source that names every requirable module. Change a module's doc block, or `stdlib_category_order`
  in `site/build.lua`, not the table.

The generated block is fenced with `<!-- dprint-ignore-start -->` / `<!-- dprint-ignore-end -->`:
dprint's markdown formatter pads table columns, which would fight the generator on every run. Leave
the fences in place — the rest of this file is formatted normally.

CI enforces both with `git diff --exit-code` after running the generator, so any new/renamed/removed
module requires a regen commit:

```sh
cargo build -p assay-lua --bin assay         # if not already built
./target/release/assay site/build.lua
git add README.md AGENTS.md && git commit
```

A pre-commit hook lives at `.githooks/pre-commit` to catch this locally — it runs the regenerator
whenever a staged change touches `docs/modules/` or `crates/assay/stdlib/` and fails the commit if
either file ends up out of sync. Enable it once per clone:

```sh
git config core.hooksPath .githooks
```

Bypass with `git commit --no-verify` only when you're explicitly committing the regen as a
follow-up.

## PR merge process

Follow this sequence for every PR merge into `main`, even as an admin who could bypass branch
protection. Branch protection is the enforcement layer, the process below is the discipline layer —
protection is the last line of defence, not the only one.

### 0. Create the PR with the right assignee

Every PR in this repo is owned by the repo owner — `developerinlondon`. Always pass
`--assignee developerinlondon` (or `--assignee @me` when you're authenticated as that user) when
running `gh pr create`:

```sh
gh pr create --title "..." --body "..." --assignee developerinlondon
```

If you forget at creation time, fix it immediately with:

```sh
gh pr edit <PR> --add-assignee developerinlondon
```

### 1. Wait for CI to settle fully

```sh
gh pr checks <PR> --watch
```

**Do not use `--required`.** On repos without required-check protection, `--required` prints "no
required checks reported" and exits immediately, letting a merge-chain proceed while checks are
still running. Even on protected repos, `--required` only watches the required contexts — other CI
in the same workflow may still be running. Plain `--watch` blocks on every check.

### 2. Never merge through red

If any check ends non-green, inspect before deciding:

```sh
gh run view <run-id>
gh api repos/developerinlondon/assay/actions/jobs/<job-id>/logs
```

- **Infrastructure flakes** — runner `No space left on device`, `Runner failed to dispatch`,
  transient network timeouts pulling crates, GitHub outages — re-run the failed job on a fresh
  runner:

  ```sh
  gh run rerun <run-id> --failed
  gh pr checks <PR> --watch   # wait again
  ```

  Merging through an infra flake is gambling that the flake hides nothing. Occasionally it does.

- **Code failures** — test assertion, clippy error, compile error — fix in a new commit on the same
  branch, push, wait for CI. Never merge a PR with a known code failure on the promise of a
  "follow-up fix" — the follow-up lives in the same PR.

### 3. Squash-merge with branch delete

```sh
gh pr merge <PR> --squash --delete-branch
```

Prefer `--squash` over `--merge` so `main` history stays one-commit-per-PR and greppable. Let
`--delete-branch` clean up the remote branch; don't leave stale feature branches around.

### 4. Verify the post-merge CI run

The `push` trigger on `ci.yml` fires a fresh CI run on the merge commit itself. Occasionally a merge
commit behaves differently than the PR HEAD (e.g. silent conflict with concurrent merges, unrelated
upstream drift). Confirm it lands green before triggering anything downstream (tag push, release
workflow, deploy):

```sh
gh run list --workflow=ci.yml --branch main --limit 3
```

If the first entry isn't `success`, treat `main` as broken and fix before anything else. Don't tag a
release off a main that isn't green.

### Why this exists

v0.11.5 was merged with one CI check still in progress because the merge-chain used
`gh pr checks 46 --watch --required` on a repo that had no required checks configured. The watch
command exited immediately, the auto-merge fired, and the still-running ubuntu check job later
errored on a runner infrastructure flake (`No space left on device`). The post-merge CI on `main`
was green, so nothing broke — but the only reason was luck. Branch protection on `main` was added
retroactively (see "PR merge process → 1" above for the correct watch invocation that would have
caught this).

## What is Assay

General-purpose enhanced Lua runtime. Single ~11 MB static binary with batteries included: HTTP
client/server, JSON/YAML/TOML, crypto, database, WebSocket, filesystem, shell execution, process
management, async, and the embedded stdlib modules for infrastructure services (Kubernetes,
Prometheus, Vault, ArgoCD, etc.) and AI agent integrations (OpenClaw, GitHub, Gmail, Google
Calendar). The [Stdlib Modules](#stdlib-modules) table below is the generated list — it carries the
count, so no other sentence in this file should name one.

Use cases:

- **Standalone scripting** — system automation, CI/CD tasks, file processing
- **Embedded runtime** — other Rust services embed assay as a library (`pub mod lua`)
- **Kubernetes Jobs** — replaces 50–250 MB Python/Node/kubectl containers (~11 MB image)
- **Infrastructure automation** — GitOps hooks, health checks, service configuration

- **Repo**: `deepbrain-oss/assay` on [agentgit.org](https://agentgit.org) is the primary; every PR
  goes there. [github.com/developerinlondon/assay](https://github.com/developerinlondon/assay)
  is a push mirror: issues only, PRs are auto-closed, releases publish from it.
- **Image**: `ghcr.io/developerinlondon/assay:latest` (~11 MB compressed)
- **Crate**: [crates.io/crates/assay-lua](https://crates.io/crates/assay-lua)
- **Stack**: Rust (2024 edition), Tokio, Lua 5.5 (mlua), reqwest, clap, axum

## Three Modes

```bash
assay script.lua     # Lua mode — run script with all builtins
assay checks.yaml    # YAML mode — structured checks with retry/backoff/parallel
assay serve          # Workflow engine mode — REST+SSE API, dashboard, CLI
```

### Workflow engine mode — short overview

Full reference: `docs/modules/workflow.md`. One binary runs the engine (`assay serve`), the CLI that
drives it (`assay workflow/schedule/namespace/worker/queue/completion …`), and the Lua client
(`require("assay.workflow")`) for worker processes and management scripts.

```bash
assay serve --backend postgres://... --port 8080       # engine (multi-instance via Postgres)
assay workflow start --type Deploy --input @req.json   # start a run; --input takes @file/-/literal
assay workflow wait <id> --timeout 300                 # block for scripts; exit 0/1/2
assay workflow retry <id> --requested-by me --reason corrected  # retry terminal activity
assay schedule create nightly --cron "0 0 2 * * *" --timezone Europe/Berlin  --type Report
assay completion bash > /etc/bash_completion.d/assay   # generate shell completion
```

Global CLI flags (all env-backed and config-file-backed): `--engine-url`, `--api-key`,
`--namespace`, `--output`, `--config`. YAML config file auto-discovered at `--config PATH` /
`$ASSAY_CONFIG_FILE` / `$XDG_CONFIG_HOME/assay/config.yaml` / `~/.config/assay/config.yaml` /
`/etc/assay/config.yaml`. `api_key_file:` keeps the secret out of env/argv. Output formats: table
(TTY default), json (pipe default), jsonl, yaml.

Lua surface (worker + management in one module):

```lua
local workflow = require("assay.workflow")
workflow.connect("http://assay:8080", { token = env.get("ASSAY_API_KEY") })

-- Worker: define handlers, listen. ctx gets execute_activity / execute_parallel (v0.11.3) /
-- sleep / wait_for_signal / start_child_workflow / side_effect / register_query (v0.11.3) /
-- upsert_search_attributes (v0.11.3) / continue_as_new (v0.11.3).
workflow.define("Pipeline", function(ctx, input) ... end)
workflow.activity("step", function(ctx, input) ... end)
workflow.listen({ queue = "default" })  -- blocks

-- Management: workflow.list/describe/get_events/get_state/list_children/retry_failed_activity/continue_as_new,
-- workflow.schedules.{create,list,describe,patch,pause,resume,delete},
-- workflow.namespaces.{create,list,describe,stats,delete},
-- workflow.workers.list, workflow.queues.stats.
```

**Dashboard** at `/workflow/` — reads + tier-1 operator controls (start, signal, cancel, terminate,
failed-activity retry, continue-as-new, live `register_query` state, schedule CRUD + pause/resume,
namespace create/delete). Engine version shown in the status bar, fetched from `/api/v1/version`.

**Dashboard whitelabel** (v0.11.10+). The embedded dashboard can be rebranded per-deployment via
`ASSAY_WHITELABEL_*` env vars so platform teams fronting assay inside another admin UI can match
their own identity. Nine knobs: `_NAME`, `_SUBTITLE` (v0.11.11), `_MARK` (v0.11.11), `_LOGO_URL`,
`_PAGE_TITLE`, `_PARENT_URL` + `_PARENT_NAME` (sidebar-footer back-link to the parent app),
`_API_DOCS_URL` (override or set to `""` to hide the built-in API Docs link), and `_CSS_URL` (extra
stylesheet loaded after assay's own CSS for re-skinning via CSS custom properties). v0.11.11 also
flips the footer to `Powered by Assay Workflow Engine vX.Y.Z` (linked to assay.rs) whenever any
identity is customised. All optional; unset env preserves assay's built-in identity. Full table +
examples in `docs/modules/workflow.md#dashboard-whitelabel`.

**Optional S3 archival.** Default `assay-engine` builds include archival support as of v0.5.6; it
activates only when `ASSAY_ARCHIVE_S3_BUCKET` is set. Custom `assay-workflow` embedders enable the
default-off `s3-archival` cargo feature. It bundles completed workflows to S3 after
`ASSAY_ARCHIVE_RETENTION_DAYS` and stubs the row with `archive_uri`.

### Example: Kubernetes Job

```yaml
apiVersion: batch/v1
kind: Job
metadata:
  name: configure-service
  annotations:
    argocd.argoproj.io/hook: PostSync
spec:
  template:
    spec:
      containers:
        - name: configure
          image: ghcr.io/developerinlondon/assay:latest
          command: ["assay", "/scripts/configure.lua"]
          volumeMounts:
            - name: scripts
              mountPath: /scripts
          env:
            - name: SERVICE_URL
              value: "http://my-service:8080"
            - name: API_TOKEN
              valueFrom:
                secretKeyRef:
                  name: my-service-credentials
                  key: admin_api_token
      volumes:
        - name: scripts
          configMap:
            name: postsync-scripts
      restartPolicy: Never
```

## Builtins

Available as globals in all `.lua` scripts — no `require` needed:

| Category       | Functions                                                                                                                                                                                                                                                                                                                                                                                |
| -------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| HTTP           | `http.get(url, opts?)`, `http.post(url, body, opts?)`, `http.put(url, body, opts?)`, `http.patch(url, body, opts?)`, `http.delete(url, opts?)`, `http.serve(port, routes)` — `http.serve` response handlers accept array header values to emit the same header name multiple times (e.g., multiple `Set-Cookie`)                                                                         |
| JSON/YAML/TOML | `json.parse(str)`, `json.encode(tbl)`, `yaml.parse(str, opts?)`, `yaml.encode(tbl, opts?)`, `toml.parse(str)`, `toml.encode(tbl)`                                                                                                                                                                                                                                                        |
| Filesystem     | `fs.read(path)`, `fs.read_bytes(path)`, `fs.write(path, str)`, `fs.write_bytes(path, data)`, `fs.remove(path)`, `fs.list(path)`, `fs.stat(path)`, `fs.mkdir(path)`, `fs.exists(path)`, `fs.copy(src, dst)`, `fs.rename(src, dst)`, `fs.glob(pattern)`, `fs.tempdir()`, `fs.chmod(path, mode)`, `fs.readdir(path, opts?)`                                                                 |
| Crypto         | `crypto.jwt_sign(claims, key, alg, opts?)`, `crypto.jwt_decode(token)` (decode without verifying), `crypto.hash(str, alg)`, `crypto.hmac(key, data, alg?, raw?)`, `crypto.random(len)`                                                                                                                                                                                                   |
| Base64         | `base64.encode(str)`, `base64.decode(str)`                                                                                                                                                                                                                                                                                                                                               |
| Regex          | `regex.match(pat, str)`, `regex.find(pat, str)`, `regex.find_all(pat, str)`, `regex.replace(pat, str, repl)`                                                                                                                                                                                                                                                                             |
| Database       | `db.connect(url)`, `db.query(conn, sql, params?)`, `db.execute(conn, sql, params?)`, `db.close(conn)`                                                                                                                                                                                                                                                                                    |
| WebSocket      | `ws.connect(url, opts?)` — `opts.{subprotocols, headers, insecure}`, `ws.send(conn, msg)`, `ws.send_binary(conn, bytes)`, `ws.recv(conn)` — raw bytes for binary frames, `ws.protocol(conn)`, `ws.close(conn)`                                                                                                                                                                           |
| DNS            | `dns.lookup(name, type, opts?)` — `type` is `A`/`AAAA`/`CNAME`/`MX`/`NS`/`TXT`, `opts.{server, timeout_ms, tries}`; MX answers are `{preference, exchange}` sorted lowest-first, TXT chunks are rejoined, NXDOMAIN is an empty list and SERVFAIL/timeout raises, `dns.dnsbl(domain, list, opts?)` → `{listed, codes}` (`127.255.255.0/24` is the public-resolver refusal, not a listing) |
| Templates      | `template.render(path, vars)`, `template.render_string(tmpl, vars)`                                                                                                                                                                                                                                                                                                                      |
| Async          | `async.spawn(fn)`, `async.spawn_interval(fn, ms)`, `handle:await()`, `handle:cancel()`                                                                                                                                                                                                                                                                                                   |
| Assert         | `assert.eq(a, b, msg?)`, `assert.ne(a, b, msg?)`, `assert.gt(a, b, msg?)`, `assert.lt(a, b, msg?)`, `assert.contains(str, sub, msg?)`, `assert.not_nil(val, msg?)`, `assert.matches(str, pat, msg?)`                                                                                                                                                                                     |
| Logging        | `log.info(msg)`, `log.warn(msg)`, `log.error(msg)`                                                                                                                                                                                                                                                                                                                                       |
| Utilities      | `env.get(key)`, `env.set(key, value)`, `env.list()`, `sleep(secs)`, `time()`                                                                                                                                                                                                                                                                                                             |
| Shell          | `shell.exec(cmd, opts?)` — execute commands with timeout, working dir, env                                                                                                                                                                                                                                                                                                               |
| Process        | `process.list()`, `process.is_running(name)`, `process.kill(pid, signal?)`, `process.wait_idle(names, timeout, interval)`                                                                                                                                                                                                                                                                |
| Disk           | `disk.usage(path)` — returns `{total, used, available, percent}`, `disk.sweep(dir, age_secs)`, `disk.dir_size(path)`                                                                                                                                                                                                                                                                     |
| OS             | `os.hostname()`, `os.arch()`, `os.platform()`                                                                                                                                                                                                                                                                                                                                            |
| Markdown       | `markdown.to_html(source)` — convert Markdown to HTML (tables, strikethrough, task lists)                                                                                                                                                                                                                                                                                                |

HTTP responses: `{status, body, headers}`. Options: `{headers = {["X-Key"] = "val"}}`.

### Lua Test Assertions

Lua tests run inside Assay and already have the `assert` builtin table. Do not define local
assertion helpers such as `assert_eq`, `assert_truthy`, `fail`, or generic `check` wrappers in
`*.test.lua` scripts. Use the builtins directly: `assert.eq`, `assert.ne`, `assert.gt`, `assert.lt`,
`assert.contains`, `assert.not_nil`, and `assert.matches`.

Small progress-print helpers like `ok(label)` are fine. Standalone orchestration runners may still
use teardown-aware fatal helpers when they are not expressing a test assertion.

Use moon/mise tasks for Lua suites. The assay-engine Lua client smoke suite is `mise run engine-lua`
/ `moon run engine-lua:test`, with lifecycle orchestration in
`crates/assay-engine/tests-lua/run.lua`. Do not add bash `run.sh` wrappers for Assay-owned Lua test
lifecycle; prefer Assay Lua plus `process.spawn`, `process.wait`, `http`, and `fs` so the test
harness exercises the runtime it is validating.

### `http.serve` Response Shapes

Route handlers return a table. Three shapes are supported:

```lua
-- Body response (default Content-Type: text/plain)
return { status = 200, body = "hello" }

-- JSON response (default Content-Type: application/json)
return { status = 200, json = { ok = true } }

-- SSE streaming response (default Content-Type: text/event-stream)
return {
  status = 200,
  sse = function(send)
    send({ data = "connected" })
    sleep(1)  -- async builtins work inside SSE handlers
    send({ event = "update", data = json.encode({ count = 1 }), id = "1" })
    -- stream closes when function returns
  end
}
```

Custom headers override defaults: `headers = { ["content-type"] = "text/html" }`.

Header values can be either a string or an array of strings. Array values emit the same header name
multiple times — required for `Set-Cookie` with multiple cookies and useful for `Link`, `Vary`,
`Cache-Control`, etc.:

```lua
return {
  status = 200,
  headers = {
    ["Set-Cookie"] = {
      "session=abc; Path=/; HttpOnly",
      "csrf=xyz; Path=/",
    },
  },
  body = "ok",
}
```

SSE `send()` accepts: `event` (string), `data` (string), `id` (string), `retry` (integer). `event`
and `id` must not contain newlines. `data` handles multi-line automatically.

## Stdlib Modules

<!-- dprint-ignore-start -->
<!-- BEGIN STDLIB TABLE -->
<!-- Generated by site/build.lua from the @module/@description/@category doc blocks in crates/assay/stdlib/**/*.lua — do not edit by hand. -->

87 embedded Lua modules loaded via `require("assay.<name>")`:

| Module | Description |
| --- | --- |
| **Core (text, URLs, versions)** | |
| `assay.ansi` | ANSI SGR → HTML conversion + stripper for log viewers. No state, no deps. |
| `assay.url` | Pure-Lua URL helpers. RFC 3986 percent-encoding plus form-encoded body builder. |
| `assay.version` | Cross-scheme version comparison: semver, debian, rpm, plain numeric. Pure Lua. |
| **Monitoring & Observability** | |
| `assay.alertmanager` | Alertmanager alert and silence management. Query, create, and delete alerts and silences. |
| `assay.grafana` | Grafana monitoring and dashboards. Health, datasources, dashboards, annotations, alerts, folders, organization. |
| `assay.healthcheck` | HTTP health checking utilities. Status codes, JSON path, body matching, latency, multi-check. |
| `assay.loki` | Loki log aggregation. Push logs, query with LogQL, labels, series, tail. |
| `assay.prometheus` | Prometheus monitoring queries. PromQL instant/range queries, alerts, targets, rules, series. |
| **Kubernetes** | |
| `assay.certmanager` | cert-manager certificate lifecycle. Certificates, issuers, ACME orders and challenges. |
| `assay.crossplane` | Crossplane infrastructure management. Providers, XRDs, compositions, managed resources. |
| `assay.k8s` | Kubernetes API client for Kubernetes clusters. 30+ resource types, CRDs, readiness checks, pod logs, rollouts. |
| `assay.velero` | Velero backup and restore. Backups, restores, schedules, storage locations. |
| **GitOps** | |
| `assay.argocd` | ArgoCD GitOps application management. Apps, sync, health, projects, repositories, clusters. |
| `assay.flux` | Flux CD GitOps toolkit. GitRepositories, Kustomizations, HelmReleases, notifications. |
| `assay.kargo` | Kargo continuous promotion. Stages, freight, promotions, warehouses, pipeline status. |
| **Security & Identity** | |
| `assay.authz` | In-process authorization engine: policy statements, grants-at-scope with typed bounds, ABAC conditions, deny-wins, asymmetric fail-closed. |
| `assay.dex` | Dex OIDC identity provider. Discovery, JWKS, health, and configuration validation. |
| `assay.engine.auth` | Lua client for assay-engine's auth module — login/whoami, passkey, OIDC client + provider, biscuit, zanzibar, and admin… |
| `assay.oauth2` | Google OAuth2 helper for loading credentials, refreshing access tokens, persisting token files, and building auth headers. |
| `assay.ory` | Convenience umbrella for the Ory stack submodules (kratos, hydra, keto, rbac). |
| `assay.ory.hydra` | Ory Hydra OAuth2 and OpenID Connect — client CRUD, authorize URL builder, token exchange, login/consent/logout challenges, introspection, JWK endpoint. |
| `assay.ory.keto` | Ory Keto authorization — relation-tuple CRUD, permission checks (including OPL permits), role membership queries. |
| `assay.ory.kratos` | Ory Kratos identity management — login/registration/recovery/settings flows, identity CRUD via admin API, session introspection, schemas. |
| `assay.ory.rbac` | Capability-based RBAC engine layered on top of Ory Keto. |
| `assay.rauthy` | Rauthy IdP admin API client. OAuth2 client reconciliation, secret rotation, discovery, health. |
| `assay.zitadel` | Zitadel OIDC identity management. Projects, OIDC apps, IdPs, users, login policies. |
| **Secrets & Vaults** | |
| `assay.engine.vault` | Lua client for assay-engine's vault module (plan 17 / v0.3.0) mounted at `/api/v1/vault/*`. |
| `assay.eso` | External Secrets Operator. ExternalSecrets, SecretStores, ClusterSecretStores sync status. |
| `assay.hashicorp` | Convenience umbrella for HashiCorp tooling submodules. |
| `assay.hashicorp.vault` | HashiCorp Vault / OpenBao client. KV, policies, auth, transit, PKI, token management. For the assay-engine native vault module use `assay.vault`. |
| `assay.openbao` | OpenBao secrets management (vault-compatible). Alias for assay.hashicorp.vault. |
| **Cloud & Network** | |
| `assay.aws.ec2` | AWS EC2 read-only queries via Signature V4. Describe instances, volumes, security groups. |
| `assay.aws.ecr` | AWS Elastic Container Registry. Get authorization tokens for pushing/pulling images. |
| `assay.aws.eks` | EKS authentication token minting (the aws eks get-token equivalent), in-process — no aws CLI subprocess, works in readonly mode. |
| `assay.aws.s3` | AWS S3 read-only queries via Signature V4. List buckets, list objects, head object. |
| `assay.aws.sigv4` | AWS Signature V4 request signing. Generates Authorization headers and presigned URLs for AWS API calls. |
| `assay.aws.sts` | AWS credential resolution + STS, in-process (no aws CLI needed, works in readonly mode). |
| `assay.infoblox` | Infoblox WAPI. DNS records, networks, DHCP ranges, grid status. Basic auth. |
| `assay.openstack` | OpenStack inventory client. Keystone v3 auth and service discovery; identity, compute, image, network, and quota reads. |
| `assay.tailscale` | Tailscale REST API client. OAuth2 client_credentials, mint auth keys, list/find devices, manage device key expiry, tags, authorize, delete, ACL preview. |
| `assay.traefik` | Traefik reverse proxy API. Routers, services, middlewares, entrypoints, TLS status. |
| **Host & Linux** | |
| `assay.apt` | Debian/Ubuntu apt package index reader. Fetches and parses Packages indexes (gz/xz/zstd/plain) from any apt-style HTTP repository. |
| `assay.cron` | Scheduled job inspector. |
| `assay.fs_snapshot` | Read-only filesystem snapshots for crash-consistent backup capture. btrfs subvolume snapshot / zfs snapshot under the hood. |
| `assay.nspawn` | nspawn machine provisioning + lifecycle. |
| `assay.pkg` | Package manager framework — catalog loading, target abstractions, plan/apply/reconcile. |
| `assay.rustic` | rustic backup CLI wrapper. Snapshots, backup, restore, init, check. Repository + credentials passed via env (kept off the cmdline). |
| `assay.shell` | WebSocket ↔ PTY bridge for in-browser terminals (xterm.js, etc). |
| `assay.system` | Host introspection and control umbrella: re-exports linux/cgroup/systemd builtins + assay.cron, plus convenience aggregates… |
| **Data & Storage** | |
| `assay.postgres` | PostgreSQL database helpers. User/database management, grants, Vault integration. |
| `assay.s3` | S3-compatible object storage. Buckets, objects, copy, list with AWS Signature V4 auth. |
| **Developer Tools** | |
| `assay.engine` | Convenience umbrella for the assay-engine clients (core, auth, workflow, vault). One `engine.connect()` call returns all four sharing a base URL + admin key. |
| `assay.engine.core` | Lua client for assay-engine's engine-core admin API mounted at `/api/v1/engine/core/*`. |
| `assay.engine.workflow` | Lua client for assay-engine's workflow module mounted at `/api/v1/engine/workflow/*`. |
| `assay.engine.workflow.worker` | Worker poll loop + task handlers for `assay.engine.workflow`. |
| `assay.github` | GitHub REST API client. PRs, issues, actions, repositories, GraphQL, releases. No gh CLI dependency. |
| `assay.gitlab` | GitLab REST API v4 — projects, repositories, commits, merge requests, pipelines, issues, releases, container registry. |
| `assay.harbor` | Harbor container registry. Projects, repositories, artifacts, vulnerability scanning. |
| `assay.sonarqube` | SonarQube code quality API. Quality gates, issues, hotspots, measures, projects. |
| `assay.unleash` | Unleash feature flag management. Projects, features, environments, strategies, API tokens. |
| **Registries & Company Data** | |
| `assay.apify` | Apify actor runs behind a mandatory spend cap — start an actor, wait for a terminal state, read the dataset back together with what the run cost — plus… |
| `assay.bettercontact` | BetterContact waterfall enrichment — submit a batch of people, poll until the run terminates, read back verified emails and phones. |
| `assay.brreg` | Norway's Enhetsregisteret (Brønnøysund) — every registered Norwegian entity, keyless and free. |
| `assay.companies_house` | UK Companies House — every registered UK company and its officers. The only registry module that needs a key; it is free, but issued per caller. |
| `assay.contactout` | ContactOut lead data — LinkedIn profile enrichment, email-to-profile reverse lookup, and flexible person enrichment. |
| `assay.cvr` | Denmark's CVR (Det Centrale Virksomhedsregister) via the public cvrapi.dk gateway — Danish and Norwegian entities by name, VAT number or phone. |
| `assay.edgar` | SEC EDGAR — US public-company registry: ticker/name lookup, company submissions (SIC, addresses, filings), and full-text search over filings. |
| `assay.email_verify` | Free email verification rung — syntax, MX via DNS-over-HTTPS, and pattern candidates. |
| `assay.gleif` | GLEIF LEI registry — search global legal entities by name, fetch a record by LEI, fuzzy-complete names. Free, keyless, every record carries provenance. |
| `assay.lead_provider` | The contract every lead-data adapter implements — uniform person/email shapes with provenance, and the budget gate that must wrap every paid call. |
| `assay.mails_so` | mails.so email verification — the paid verify_email rung of NEP-0007 §10. |
| **SaaS & Business** | |
| `assay.clayinbox` | Clayinbox mailbox provisioning (app.clayinbox.ai) — the domains a workspace holds, the Google mailboxes on them, and what those boxes bill, listed to the… |
| `assay.clickup` | ClickUp REST API — tasks, lists, folders, spaces, goals, custom fields, time tracking, comments, and Docs. |
| `assay.excalidash` | ExcaliDash REST API — Excalidraw drawings, collections, version-history snapshots, and user/link sharing on a self-hosted ExcaliDash dashboard. |
| `assay.forge` | Primeforge and Warmforge over the shared forge MCP endpoint — domains, mailboxes, warm-up progress, placement tests, the DNS health report, and Primeforge's… |
| `assay.gcal` | Google Calendar REST API client with OAuth2 token refresh. Events CRUD, calendar list. |
| `assay.huly` | Huly transactor REST API — document queries, transactions, fulltext search, and tracker helpers for projects, issues, milestones, and components on a… |
| `assay.jev` | TypeSafe Jev System One — ask a small semantic judgment as a typed question and get a typed answer back, never free text a caller has to parse. |
| `assay.n8n` | n8n public REST API (/api/v1) — workflows, executions, credentials, tags, variables, projects, folders, users, data tables, source control, audit, settings. |
| `assay.neutron` | Neutron self-hosted agent platform — full admin API: agents (personas, tool policies, guardrails, baked assay modules), secrets, git-host connections… |
| `assay.openclaw` | OpenClaw AI agent platform integration. Invoke tools, send messages, manage state, spawn sub-agents, approval gates, LLM tasks. |
| `assay.plane` | Plane REST API — projects, work items, cycles, modules, states, labels, members, comments, and links. Covers sprint execution on self-hosted or cloud Plane. |
| `assay.salesforge` | Salesforge sequencer — the public REST API for workspaces, mailboxes, sequences, contacts, do-not-contact and replies, plus the web app's own API for the… |
| `assay.servicenow` | ServiceNow Table API and a small CMDB helper. List, get, create, update records; query CMDB classes. |
| `assay.vendor_cost` | The cost contract every vendor module answers with — money as whole cents, and one item shape whose absent keys mean the vendor never stated the fact rather… |
| **Email & Comms** | |
| `assay.email_triage` | Email triage: what a message says about itself, read from its own headers and words with no model and no network — whether it declares itself a machine… |
| `assay.gmail` | Gmail REST API client with OAuth2 token refresh. Search, read, reply, send emails, manage labels. |
<!-- END STDLIB TABLE -->
<!-- dprint-ignore-end -->

### Client Pattern

Every stdlib module follows the same structure:

```lua
local grafana = require("assay.grafana")
local c = grafana.client("http://grafana:3000", { api_key = "glsa_..." })
local h = c:health()
assert.eq(h.database, "ok")
```

1. `require("assay.<name>")` returns module table `M`
2. `M.client(url, opts?)` creates a client with auth config
3. Client methods use `c:method()` (colon = implicit self)
4. Errors raised via `error()` — use `pcall()` to catch

Auth varies by service: `{ token = "..." }`, `{ api_key = "..." }`,
`{ username = "...", password = "..." }`.

## Tool Mode (OpenClaw Integration)

Assay v0.6.0 adds tool mode for integration with OpenClaw AI agents:

```bash
assay run --mode tool script.lua                    # Run as agent tool, structured JSON output
assay resume --token <token> --approve yes|no       # Resume paused approval gate
```

Tool mode produces structured JSON output suitable for agent consumption. When a script hits an
approval gate (via `openclaw.approve()`), execution pauses and returns a resume token. The agent or
human can then approve or reject via `assay resume`.

```
+-------------------------------------------------------+
| OpenClaw Agent                                        |
|   |                                                   |
|   +--> assay run --mode tool deploy.lua               |
|   |      |                                            |
|   |      +--> approval gate -> pauses, returns token  |
|   |                                                   |
|   +--> human reviews                                  |
|   |                                                   |
|   +--> assay resume --token <t> --approve yes         |
|          |                                            |
|          +--> script resumes from gate                |
+-------------------------------------------------------+
```

### OpenClaw Extension

The `@developerinlondon/assay-openclaw-extension` package (GitHub Packages) registers Assay as an
OpenClaw agent tool:

```bash
# One-time: configure npm to use GitHub Packages for @developerinlondon scope
echo "@developerinlondon:registry=https://npm.pkg.github.com" >> ~/.npmrc

# Install the extension
openclaw plugins install @developerinlondon/assay-openclaw-extension
```

Configuration in OpenClaw plugin config:

| Key             | Default        | Description                         |
| --------------- | -------------- | ----------------------------------- |
| `binaryPath`    | PATH lookup    | Explicit path to the `assay` binary |
| `timeout`       | `20`           | Execution timeout in seconds        |
| `maxOutputSize` | `524288`       | Maximum stdout collected from Assay |
| `scriptsDir`    | workspace root | Root directory for Lua scripts      |

See [openclaw-extension/README.md](openclaw-extension/README.md) for full details.

## Adding a New Stdlib Module

No Rust changes needed. Modules are auto-discovered via `include_dir!("$CARGO_MANIFEST_DIR/stdlib")`
in `src/lua/mod.rs`.

### 1. Create `stdlib/<name>.lua`

Follow the client pattern. Reference `grafana.lua` (simple, 110 lines) or `vault.lua`
(comprehensive, 330 lines):

```lua
local M = {}

function M.client(url, opts)
  opts = opts or {}
  local c = {
    url = url:gsub("/+$", ""),
    token = opts.token,
  }

  local function headers(self)
    local h = { ["Content-Type"] = "application/json" }
    if self.token then h["Authorization"] = "Bearer " .. self.token end
    return h
  end

  local function api_get(self, path_str)
    local resp = http.get(self.url .. path_str, { headers = headers(self) })
    if resp.status ~= 200 then
      error("<name>: GET " .. path_str .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  local function api_post(self, path_str, payload)
    local resp = http.post(self.url .. path_str, payload, { headers = headers(self) })
    if resp.status ~= 200 and resp.status ~= 201 then
      error("<name>: POST " .. path_str .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  function c:health()
    return api_get(self, "/api/health")
  end

  return c
end

return M
```

Conventions:

- `api_get/api_post/api_put/api_delete` are local helpers, not exported
- Error format: `"<module>: <METHOD> <path> HTTP <status>: <body>"`
- Strip trailing slashes: `url:gsub("/+$", "")`
- All HTTP uses builtins (`http.get`, `json.parse`) — no external requires
- 404 = nil pattern: check `resp.status == 404`, return `nil`
- Idempotent helpers go on `M` (module level), not on the client

### 2. Create `tests/stdlib_<name>.rs`

Wiremock-based tests. One test per client method:

```rust
mod common;

use common::run_lua;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn test_require_<name>() {
    let script = r#"
        local mod = require("assay.<name>")
        assert.not_nil(mod)
        assert.not_nil(mod.client)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_<name>_health() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/health"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "status": "OK"
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local mod = require("assay.<name>")
        local c = mod.client("{}", {{ token = "test-token" }})
        local h = c:health()
        assert.eq(h.status, "OK")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}
```

Note: Lua tables in `format!` strings need doubled braces: `{{ key = "value" }}`.

### 3. Add documentation

Add a `docs/modules/<name>.md` file with method signatures and examples. `site/build.lua` generates
the website pages, `llms-full.txt` and the README stdlib table from these files, so there is no need
to update `README.md`, `SKILL.md`, or the website by hand.

The module's own `--- @module` / `--- @description` / `--- @category` doc block is what feeds
`assay context` and the AGENTS.md stdlib table, so write it as carefully — one line per tag, and a
`@description` whose first sentence or two stand alone as a summary.

### 4. Verify

```bash
cargo check && cargo clippy -- -D warnings && cargo test
assay site/build.lua   # verify docs build
```

## Directory Structure

```
assay/
├── Cargo.toml
├── Cargo.lock
├── CHANGELOG.md              # Release notes (manual, rendered to website by build.lua)
├── AGENTS.md                 # This file — agent instructions
├── SKILL.md                  # LLM agent integration guide
├── README.md                 # Human-facing README
├── Dockerfile                # Multi-stage: rust builder -> scratch
├── src/
│   ├── main.rs               # CLI entry point (clap)
│   ├── lib.rs                # Library root (pub mod lua)
│   └── lua/
│       ├── mod.rs            # VM setup, sandbox, stdlib loader (include_dir!)
│       └── builtins/
│           ├── mod.rs        # register_all() — wires builtins into Lua globals
│           ├── http.rs       # http.{get,post,put,patch,delete,serve} + wildcard routes
│           ├── core.rs       # env, sleep, time, fs (read/write + read_bytes/write_bytes), base64, regex, log, async
│           ├── markdown.rs   # markdown.to_html() via pulldown-cmark
│           └── ...           # json, yaml, toml, crypto, db, ws, template, assert, etc.
├── stdlib/                   # Embedded Lua modules (auto-discovered)
│   ├── vault.lua             # Comprehensive reference (330 lines)
│   ├── grafana.lua           # Simple reference (110 lines)
│   └── ...                   # one file per module
├── docs/
│   └── modules/              # Module documentation (single source of truth)
│       ├── ory.md
│       └── ...               # one page per documented module
├── site/                     # Website source (tracked in git)
│   ├── build.lua             # Assay builds its own docs (replaces bash/npm)
│   ├── serve.lua             # Dev server using http.serve() with wildcard routes
│   ├── pages/                # HTML templates with __PLACEHOLDER__ markers
│   ├── partials/             # header.html, footer.html (nav, theme toggle, search)
│   └── static/               # style.css, _headers, _redirects, llms.txt
├── build/                    # Build output (gitignored)
│   └── site/                 # Generated website, deployed to Cloudflare Pages
├── tests/
│   ├── common/mod.rs         # Test helpers: run_lua(), create_vm(), eval_lua()
│   ├── stdlib_vault.rs       # One test file per stdlib module
│   ├── http_serve.rs         # HTTP server tests (incl. wildcard routes)
│   ├── markdown.rs           # Markdown builtin tests
│   └── ...
└── examples/                 # Example scripts and check configs
```

## Design Decisions (FINAL)

| Decision         | Choice   | Reason                                                                   |
| ---------------- | -------- | ------------------------------------------------------------------------ |
| Language runtime | Lua 5.5  | ArgoCD compatible, 30yr ecosystem, native int64, perf irrelevant for I/O |
| Not Luau         | Rejected | Lua 5.1 base, Roblox ecosystem, no int64                                 |
| Not Rhai         | Rejected | 6x slower, no async, no coroutines                                       |
| Not Wasmtime     | Rejected | Requires compile step, bad for script iteration                          |

## Release Process

### When making changes

1. **Code changes** — implement in `src/` or `stdlib/`, add tests in `tests/`
2. **Document** — add/update `docs/modules/<name>.md` (single source of truth)
3. **CHANGELOG.md** — add entry under the current version section
4. **AGENTS.md** — update builtins table if API surface changed
5. **Verify**: `cargo clippy --tests -- -D warnings && cargo test && assay site/build.lua`

The following are **auto-generated** — do NOT edit manually:

- `build/site/` — entire website (generated by `assay site/build.lua`)
- Module count on the website (computed from `src/lua/builtins/mod.rs` + `stdlib/`)
- `build/site/llms-full.txt` (concatenated from `docs/modules/*.md`)
- `build/site/modules/*.html` (generated from `docs/modules/*.md`)
- `build/site/modules.html` (index page)
- Changelog page (rendered from `CHANGELOG.md`)

### Releasing a version

1. **Bump version** in `Cargo.toml`
2. **Finalize CHANGELOG.md** — ensure version header has correct date
3. **Run checks**: `cargo clippy --tests -- -D warnings && cargo test`
4. **Merge PR** to main
5. **Tag the release**: `git tag v0.9.0 && git push origin v0.9.0`

The tag push triggers `.github/workflows/release.yml` which publishes:

- GitHub Release (binaries + checksums)
- crates.io (`assay-lua` crate)
- Docker image (`ghcr.io/developerinlondon/assay`)

The merge to main triggers `.github/workflows/deploy.yml` which:

- Builds assay (`cargo build --release`)
- Builds the website (`assay site/build.lua`)
- Indexes for search (`pagefind --site build/site`)
- Deploys to Cloudflare Pages (`wrangler pages deploy build/site/`)

### Local development

```bash
cargo build --release               # build assay
assay site/build.lua                 # build website
npx pagefind --site build/site       # index for search (optional)
assay site/serve.lua                 # serve at http://localhost:3000
```

## Commands

```bash
cargo check                        # Type check
cargo clippy -- -D warnings        # Lint (warnings = errors)
cargo test                         # Run all tests
cargo build --release              # Release build (~11 MB)
```
