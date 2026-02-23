# Assay: A Knowledge-Aware Execution Engine for LLMs

## The Problem Today

AI coding agents interact with external services through MCP servers. Each server dumps its full
tool schemas into the LLM's context **before any conversation starts**:

```
┌──────────────────────────────────────────────────────────────┐
│  LLM Context Window (200K tokens)                            │
│                                                              │
│  ┌─────────────────────────────────────────────────────────┐ │
│  │ SYSTEM PROMPT                                           │ │
│  │ Tool: github_create_issue          (~600 tokens)        │ │
│  │ Tool: github_list_issues           (~600 tokens)        │ │
│  │ ... 18 more github tools ...       (~10,000 tokens)     │ │
│  │ Tool: cloudinary_upload            (~800 tokens)        │ │
│  │ ... 18 more cloudinary tools ...   (~11,000 tokens)     │ │
│  │ Tool: filesystem_read              (~400 tokens)        │ │
│  │ ... 10 more filesystem tools ...   (~5,000 tokens)      │ │
│  │                                                         │ │
│  │ TOTAL: 61 tools = ~36,000 tokens (18% of context)       │ │
│  └─────────────────────────────────────────────────────────┘ │
│  Actual conversation: 164K tokens remaining                  │
└──────────────────────────────────────────────────────────────┘
```

Three problems:

1. **Token waste** - 36K tokens of schemas for tools the LLM may never use
2. **Round-trip overhead** - each operation = separate tool call = separate LLM turn
3. **No composition** - can't combine 3 API calls into one logical operation

Anthropic's Tool Search (defer_loading + BM25) reduces problem #1 by ~85%. But problems #2 and #3
remain. And it's still the same model: one tool call at a time.

---

## The Key Insight: Assay Already Solved This (for Kubernetes)

Assay's existing architecture is a working proof of concept:

```lua
-- This 10-line script replaces what would be 4+ separate MCP tool calls:
local vault = require("assay.vault")
local k8s   = require("assay.k8s")

local c = vault.authenticated_client("http://vault:8200")
local secret = c:kv_get("secrets", "myapp/config")
assert.not_nil(secret, "secret missing")

local deploy = k8s.rollout_status("default", "myapp")
assert.eq(deploy.complete, true, "rollout not complete")
log.info("Secret verified, deployment healthy")
```

The LLM writes a script. Assay executes it. Multiple API calls, conditional logic, error handling --
all in one execution. No round-trips between calls.

But currently assay only covers Kubernetes-adjacent services (Vault, ArgoCD, Prometheus...). The
question is: **can this model scale to ANY API?**

---

## The Design: Assay as a Skill (Not an MCP Server)

Assay is NOT another MCP server. It's a **CLI tool** that an agent invokes via its existing shell,
guided by a **skill** (a lightweight prompt expansion, ~100 tokens idle).

Why not MCP? If the whole thesis is "MCP tool schemas waste tokens," adding another MCP server is
contradictory. Instead, assay uses what every agent already has: a shell (Bash tool).

### The Interface

```
assay search <query>          Search knowledge index, return quick-ref + auth status
assay exec '<lua script>'     Execute inline Lua in sandboxed VM
assay exec script.lua         Execute Lua file (existing behavior)
```

### The Skill (how the LLM learns about assay)

A `SKILL.md` file (~100 tokens idle, ~400 tokens when activated):

```
When you need to call external APIs (GitHub, Slack, Stripe, Vault, K8s, etc.), use assay:

  $ assay search 'your query'     # find modules + check auth
  $ assay exec 'lua code'         # run script

Modules auto-import. No require() needed:
  local c = github.client()
  c:create_issue("owner", "repo", {title = "Bug", labels = {"bug"}})

Run assay search first if unsure which module to use.
```

---

## How Search Works: BM25 in ~80 Lines of Rust

### Performance

For 23-100 documents with ~100 terms each: **1-10 microseconds per query**. Hand-rolled, zero
dependencies, pure arithmetic. Not "fast" -- instantaneous.

### Why BM25, not grep

