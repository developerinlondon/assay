# hello-workflow

The simplest possible assay workflow: one activity that says hello.

## What it does

```
GreetWorkflow(input)
  └─> activity "greet"  →  { message = "hello, " .. input.who }
  return { greeting = ... }
```

## Run

In one terminal:

```sh
assay serve
```

In another, from this directory:

```sh
assay run worker.lua
```

In a third, start a workflow:

```sh
curl -X POST http://localhost:8080/api/v1/workflows \
  -H 'Content-Type: application/json' \
  -d '{
    "workflow_type": "GreetWorkflow",
    "workflow_id": "hello-1",
    "task_queue": "default",
    "input": {"who": "world"}
  }'
```

Within ~1 second:

```sh
curl -s http://localhost:8080/api/v1/workflows/hello-1 | jq .result
# "{\"greeting\":\"hello, world\"}"
```

The dashboard at <http://localhost:8080/workflow/> shows it under **Workflows**; click the row to
see the event timeline (`WorkflowStarted` → `ActivityScheduled` → `ActivityCompleted` →
`WorkflowCompleted`).
