pub mod activities;
pub mod events;
pub mod namespaces;
pub mod openapi;
pub mod public;
pub mod queues;
pub mod schedules;
pub mod tasks;
pub mod workers;
pub mod workflow_tasks;
pub mod workflows;

use std::sync::Arc;

use axum::Router;

use crate::ctx::WorkflowCtx;
use crate::store::WorkflowStore;

/// Build the workflow HTTP API router.
///
/// Three tiers, all under `/api/v1/engine/workflow/*`:
///
/// 1. **Authenticated** — workflows, schedules, namespaces, activities,
///    tasks, workers, queues, events. Authentication + authorization
///    happens at the engine layer (via [`assay_auth::gate`] in
///    `assay_engine::server`); the workflow router itself carries no
///    gate. (The `workflow.api_keys` surface was retired in plan-15
///    slice 3 — auth tokens come from the auth module now.)
/// 2. **Public** — health, version. Always unauthenticated so
///    Kubernetes probes, load balancers, and third-party monitors can
///    reach them without a bearer token.
/// 3. **OpenAPI** — `openapi.json` + `docs` HTML. Always public.
///
/// The standalone `serve*` entry points were removed in plan-15
/// slice 2 — workflow now only runs inside `assay-engine`, which owns
/// the listener, the auth gate, and the surrounding state.
pub fn router<S: WorkflowStore>(state: Arc<WorkflowCtx<S>>) -> Router {
    let authed_api = Router::new()
        .nest("/api/v1/engine/workflow", api_v1_router::<S>())
        .nest("/api/v1/engine/workflow", events::router::<S>());

    let public_api = Router::new().nest("/api/v1/engine/workflow", public::router::<S>());

    let app = authed_api.merge(public_api).merge(openapi::router::<S>());

    app.with_state(state)
}

fn api_v1_router<S: WorkflowStore>() -> Router<Arc<WorkflowCtx<S>>> {
    Router::new()
        .merge(workflows::router::<S>())
        .merge(activities::router::<S>())
        .merge(workflow_tasks::router::<S>())
        .merge(tasks::router::<S>())
        .merge(schedules::router::<S>())
        .merge(workers::router::<S>())
        .merge(namespaces::router::<S>())
        .merge(queues::router::<S>())
}
