# Dashboard Redesign — assay workflow

## Overview

Redesign the workflow engine dashboard from a bare 3-tab top-nav layout to a production-grade admin
panel with left sidebar, namespace management, real-time updates, and proper workflow inspection
tools.

## Layout

```
┌──────────────────────────────────────────────────────────────────────────┐
│  assay workflow                                        ● Connected      │
├──────────────┬───────────────────────────────────────────────────────────┤
│              │                                                           │
│  ┌─────────┐ │  [Active View Content]                                    │
│  │  main  ▼│ │                                                           │
│  └─────────┘ │                                                           │
│              │                                                           │
│  ▸ Workflows │                                                           │
│    Schedules │                                                           │
│    Workers   │                                                           │
│    Queues    │                                                           │
│    Settings  │                                                           │
│              │                                                           │
│              │                                                           │
│              │                                                           │
│              │                                                           │
├──────────────┴───────────────────────────────────────────────────────────┤
│  Engine: localhost:8085  │  Namespace: main  │  Workers: 3  │  v0.11.1  │
└──────────────────────────────────────────────────────────────────────────┘
```

### Sidebar (left, fixed)

- **Namespace switcher** — dropdown at top, shows current namespace, switch between
  main/staging/production etc.
- **Navigation links** — Workflows, Schedules, Workers, Queues, Settings
- Active item highlighted with accent color
- Collapsed on mobile (hamburger menu)

### Status bar (bottom, fixed)

- Engine URL
- Active namespace
- Connected worker count
- Engine version
- Green pulse dot for SSE connection status

## Views

### 1. Workflows View (default)

```
┌─────────────────────────────────────────────────────────────────────┐
│  Workflows                               [Search...] [Status ▼]   │
│                                                                     │
│  ┌─────────────────────────────────────────────────────────────┐   │
│  │ ID            Type         Status    Queue   Created  Actions│   │
│  ├─────────────────────────────────────────────────────────────┤   │
│  │ ingest-45     IngestData   ● RUN     data    2m ago   [···] │   │
│  │ deploy-12     Deploy       ● DONE    deploy  15m ago        │   │
│  │ order-789     ProcessOrder ● PEND    main    30s ago  [···] │   │
│  │ batch-33      IngestData   ● FAIL    data    1h ago         │   │
│  │ approval-7    Approval     ● WAIT    ops     5m ago   [···] │   │
│  └─────────────────────────────────────────────────────────────┘   │
│                                                                     │
│  Showing 5 of 128 workflows                    [< 1 2 3 4 >]      │
└─────────────────────────────────────────────────────────────────────┘
```

- **Search** — filter by workflow ID or type (client-side)
- **Status filter** — dropdown: All, Pending, Running, Waiting, Completed, Failed
- **Table columns** — ID (clickable), Type, Status (colored badge), Queue, Created (relative),
  Actions
- **Actions** — Signal, Cancel (only for non-terminal workflows)
- **Pagination** — 20 per page, prev/next + page numbers
- **Live updates** — SSE refreshes table when workflow events occur

### 2. Workflow Detail (slide-out panel)

Triggered by clicking a workflow ID. Slides in from the right.

```
┌─────────────────────────────────────────────────────┐
│  ingest-45                          [Signal] [Cancel]│
├─────────────────────────────────────────────────────┤
│                                                      │
│  Status       ● RUNNING                              │
│  Type         IngestData                             │
│  Namespace    main                                   │
│  Queue        data                                   │
│  Run ID       run-ingest-45-1713220617               │
│  Created      2026-04-15 23:03:17                    │
│  Claimed By   worker-pod-3                           │
│                                                      │
│  ┌─ Input ──────────────────────────────────────┐   │
│  │ {                                             │   │
│  │   "source": "s3://data-lake/batch-45",       │   │
│  │   "format": "parquet"                         │   │
│  │ }                                             │   │
│  └───────────────────────────────────────────────┘   │
│                                                      │
│  ┌─ Result ─────────────────────────────────────┐   │
│  │ (pending)                                     │   │
│  └───────────────────────────────────────────────┘   │
│                                                      │
│  Event Timeline (4)                                  │
│  ─────────────────                                   │
│  #1  WorkflowStarted           23:03:17              │
│      ▸ {"source": "s3://..."}                        │
│  #2  ActivityScheduled          23:03:17              │
│      ▸ {"name": "fetch_s3", "queue": "data"}        │
│  #3  ActivityCompleted          23:03:22              │
│      ▸ {"result": {"rows": 1420}}                    │
│  #4  ActivityScheduled          23:03:22              │
│      ▸ {"name": "load_warehouse"}                    │
│                                                      │
│  Child Workflows (1)                                 │
│  ─────────────────                                   │
│  └─ enrich-45  EnrichML  ● RUNNING  gpu-tasks       │
│                                                      │
└─────────────────────────────────────────────────────┘
```

