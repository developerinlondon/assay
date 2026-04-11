## async

Async task management. No `require()` needed.

- `async.spawn(fn)` → handle — Spawn async task, returns handle
- `async.spawn_interval(fn, ms)` → handle — Spawn recurring task every `ms` milliseconds
- `handle:await()` → result — Wait for task completion, returns result
- `handle:cancel()` → nil — Cancel recurring task
