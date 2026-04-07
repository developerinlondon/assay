# Assay

Assay executes Lua workflows for infrastructure automation, AI agent orchestration, and resumable approval flows.

## When to use Assay

- Multi-step infrastructure tasks that should live in a checked-in Lua script
- Scheduled or repeatable workflows with deterministic behavior
- Data processing or service automation that benefits from Assay's builtin HTTP, JSON, DB, and filesystem support
- Tasks that may pause for human approval and resume later
- Workflows that need Assay stdlib modules for Kubernetes, GitOps, observability, identity, storage, or AI agents

## When not to use Assay

- Simple one-liners better handled directly with shell or an existing OpenClaw tool
- Interactive debugging where the agent should inspect each command step-by-step
- Ad hoc tasks with no reusable script or workflow value
- Cases where you need broad host access outside the configured `scriptsDir`

## Tool contract

Use the `assay` OpenClaw tool.

### `run`

Runs a Lua script from `scriptsDir`.

```json
{
  "action": "run",
  "script": "workflows/deploy.lua",
  "args": {
    "APP_NAME": "payments",
    "TARGET_ENV": "prod"
  }
}
```

- `script` must be relative to `scriptsDir`
- `args` becomes environment variables for the Assay subprocess
- The tool returns the Assay JSON envelope

### `resume`

Resumes a workflow that halted for approval.

```json
{
  "action": "resume",
  "token": "<resumeToken>",
  "approve": "yes"
}
```

## Approval workflow pattern

When Assay returns `status: "needs_approval"`:

1. Read `requiresApproval.prompt`
2. Summarize `requiresApproval.context` for the user
3. Ask the user for approval if they have not already provided it
4. Call `resume` with the returned `resumeToken`
5. Use `approve: "yes"` to continue or `approve: "no"` to reject

## Error handling pattern

- `status: "ok"` means the script completed successfully
- `status: "needs_approval"` means execution paused and expects `resume`
- `status: "error"` means the script failed; report the error and fix the script or inputs
- `status: "timeout"` means the workflow exceeded the configured timeout; simplify the workflow or raise the timeout
- If stdout contains debug lines before JSON, trust the final JSON envelope

## Stdlib modules quick reference

### Infrastructure

- `assay.k8s` - Kubernetes resources, CRDs, readiness checks, pod and rollout automation
- `assay.argocd` - ArgoCD apps, syncs, health, projects, repositories, clusters
- `assay.kargo` - Kargo stages, freight, promotions, warehouses, verification status
- `assay.vault` - Vault KV secrets, policies, auth, transit, PKI, token flows
- `assay.openbao` - OpenBao-compatible Vault API access for secrets and policies
- `assay.prometheus` - PromQL queries, alerts, rules, targets, labels, series
- `assay.alertmanager` - Alert silences, receivers, routes, config, alert inspection
- `assay.loki` - LogQL queries, labels, series, tailing, log pushes
- `assay.grafana` - Health, dashboards, datasources, folders, annotations, alert rules
- `assay.flux` - Flux GitRepositories, Kustomizations, HelmReleases, notifications
- `assay.traefik` - Routers, services, middlewares, entrypoints, TLS status
- `assay.certmanager` - Certificates, issuers, ACME orders, ACME challenges
- `assay.eso` - ExternalSecrets and SecretStore synchronization status
- `assay.dex` - Dex OIDC discovery, JWKS, health, config validation
- `assay.crossplane` - Providers, XRDs, compositions, managed resources
- `assay.velero` - Backups, restores, schedules, storage locations
- `assay.temporal` - Workflows, task queues, schedules, signals
- `assay.harbor` - Projects, repositories, artifacts, vulnerability scanning

### Services

- `assay.healthcheck` - HTTP checks, JSON path assertions, latency checks, multi-check runs
- `assay.s3` - S3-compatible object storage for AWS, R2, MinIO, and similar services
- `assay.postgres` - PostgreSQL helper workflows for users, databases, grants, Vault integration
- `assay.zitadel` - Zitadel identity management for projects, apps, users, policies, IdPs
- `assay.unleash` - Feature flags, projects, environments, strategies, API tokens

### AI agent and SaaS automation

- `assay.openclaw` - OpenClaw tool invocation, state, diff, approval, and LLM task automation
- `assay.github` - GitHub issues, pull requests, repos, actions, GraphQL workflows
- `assay.gmail` - Gmail search, read, send, reply, label management with OAuth2
- `assay.gcal` - Google Calendar event CRUD and calendar listing with OAuth2
- `assay.oauth2` - OAuth2 token acquisition and refresh helpers for API integrations
- `assay.email_triage` - Email classification and structured triage workflows for agent pipelines

## Usage examples

### GitOps verification

```json
{
  "action": "run",
  "script": "gitops/verify-release.lua",
  "args": {
    "APP": "api",
    "STAGE": "prod"
  }
}
```

### Approval-gated deployment

```json
{
  "action": "run",
  "script": "deploy/promote.lua",
  "args": {
    "SERVICE": "billing"
  }
}
```

If it returns `needs_approval`, resume with:

```json
{
  "action": "resume",
  "token": "<resumeToken>",
  "approve": "yes"
}
```

### SaaS triage workflow

```json
{
  "action": "run",
  "script": "agents/email-triage.lua",
  "args": {
    "MAILBOX": "ops@example.com"
  }
}
```