- **Metadata grid** — key/value pairs for all workflow fields
- **Input/Result** — collapsible JSON viewer with syntax highlighting (monospace, indented)
- **Error** — shown in red if workflow failed
- **Event timeline** — ordered by seq, each event expandable (click ▸ to show payload JSON)
- **Child workflows** — clickable list, shown as tree if nested
- **Action buttons** — Signal (prompt for name + payload), Cancel (confirm dialog)

### 3. Schedules View

```
┌─────────────────────────────────────────────────────────────────────┐
│  Schedules                                             [+ Create]  │
│                                                                     │
│  ┌─────────────────────────────────────────────────────────────┐   │
│  │ Name           Type        Cron         Queue   Last Run    │   │
│  ├─────────────────────────────────────────────────────────────┤   │
│  │ hourly-ingest  IngestData  0 * * * *    data    14:00       │   │
│  │ daily-report   GenReport   0 9 * * *    main    09:00       │   │
│  │ cleanup        Cleanup     0 0 * * 0    main    Sun 00:00   │   │
│  └─────────────────────────────────────────────────────────────┘   │
│                                                                     │
│  ┌─ Create Schedule ────────────────────────────────────────────┐  │
│  │  Name:          [________________]                           │  │
│  │  Workflow Type: [________________]                           │  │
│  │  Cron:          [________________]  (e.g. 0 * * * *)        │  │
│  │  Queue:         [main___________]                            │  │
│  │  Input (JSON):  [________________]                           │  │
│  │                                          [Create Schedule]   │  │
│  └──────────────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────────────┘
```

- **Table** — name, type, cron expression, queue, last run time
- **Actions** — Delete (per row, with confirm)
- **Create form** — inline form below the table, collapsible via [+ Create] button
- **Cron helper** — show human-readable description of cron expression

### 4. Workers View

```
┌─────────────────────────────────────────────────────────────────────┐
│  Workers                                                            │
│                                                                     │
│  ┌─────────────────────────────────────────────────────────────┐   │
│  │ ID        Identity       Queue   Active  Last Heartbeat     │   │
│  ├─────────────────────────────────────────────────────────────┤   │
│  │ w-abc1    pipeline-pod-1 data    3/10    5s ago    ● alive  │   │
│  │ w-def2    pipeline-pod-2 data    1/10    3s ago    ● alive  │   │
│  │ w-ghi3    deploy-bot-1   deploy  0/10    45s ago   ● alive  │   │
│  └─────────────────────────────────────────────────────────────┘   │
│                                                                     │
│  3 workers connected                                                │
└─────────────────────────────────────────────────────────────────────┘
```

- **Columns** — ID, identity (pod name), queue, active tasks/max, last heartbeat (relative), status
  dot
- **Status dot** — green if heartbeat < 30s, yellow if 30-60s, red if > 60s
- **Auto-refresh** — table refreshes every 10s

### 5. Queues View

```
┌─────────────────────────────────────────────────────────────────────┐
│  Task Queues                                                        │
│                                                                     │
│  ┌─────────────────────────────────────────────────────────────┐   │
│  │ Queue     Pending   Running   Workers                       │   │
│  ├─────────────────────────────────────────────────────────────┤   │
│  │ data      12        3         2                              │   │
│  │ deploy    0         1         1                              │   │
│  │ main      5         0         0          ⚠ no workers       │   │
│  │ gpu       0         0         0          ⚠ no workers       │   │
│  └─────────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────────┘
```

- **Columns** — queue name, pending activity count, running activity count, worker count
- **Warning** — show ⚠ when pending > 0 but workers = 0

### 6. Settings View

```
┌─────────────────────────────────────────────────────────────────────┐
│  Settings                                                           │
│                                                                     │
│  Namespaces                                            [+ Create]  │
│  ┌─────────────────────────────────────────────────────────────┐   │
│  │ Name         Workflows  Schedules  Workers  Created         │   │
│  ├─────────────────────────────────────────────────────────────┤   │
│  │ main         128        3          5        Apr 10   [   ]  │   │
│  │ staging      12         1          2        Apr 12   [Del]  │   │
│  │ production   450        8          12       Apr 1    [Del]  │   │
│  └─────────────────────────────────────────────────────────────┘   │
│                                                                     │
│  Engine Info                                                        │
│  ──────────                                                         │
│  Auth Mode:    no-auth                                              │
│  Backend:      sqlite://assay-workflow.db                           │
│  Version:      0.11.1                                               │
│  Uptime:       2h 15m                                               │
│  API Docs:     /api/v1/docs                                         │
│  OpenAPI Spec: /api/v1/openapi.json                                 │
└─────────────────────────────────────────────────────────────────────┘
```