Grep answers: "does this word appear?" (yes/no) BM25 answers: "which document is MOST relevant?"
(ranked)

When the LLM asks "how do I create a github issue and check sentry errors," we need to know that
`github` scores highest, `sentry` scores second, and `alertmanager` barely matches on "error." Grep
can't rank. BM25 can. At this scale both are sub-millisecond, so we pick the better answer.

### What Gets Indexed

Each module becomes a "document" of searchable terms:

```
┌──────────────────────────────────────────────────────────────┐
│  Document: "github"                                          │
│                                                              │
│  Terms extracted from:                                       │
│   @module header    → "github"                               │
│   @description      → "repos issues pull requests actions"   │
│   @keywords         → "git repository commit pr"             │
│   function names    → "create_issue issues pulls merge_pull"  │
│   parameter names   → "owner repo title body labels state"   │
│                                                              │
│  Term frequencies:                                           │
│   { "github": 3, "issue": 5, "pull": 4, "create": 2, ... }  │
│   doc_len: 47 terms                                          │
└──────────────────────────────────────────────────────────────┘
```

### The BM25 Formula (entire implementation)

```rust
fn bm25_score(tf: f32, df: usize, n_docs: usize, dl: usize, avgdl: f32) -> f32 {
    let k1 = 1.2_f32;
    let b = 0.75_f32;
    let idf = ((n_docs as f32 - df as f32 + 0.5) / (df as f32 + 0.5) + 1.0).ln();
    let tf_norm = (tf * (k1 + 1.0)) / (tf + k1 * (1.0 - b + b * dl as f32 / avgdl));
    idf * tf_norm
}
```

Rare terms (like "github" appearing in 1 doc) get HIGH scores. Common terms (like "api" appearing in
all docs) get LOW scores. That's what makes it work.

### Index is Built at Compile Time

`build.rs` scans `stdlib/*.lua` headers and function definitions → generates a static
`INDEX: &[IndexDoc]` array compiled into the binary. Zero runtime allocation to load. For externally
indexed APIs (`~/.assay/index/`), loaded once at startup into memory.

Total index size for 100 APIs: ~1-2 MB. Negligible.

---

## Auto-Import: No require() Needed

The LLM doesn't need to write `require("assay.github")`. A 6-line metatable on `_G` lazy-loads
modules on first access:

```lua
-- Injected into VM at creation time:
setmetatable(_G, {
  __index = function(_, key)
    local ok, mod = pcall(require, "assay." .. key)
    if ok then
      rawset(_G, key, mod)  -- cache for next access
      return mod
    end
    return nil
  end
})
```

Now `github.client()` just works -- it triggers `require("assay.github")` transparently. The LLM
writes natural Lua, not boilerplate.

---

## Auth: How Assay Handles API Keys

Three layers, from early detection to clear runtime errors:

### Layer 1: Module Metadata (build time)

Each module declares what credentials it needs:

```lua
--- @module assay.github
--- @description GitHub API: repos, issues, PRs, actions, releases
--- @env GITHUB_TOKEN  GitHub personal access token (required)
--- @env GITHUB_API_URL  GitHub Enterprise URL (optional)
```

### Layer 2: Search Response (runtime, before execution)

When the LLM runs `assay search`, the response includes auth status:

```
$ assay search 'create github issue'

{
  "results": [{
    "module": "github",
    "quick_ref": [
      "github.client({token?, url?}) → c",
      "c:create_issue(owner, repo, {title, body?, labels?}) → {number, url}",
      "c:issues(owner, repo, {state?, labels?}) → [{number, title}]",
      "c:pulls(owner, repo, {state?}) → [{number, title, merged}]"
    ],
    "auth": {
      "GITHUB_TOKEN": { "required": true,  "set": true  },
      "GITHUB_API_URL": { "required": false, "set": false }
    },
    "ready": true
  }]
}
```

The LLM sees `"ready": true` → safe to write the script. Or `"ready": false` → tell the user what's
missing before trying.

### Layer 3: Runtime Error (clear, actionable)

