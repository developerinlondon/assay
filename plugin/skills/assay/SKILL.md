---
description: Inspect and operate infrastructure — Kubernetes, ArgoCD, Vault, Prometheus, Alertmanager, GitLab, GitHub, AWS, Grafana, Loki, databases, and more — by writing short Lua scripts run through the gated `assay_run` tool. Use whenever a task needs to read or change cloud/cluster/CI state. Discover module APIs first with `assay_context`.
---

# assay — gated infrastructure toolkit

This plugin exposes two MCP tools backed by the `assay` runtime. Instead of a separate tool per CLI,
you compose infrastructure operations as small Lua scripts, and every run is gated.

## The two tools

- **`assay_context(query)`** — search assay's modules and return prompt-ready docs (method
  signatures, return shapes, env vars). Call this FIRST to learn a module's API before writing a
  script. Example queries: `kubernetes`, `argocd`, `vault`, `prometheus`, `sonarqube`, `aws`.
- **`assay_run(script, mode)`** — run a Lua script and get back a JSON envelope (`status`: `ok` |
  `needs_approval` | `error`). Every embedded module is available via `require("assay.<module>")`
  alongside the builtins (`http`, `json`, `fs`, `crypto`, …). Return a value from the script to
  surface it.

## Gating — always pick a mode

- `mode = "readonly"` (default): mutating operations are blocked. Use this for all investigation and
  inspection.
- `mode = "approval"`: each mutating operation suspends and returns a resume token so a human (or
  your caller) approves it per-operation. Use this only when a change is intended.
- Unrestricted execution is never available — you cannot bypass the gate.

## How to work

1. `assay_context("<service>")` to find the module and methods.
2. Write a small Lua script that does the whole step (list → filter → correlate) and `return` the
   result, so you get structured data in one call instead of many.
3. Run it with `assay_run(script, "readonly")`. If you intend a change, use `"approval"` and handle
   the `needs_approval` envelope.

### Example — read-only investigation

```lua
local k8s = require("assay.k8s")
local pods = k8s.resources:list("pods", "my-namespace")
local unhealthy = {}
for _, p in ipairs(pods.items or {}) do
  if p.status and p.status.phase ~= "Running" then
    unhealthy[#unhealthy + 1] = { name = p.metadata.name, phase = p.status.phase }
  end
end
return unhealthy
```

Prefer one composed script over many round-trips. Read the module docs with `assay_context` rather
than guessing method names.