- **Namespace table** — name, workflow/schedule/worker counts (from stats endpoint), created date
- **Create** — inline input field
- **Delete** — per row, with confirm. Cannot delete "main"
- **Engine info** — read-only metadata about the running engine

## Theme — Dark + Light Mode

Auto-detects system preference via `prefers-color-scheme`. Manual toggle button (sun/moon icon) in
the header. Preference saved to `localStorage`.

### Dark theme (default)

| Token        | Value   | Usage                       |
| ------------ | ------- | --------------------------- |
| --bg         | #0d1117 | Page background             |
| --surface    | #161b22 | Cards, sidebar, panels      |
| --border     | #30363d | Borders, dividers           |
| --text       | #e6edf3 | Primary text                |
| --text-muted | #8b949e | Secondary text, labels      |
| --accent     | #58a6ff | Links, active nav, focus    |
| --green      | #3fb950 | Completed, alive, connected |
| --red        | #f85149 | Failed, errors              |
| --yellow     | #d29922 | Waiting, warning            |
| --orange     | #db6d28 | Cancelled                   |

### Light theme

| Token        | Value   | Usage                       |
| ------------ | ------- | --------------------------- |
| --bg         | #ffffff | Page background             |
| --surface    | #f6f8fa | Cards, sidebar, panels      |
| --border     | #d1d9e0 | Borders, dividers           |
| --text       | #1f2328 | Primary text                |
| --text-muted | #656d76 | Secondary text, labels      |
| --accent     | #0969da | Links, active nav, focus    |
| --green      | #1a7f37 | Completed, alive, connected |
| --red        | #cf222e | Failed, errors              |
| --yellow     | #9a6700 | Waiting, warning            |
| --orange     | #bc4c00 | Cancelled                   |

### Implementation

```css
:root { /* dark by default */ }
[data-theme="light"] { /* light overrides */ }
@media (prefers-color-scheme: light) {
    :root:not([data-theme="dark"]) { /* auto light */ }
}
```

Toggle button in header sets `data-theme` on `<html>` and persists to `localStorage('assay-theme')`.

## Tech Stack

- **HTML/CSS/JS** — vanilla, no build step, no npm
- **htmx** — CDN (~14KB), handles SSE subscription + DOM updates
- **Embedded** — all assets via `include_str!` in Rust, no separate deploy
- **Dark theme** — GitHub-inspired, CSS custom properties for all tokens
- **Responsive** — sidebar collapses on mobile

## API Endpoints Used

| View              | Endpoint                            | Method    |
| ----------------- | ----------------------------------- | --------- |
| Workflows         | /api/v1/workflows?namespace=X       | GET       |
| Workflow detail   | /api/v1/workflows/:id               | GET       |
| Workflow events   | /api/v1/workflows/:id/events        | GET       |
| Workflow children | /api/v1/workflows/:id/children      | GET       |
| Signal            | /api/v1/workflows/:id/signal/:name  | POST      |
| Cancel            | /api/v1/workflows/:id/cancel        | POST      |
| Schedules         | /api/v1/schedules?namespace=X       | GET       |
| Create schedule   | /api/v1/schedules                   | POST      |
| Delete schedule   | /api/v1/schedules/:name?namespace=X | DELETE    |
| Workers           | /api/v1/workers?namespace=X         | GET       |
| Queue stats       | /api/v1/queues?namespace=X          | GET       |
| Namespaces        | /api/v1/namespaces                  | GET       |
| Create namespace  | /api/v1/namespaces                  | POST      |
| Delete namespace  | /api/v1/namespaces/:name            | DELETE    |
| Namespace stats   | /api/v1/namespaces/:name/stats      | GET       |
| SSE stream        | /api/v1/events/stream?namespace=X   | GET (SSE) |
| Health            | /api/v1/health                      | GET       |

## SSE Integration

- Dashboard connects to `/api/v1/events/stream?namespace=X` on load
- Auto-reconnects on disconnect (EventSource built-in)
- Events trigger table refresh for the relevant view:
  - WorkflowStarted/Completed/Failed/Cancelled → refresh workflow list
  - ActivityCompleted → refresh workflow list
- Workflow detail panel refreshes its own events on any event for that workflow ID
- Green pulse dot in status bar indicates live SSE connection
