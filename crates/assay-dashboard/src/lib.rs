//! Dashboard — typed asset bundle + axum router composition.
//!
//! Feature flags:
//!  - `workflow` (default): workflow run lists, events, timers, retries
//!  - `auth`: user + session + Zanzibar + OIDC client registry views
//!
//! The real asset relocation lands in plan 12a Task 1.5 alongside the
//! `DashboardCtx` state refactor. This file is a scaffold until then.