If the LLM skips search and jumps straight to `exec`:

```lua
-- Inside github.lua client():
function M.client(opts)
  opts = opts or {}
  local c = {
    url = (opts.url or env.get("GITHUB_API_URL") or "https://api.github.com"):gsub("/+$", ""),
    token = opts.token or env.get("GITHUB_TOKEN"),
  }

  if not c.token then
    error("github: GITHUB_TOKEN not set.\n"
      .. "Set it: export GITHUB_TOKEN='ghp_...'\n"
      .. "Or pass explicitly: github.client({token = '...'})\n"
      .. "Create one at: https://github.com/settings/tokens")
  end
  -- ...
end
```

### What the LLM Does With This

```
Scenario A: Auth is set (happy path)
  assay search → "ready": true → write script → works

Scenario B: Auth is missing
  assay search → "ready": false, "GITHUB_TOKEN not set"
  LLM tells user: "I need a GitHub token. Run:
    export GITHUB_TOKEN='ghp_your_token_here'
    or create one at https://github.com/settings/tokens"
  User sets it → LLM retries search → "ready": true → works

Scenario C: LLM skips search, auth fails at runtime
  assay exec 'github.client()...' → error with clear instructions
  LLM reads the error, tells user what's needed
```

### Auth Strategy Per Module

```
┌──────────────────────────────────────────────────────────────┐
│  Module        │ Strategy                                    │
├────────────────┼─────────────────────────────────────────────┤
│  github        │ GITHUB_TOKEN env or {token=...}             │
│  slack         │ SLACK_TOKEN or SLACK_WEBHOOK_URL             │
│  stripe        │ STRIPE_SECRET_KEY or {api_key=...}          │
│  sentry        │ SENTRY_TOKEN or {token=...}                 │
│  k8s (exists)  │ Auto: service account in-cluster             │
│  vault (exists)│ Auto: K8s secret or explicit token           │
│  s3 (exists)   │ AWS_ACCESS_KEY_ID + AWS_SECRET_ACCESS_KEY   │
│                │                                             │
│  Pattern:      │ 1. Check opts (explicit)                    │
│                │ 2. Check env var (implicit)                  │
│                │ 3. Error with setup URL and instructions     │
└──────────────────────────────────────────────────────────────┘
```

---

## Complete Interaction Flow

### Flow A: LLM Knows the API (1 Bash call)

```
User: "create a github issue for the login bug"

LLM thinks: I know GitHub's API, assay has a github module.

LLM runs via Bash:
  $ assay exec '
    local c = github.client()
    local issue = c:create_issue("myorg", "myapp", {
      title = "Login bug",
      body = "Login fails after OAuth redirect",
      labels = {"bug"}
    })
    print(json.encode(issue))
  '

  → {"number": 42, "url": "https://github.com/myorg/myapp/issues/42"}

LLM: "Created issue #42: https://github.com/myorg/myapp/issues/42"

Total: 1 Bash call
```

### Flow B: LLM Needs Discovery (2 Bash calls)

```
User: "check our PagerDuty on-call schedule and post to Slack"

LLM thinks: Not sure about PagerDuty API. Let me search.

Call 1:
  $ assay search 'pagerduty oncall schedule slack message'

  {
    "results": [
      {
        "module": "slack",
        "type": "native",
        "quick_ref": [
          "slack.client({token?, webhook?}) → c",
          "c:post_message(channel, text, {attachments?}) → {ok, ts}"
        ],
        "auth": { "SLACK_TOKEN": {"required": true, "set": true} },
        "ready": true
      },
      {
        "module": "pagerduty",
        "type": "indexed_api",
        "quick_ref": [
          "GET /oncalls → {oncalls: [{user: {summary}, start, end}]}",
          "GET /incidents?statuses[]=triggered → {incidents: [...]}"
        ],
        "auth": { "PD_TOKEN": {"required": true, "set": true} },
        "ready": true,
        "base_url": "https://api.pagerduty.com",
        "auth_header": "Token token={PD_TOKEN}"
      }
    ]
  }

Call 2:
  $ assay exec '
    local s = slack.client()

    -- PagerDuty: no native module, use http with indexed API docs
    local resp = http.get("https://api.pagerduty.com/oncalls", {
      headers = {
        Authorization = "Token token=" .. env.get("PD_TOKEN"),
        ["Content-Type"] = "application/json",
      }
    })
    local oncalls = json.parse(resp.body).oncalls

    local msg = "On-call right now:\n"
    for _, oc in ipairs(oncalls) do
      msg = msg .. "- " .. oc.user.summary .. "\n"
    end

    s:post_message("#ops", msg)
    print("Posted schedule to #ops")
  '

  → "Posted schedule to #ops"

Total: 2 Bash calls (search + execute)
```

