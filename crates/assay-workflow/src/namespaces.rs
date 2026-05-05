//! Namespace CRUD + snapshot methods.

use anyhow::Result;

use crate::ctx::WorkflowCtx;
use crate::store::WorkflowStore;
use crate::store::{NamespaceRecord, NamespaceStats, QueueStats};
use crate::types::*;

impl<S: WorkflowStore> WorkflowCtx<S> {
    pub async fn create_namespace(&self, name: &str) -> Result<()> {
        self.store.create_namespace(name).await
    }

    pub async fn list_namespaces(&self) -> Result<Vec<NamespaceRecord>> {
        self.store.list_namespaces().await
    }

    pub async fn delete_namespace(&self, name: &str) -> Result<bool> {
        self.store.delete_namespace(name).await
    }

    pub async fn get_namespace_stats(&self, namespace: &str) -> Result<NamespaceStats> {
        self.store.get_namespace_stats(namespace).await
    }

    pub async fn get_queue_stats(&self, namespace: &str) -> Result<Vec<QueueStats>> {
        self.store.get_queue_stats(namespace).await
    }

    pub async fn create_snapshot(
        &self,
        workflow_id: &str,
        event_seq: i32,
        state_json: &str,
    ) -> Result<()> {
        self.store
            .create_snapshot(workflow_id, event_seq, state_json)
            .await
    }

    pub async fn get_latest_snapshot(&self, workflow_id: &str) -> Result<Option<WorkflowSnapshot>> {
        self.store.get_latest_snapshot(workflow_id).await
    }
}
