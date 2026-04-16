# Workflow examples

Runnable examples of the `assay serve` workflow engine + the `assay.workflow` Lua client. Each
subdirectory is self-contained: a worker script, a README explaining what it does, and the exact
commands to run it against a local engine.

All three examples assume:

```sh
# In one terminal: start the engine (ships data in ./assay-workflow.db)
assay serve

# In another terminal: cd into the example and run its worker
cd examples/workflows/hello-workflow
assay run worker.lua
```

Then you can drive workflows from the CLI (`assay workflow start ...`), from any HTTP client, or
from the dashboard at <http://localhost:8080/workflow/>.

| Example                                      | What it shows                                                                                    |
| -------------------------------------------- | ------------------------------------------------------------------------------------------------ |
| [`hello-workflow/`](./hello-workflow/)       | Smallest possible workflow — one activity, one return value. Start here.                         |
| [`approval-pipeline/`](./approval-pipeline/) | Wait for a human signal between two activities. Pause / resume from the dashboard.               |
| [`nightly-report/`](./nightly-report/)       | Cron-fired workflow + `side_effect` for non-determinism + a child workflow per detected anomaly. |

Tested against the in-tree engine — when this directory's worker scripts diverge from what the
engine accepts, the `tests/orchestration.rs` suite in `crates/assay-workflow/` will catch it.
