//! Signal and event-history methods.

use anyhow::Result;

use crate::ctx::{WorkflowCtx, timestamp_now};
use crate::events::WorkflowBusEvent;
use crate::store::WorkflowStore;
use crate::types::*;

impl<S: WorkflowStore> WorkflowCtx<S> {
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

        // so the worker can replay and notice the signal in history. The
        // helper also emits WorkflowNeedsDispatch on the engine event bus.
        self.mark_and_emit_needs_dispatch(workflow_id).await?;

        // Emit so the dashboard can refresh the run's row (signal
        // count bump, log-tail tick, etc.).
        let ns = self
            .store
            .get_workflow(workflow_id)
            .await?
            .map(|w| w.namespace)
            .unwrap_or_default();
        self.emit(
            &ns,
            WorkflowBusEvent::SignalReceived {
                workflow_id: workflow_id.to_string(),
                signal_name: name.to_string(),
            },
        )
        .await;

        Ok(())
    }

    pub async fn get_events(&self, workflow_id: &str) -> Result<Vec<WorkflowEvent>> {
        self.store.list_events(workflow_id).await
    }
}
