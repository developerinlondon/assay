# Dashboard end-to-end tests

Headless browser tests covering the assay-workflow dashboard's behaviour
against a real running engine + worker. Live in this directory so anyone
touching the dashboard JS / CSS or the state-snapshot HTTP surface can
add coverage in the same PR.

## Run locally

From `crates/assay-workflow/tests-e2e/`:

```sh
npm install                                  # one-time
npx playwright install --with-deps chromium  # one-time

# In a separate shell — start the engine + demo worker fixtures:
../../../scripts/run-e2e-fixtures.sh         # see helper below

npx playwright test                          # run all specs
npx playwright test --headed                 # watch the browser
npx playwright show-report                   # inspect a failed run
```

The fixtures script is what CI uses too — it boots `assay serve` on
SQLite, registers the `demo` namespace, starts the demo worker that
emits the canonical `pipeline_state.steps[]` shape, and starts a
`DemoPipeline` workflow named `demo-2`. Tests then drive the dashboard.

## What's covered

- Pipeline tab is added when `pipeline_state.steps[]` is present and
  default-selected.
- Five circles render with the correct initial states; the Approval
  step exposes `approve` / `reject` action buttons.
- Clicking an action button POSTs the `step_action` signal, the
  workflow advances, and the dashboard's 1 Hz poller diff-applies the
  new circle / connector / log state.
- Clicking a step circle filters the log to that step's entries.

## Adding new tests

Each new spec lives next to `pipeline.spec.ts`. Reuse the helpers if you
need to switch namespaces or open the demo workflow row. Avoid creating
new long-running workflows in tests — the demo worker already exposes
every status (waiting / running / done / cancelled) so a focused
assertion against it is usually enough.