### Flow C: Auth Missing (LLM asks user)

```
User: "create a stripe charge for $20"

LLM runs:
  $ assay search 'stripe charge payment'

  {
    "results": [{
      "module": "stripe",
      "quick_ref": [...],
      "auth": { "STRIPE_SECRET_KEY": {"required": true, "set": false} },
      "ready": false
    }]
  }

LLM: "I need your Stripe API key to proceed. You can:
  1. Set it: export STRIPE_SECRET_KEY='sk_test_...'
  2. Get one at: https://dashboard.stripe.com/apikeys"

User: "ok, set STRIPE_SECRET_KEY=sk_test_abc123"

LLM runs the script → works
```

---

## What Assay Becomes

Not an MCP server. A **knowledge-aware CLI tool** used via Bash, guided by a skill:

```
┌──────────────────────────────────────────────────────────────────┐
│                          ASSAY                                    │
│               (CLI tool, ~9MB static binary)                      │
│                                                                  │
│  ┌────────────────────────────────────────────────────────────┐  │
│  │                    Knowledge Index                         │  │
│  │                                                            │  │
│  │   Native Modules    API Docs          MCP Extracts         │  │
│  │   ┌──────────┐     ┌──────────┐      ┌──────────┐         │  │
│  │   │github.lua│     │Stripe API│      │PagerDuty │         │  │
│  │   │slack.lua │     │Twilio API│      │Datadog   │         │  │
│  │   │k8s.lua   │     │SendGrid  │      │LaunchDark│         │  │
│  │   │vault.lua │     │100s more │      │any MCP   │         │  │
│  │   │30+ more  │     └──────────┘      └──────────┘         │  │
│  │   └──────────┘          │                 │               │  │
│  │         │               │                 │               │  │
│  │         └───────────────┼─────────────────┘               │  │
│  │                         ▼                                 │  │
│  │               BM25 Search (1-10 μs)                        │  │
│  │        hand-rolled, 80 lines, zero deps                   │  │
│  └─────────────────────────┬─────────────────────────────────┘  │
│                            │                                    │
│  ┌─────────────────────────▼─────────────────────────────────┐  │
│  │              Sandboxed Lua 5.5 VM                          │  │
│  │                                                            │  │
│  │  Builtins: http, json, crypto, db, ws, fs, base64, regex   │  │
│  │  Auto-import: _G metatable lazy-loads assay.* modules       │  │
│  │  Auth: modules check env vars, error with instructions      │  │
│  └────────────────────────────────────────────────────────────┘  │
│                                                                  │
│  CLI:                                                            │
│    assay search <query>    → JSON quick-ref + auth status        │
│    assay exec '<script>'   → sandboxed execution result          │
│    assay index --from-mcp  → extract API knowledge (one-time)    │
│                                                                  │
│  LLM interface: Bash tool + assay skill (~100 tokens idle)       │
└──────────────────────────────────────────────────────────────────┘
```

### Compared to Alternatives

```
┌─────────────────────────────────────────────────────────────────┐
│ APPROACH              │ IDLE TOKENS │ ROUND-TRIPS │ NEW APIS    │
├───────────────────────┼─────────────┼─────────────┼─────────────┤
│ Raw MCP (today)       │ 36,000      │ 1 per op    │ install MCP │
│ Tool Search           │ ~5,000      │ 2+ per op   │ install MCP │
│ Skills                │ ~2,000      │ 1 per op    │ write skill │
│ Assay (this design)   │ ~100        │ 1-2 total   │ index docs  │
└─────────────────────────────────────────────────────────────────┘
```

