//! Engine-wide CDC outbox. Every state-mutating store method writes a
//! typed event via a subsystem wrapper (e.g. `WorkflowEventBus`) that
//! in turn calls [`EngineEventBus::publish_committed`]. Subscribers —
//! scheduler, task workers, SSE dashboards — consume from a node-local
//! `tokio::broadcast` fed by same-node writes plus a single PG `LISTEN`
//! bridge for cross-node bumps. Events are durable in the `engine_events`
//! table with a configurable TTL (default 3 days) so reconnecting
//! clients can replay from a cursor (`Last-Event-ID` in SSE, `after:
//! Option<i64>` on the bus).

pub mod trait_;
pub use trait_::*;

#[cfg(feature = "backend-postgres")]
pub mod pg;

#[cfg(feature = "backend-postgres")]
pub use pg::PgEngineEventBus;

#[cfg(feature = "backend-sqlite")]
pub mod sqlite;

#[cfg(feature = "backend-sqlite")]
pub use sqlite::SqliteEngineEventBus;
