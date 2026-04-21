//! Grouped small operations: signals, event history, workers, schedules,
//! namespaces, and snapshots.

use anyhow::Result;

use super::WorkflowEngine;
use super::timestamp_now;
use crate::store::WorkflowStore;
use crate::types::*;

impl<S: WorkflowStore> WorkflowEngine<S> {
    pub async fn send_signal(
        &self,
        workflow_id: &str,
        name: &str,
        payload: Option<&str>,
    ) -> Result<()> {
        let now = timestamp_now();

        self.store
            .send_signal(&WorkflowSignal {
                id: None,
                workflow_id: workflow_id.to_string(),
                name: name.to_string(),
                payload: payload.map(String::from),
                consumed: false,
                received_at: now,
            })
            .await?;

        let seq = self.store.get_event_count(workflow_id).await? as i32 + 1;
        // Parse the incoming payload string back to a JSON value so the
        // event payload nests cleanly (otherwise the recorded payload is
        // a stringified JSON-inside-JSON and Lua workers would have to
        // double-decode).
        let payload_value: serde_json::Value = payload
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or(serde_json::Value::Null);
        self.store
            .append_event(&WorkflowEvent {
                id: None,
                workflow_id: workflow_id.to_string(),
                seq,
                event_type: "SignalReceived".to_string(),
                payload: Some(
                    serde_json::json!({ "signal": name, "payload": payload_value }).to_string(),
                ),
                timestamp: now,
            })
            .await?;

        // Phase 9: a workflow waiting on this signal needs to be re-dispatched
        // so the worker can replay and notice the signal in history.
        self.store.mark_workflow_dispatchable(workflow_id).await?;

        // Broadcast so the dashboard can refresh the run's row (signal
        // count bump, log-tail tick, etc.) without waiting for the
        // next list poll.
        let ns = self
            .store
            .get_workflow(workflow_id)
            .await?
            .map(|w| w.namespace)
            .unwrap_or_default();
        self.broadcast("signal_received", workflow_id, &ns);

        Ok(())
    }

    pub async fn get_events(&self, workflow_id: &str) -> Result<Vec<WorkflowEvent>> {
        self.store.list_events(workflow_id).await
    }

    pub async fn register_worker(&self, worker: &WorkflowWorker) -> Result<()> {
        self.store.register_worker(worker).await
    }

    pub async fn heartbeat_worker(&self, id: &str) -> Result<()> {
        self.store.heartbeat_worker(id, timestamp_now()).await
    }

    pub async fn list_workers(&self, namespace: &str) -> Result<Vec<WorkflowWorker>> {
        self.store.list_workers(namespace).await
    }

    pub async fn create_schedule(&self, schedule: &WorkflowSchedule) -> Result<()> {
        self.store.create_schedule(schedule).await
    }

    pub async fn list_schedules(&self, namespace: &str) -> Result<Vec<WorkflowSchedule>> {
        self.store.list_schedules(namespace).await
    }

    pub async fn get_schedule(&self, namespace: &str, name: &str) -> Result<Option<WorkflowSchedule>> {
        self.store.get_schedule(namespace, name).await
    }

    pub async fn delete_schedule(&self, namespace: &str, name: &str) -> Result<bool> {
        self.store.delete_schedule(namespace, name).await
    }

    pub async fn update_schedule(
        &self,
        namespace: &str,
        name: &str,
        patch: &SchedulePatch,
    ) -> Result<Option<WorkflowSchedule>> {
        self.store.update_schedule(namespace, name, patch).await
    }

    pub async fn set_schedule_paused(
        &self,
        namespace: &str,
        name: &str,
        paused: bool,
    ) -> Result<Option<WorkflowSchedule>> {
        self.store.set_schedule_paused(namespace, name, paused).await
    }

    pub async fn create_namespace(&self, name: &str) -> Result<()> {
        self.store.create_namespace(name).await
    }

    pub async fn list_namespaces(&self) -> Result<Vec<crate::store::NamespaceRecord>> {
        self.store.list_namespaces().await
    }

    pub async fn delete_namespace(&self, name: &str) -> Result<bool> {
        self.store.delete_namespace(name).await
    }

    pub async fn get_namespace_stats(&self, namespace: &str) -> Result<crate::store::NamespaceStats> {
        self.store.get_namespace_stats(namespace).await
    }

    pub async fn get_queue_stats(&self, namespace: &str) -> Result<Vec<crate::store::QueueStats>> {
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

    pub async fn get_latest_snapshot(
        &self,
        workflow_id: &str,
    ) -> Result<Option<WorkflowSnapshot>> {
        self.store.get_latest_snapshot(workflow_id).await
    }
}