The assay skill is ~100 tokens idle. When activated, it expands to ~400 tokens with usage
instructions. The LLM uses Bash (already available in every agent) to run assay commands. No MCP
protocol overhead, no persistent processes, no JSON-RPC.

---

## How an Agent Opts In

### Step 1: Install assay

```bash
cargo install assay-lua
# or: brew install assay
# or: docker pull ghcr.io/developerinlondon/assay
```

### Step 2: Add the skill

Claude Code:

```bash
# Project-level (recommended)
cp assay-skill/SKILL.md .claude/skills/assay.md

# Or use the skills package
npx skills add developerinlondon/assay-skill
```

Other agents: add the skill content to the agent's instruction/system prompt.

### Step 3: Index additional APIs (optional)

```bash
# From an existing MCP server (one-time, extracts knowledge):
assay index --from-mcp -- npx @pagerduty/mcp-server

# From an OpenAPI spec:
assay index --from-openapi https://api.stripe.com/openapi.json

# Pre-built community packs:
assay index --add popular-apis
```

### Step 4: Remove individual MCP servers

```diff
{
  "mcpServers": {
-   "github": { "command": "npx", "args": ["@github/mcp-server"] },
-   "cloudinary": { "command": "npx", "args": ["@cloudinary/mcp-server"] },
-   "filesystem": { "command": "npx", "args": ["@mcp/server-filesystem", "/tmp"] }
  }
}
```

No MCP servers needed. Assay handles everything via Bash.

---

## Implementation Phases

### Phase 1: Foundation (this PR)

- `assay search` subcommand with BM25 over native modules
- `assay exec` subcommand for inline Lua execution
- Auto-import via `_G` metatable (no require() needed)
- Module metadata headers (`@env`, `@description`) on all 23 stdlib modules
- Quick-ref generation in `build.rs` (function signatures from Lua source)
- Auth status checking in search results
- `assay.github` native module (first developer-tool module)
- `SKILL.md` for Claude Code integration
- Tests: BM25 search, github module (wiremock), exec subcommand (E2E)

### Phase 2: Knowledge Index

- `assay index` subcommand for external API knowledge
- `--from-mcp` flag: spawn MCP server, extract tools/list, index, discard
- `--from-openapi` flag: parse OpenAPI specs into indexed quick-refs
- Persistent index storage (`~/.assay/index/`)
- Search spans native modules + indexed APIs

### Phase 3: Community Knowledge

- Pre-built index packages (`assay index --add popular-apis`)
- Community-contributed native modules for most-used APIs
- Auto-generate module scaffolds from indexed APIs
- More native modules: slack, stripe, sentry, datadog, pagerduty, etc.

---

## Token Math

### Scenario: 5 services, user needs 3 of them in a session

**Raw MCP (today):**

```
Idle:     61 tools × ~600 tokens = 36,600 tokens (always)
Active:   3 operations × 1 round-trip = 3 round-trips
Total:    ~40,200 tokens, 3 round-trips
```

**Anthropic Tool Search:**

```
Idle:     ~2,000 tokens (search tool + deferred refs)
Search:   3 searches × ~800 tokens = 2,400 tokens
Active:   3 operations × 1 round-trip = 3 round-trips
Total:    ~5,600 tokens, 6 round-trips
```

**Assay (skill + Bash):**

```
Idle:     ~100 tokens (skill description only)
Search:   1 assay search × ~400 tokens = 400 tokens (covers all 3 services)
Active:   1 assay exec × ~200 tokens = 200 tokens (1 script does all 3)
Total:    ~700 tokens, 2 Bash calls
```

**Improvement over raw MCP:** 98% fewer tokens, 33% fewer round-trips **Improvement over Tool
Search:** 87% fewer tokens, 67% fewer round-trips
