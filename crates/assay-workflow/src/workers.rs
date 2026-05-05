//! Worker registry methods.

use anyhow::Result;

use crate::ctx::{WorkflowCtx, timestamp_now};
use crate::store::WorkflowStore;
use crate::types::*;

impl<S: WorkflowStore> WorkflowCtx<S> {
    pub async fn register_worker(&self, worker: &WorkflowWorker) -> Result<()> {
        self.store.register_worker(worker).await
    }

    pub async fn heartbeat_worker(&self, id: &str) -> Result<()> {
        self.store.heartbeat_worker(id, timestamp_now()).await
    }

    pub async fn list_workers(&self, namespace: &str) -> Result<Vec<WorkflowWorker>> {
        self.store.list_workers(namespace).await
    }
}
