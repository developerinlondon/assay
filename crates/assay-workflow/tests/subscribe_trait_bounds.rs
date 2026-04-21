//! Compile-time assertion that subscribe_runnable + subscribe_tasks
//! exist on WorkflowStore and produce Send streams.
//!
//! Runtime behaviour is tested per-backend in Phase 3.

use assay_workflow::store::WorkflowStore;
use futures_core::Stream;

fn _assert_runnable<S: WorkflowStore>(s: &S, ns: &str) {
    let _: std::pin::Pin<Box<dyn Stream<Item = String> + Send>> =
        Box::pin(s.subscribe_runnable(ns));
}

fn _assert_tasks<S: WorkflowStore>(s: &S, queues: &[&str]) {
    let _: std::pin::Pin<Box<dyn Stream<Item = String> + Send>> =
        Box::pin(s.subscribe_tasks(queues));
}

#[test]
fn trait_surface_compiles() {
    // If this file compiles, the trait methods exist with the expected
    // Send bounds. The real test is the compile check — this fn is a
    // no-op at runtime.
}
