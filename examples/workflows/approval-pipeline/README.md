# approval-pipeline

A two-step workflow that pauses for a human approval signal between build and deploy. Demonstrates
`ctx:wait_for_signal` and how a workflow can sit indefinitely without consuming worker resources.

## What it does

```
ApproveAndDeploy(input)
  ├─> activity "build"          →  produce {image, sha}
  ├─> wait_for_signal "approve" →  receive {by = "alice"}
  └─> activity "deploy"         →  return {url, approver}
```

While waiting for `approve`, the workflow's status in the dashboard is `RUNNING` and an event
`WorkflowAwaitingSignal` is recorded — but no worker is busy on it. Send the signal to wake it up.

## Run

```sh
# Terminal 1
assay serve

# Terminal 2
cd examples/workflows/approval-pipeline
assay run worker.lua
```

Start a build:

```sh
curl -X POST http://localhost:8080/api/v1/workflows \
  -H 'Content-Type: application/json' \
  -d '{
    "workflow_type": "ApproveAndDeploy",
    "workflow_id": "deploy-prod-001",
    "task_queue": "default",
    "input": {"git_sha": "abc123", "target_env": "production"}
  }'
```

Watch <http://localhost:8080/workflow/> — the workflow runs `build`, then sits at
`WorkflowAwaitingSignal`. Approve it from the CLI:

```sh
assay workflow signal deploy-prod-001 approve '{"by":"alice"}'
```

Within ~1s the workflow completes:

```sh
curl -s http://localhost:8080/api/v1/workflows/deploy-prod-001 | jq .result
# "{\"url\":\"https://.../deploy/...\",\"approver\":\"alice\"}"
```

You can also send the signal from any HTTP client:

```sh
curl -X POST 'http://localhost:8080/api/v1/workflows/deploy-prod-001/signal/approve' \
  -H 'Content-Type: application/json' \
  -d '{"payload": {"by": "bob"}}'
```
