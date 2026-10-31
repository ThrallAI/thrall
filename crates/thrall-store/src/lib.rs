//! Storage behind one `Store` trait: SQLite locally, Postgres on servers (see `docs/storage.md`).

pub mod constants;
pub mod error;
pub mod models;
pub mod sqlite;
pub mod store;

pub use error::StoreError;
pub use models::{
    AgentRow, AuditEntry, Checkpoint, Device, Episode, Job, JobRun, Knowledge, MemoryKind,
    NewEvent, NewMessage, NewSession, NewStep, Person, PersonTerms, Session, SessionGroup,
    StoredEvent, StoredMessage, StoredVector, TaskRow, TextHit, TurnEnd, UsageRecord, UsageTotals,
};
pub use sqlite::SqliteStore;
pub use store::{Store, StoreFuture};
