# nightly-report

Cron-fired workflow that demonstrates the rest of the workflow engine in one example: a recurring
schedule kicks off the parent, the parent uses `ctx:side_effect` to capture a non-deterministic
report ID, scans for "anomalies", and starts a child workflow per anomaly to handle each in
parallel-ish (each child runs independently against the same engine).

## What it does

```
NightlyReport(input)              ← fired by cron
  ├─> side_effect "issue_report_id" → "rep-2026-04-16-XXXXX"
  ├─> activity "scan_anomalies"     → returns [a1, a2, a3]
  └─> for each anomaly:
        start_child_workflow "HandleAnomaly" {anomaly}
          └─> activity "remediate"  → {fixed = true, anomaly}
  return {report_id, anomalies_handled = 3}
```

`side_effect` makes the report ID stable across worker crashes — even if the worker dies after the
report ID is generated but before the scan runs, the next worker re-replaying the workflow will see
the same report ID from the event log instead of generating a new one.

## Run

```sh
# Terminal 1
assay serve

# Terminal 2
cd examples/workflows/nightly-report
assay run worker.lua
```

Wire up the schedule (one-off; persists in the engine DB):

```sh
# Fires every 5 seconds for the demo. Replace with "0 0 0 * * *" for
# real nightly cadence (the cron crate wants 6/7 fields, with seconds).
curl -X POST http://localhost:8080/api/v1/schedules \
  -H 'Content-Type: application/json' \
  -d '{
    "namespace": "main",
    "name": "nightly-report",
    "workflow_type": "NightlyReport",
    "cron_expr": "*/5 * * * * *",
    "task_queue": "default",
    "input": {"region": "eu-west-1"}
  }'
```

Within ~15s (one scheduler tick) you'll see workflows appearing on the dashboard:

- One `NightlyReport` workflow per fire
- Three `HandleAnomaly` child workflows per `NightlyReport`

Pause the schedule when you've seen enough:

```sh
curl -X POST 'http://localhost:8080/api/v1/schedules/nightly-report/pause?namespace=main'
```

## What to look for

- **In the event log** of a `NightlyReport` workflow: `WorkflowStarted`, `SideEffectRecorded`,
  `ActivityScheduled` (scan), `ActivityCompleted`, `ChildWorkflowStarted` × 3,
  `ChildWorkflowCompleted` × 3, `WorkflowCompleted`.
- **In a child** `HandleAnomaly`: `parent_id` set, normal activity-driven flow.
- **In the workers view**: one worker, registered for the `default` queue, polling both workflow
  tasks and activity tasks.
