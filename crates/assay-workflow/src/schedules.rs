//! Schedule CRUD methods.

use anyhow::Result;

use crate::ctx::WorkflowCtx;
use crate::store::WorkflowStore;
use crate::types::*;

impl<S: WorkflowStore> WorkflowCtx<S> {
    /// Persist a new schedule. Returns the stored record, whose `next_run_at`
    /// is seeded from the cron expression so the first run happens at the
    /// first cron time rather than at registration.
    pub async fn create_schedule(&self, schedule: &WorkflowSchedule) -> Result<WorkflowSchedule> {
        let mut stored = schedule.clone();
        stored.next_run_at = crate::scheduler::seed_next_run(schedule);
        self.store.create_schedule(&stored).await?;
        Ok(stored)
    }

    pub async fn list_schedules(&self, namespace: &str) -> Result<Vec<WorkflowSchedule>> {
        self.store.list_schedules(namespace).await
    }

    pub async fn get_schedule(
        &self,
        namespace: &str,
        name: &str,
    ) -> Result<Option<WorkflowSchedule>> {
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
        self.store
            .set_schedule_paused(namespace, name, paused)
            .await
    }
}
