//! The `Store` trait: everything the engine asks of storage. Every call takes the owner's
//! `user_id` and only ever touches that user's rows.

use futures::future::BoxFuture;

use crate::error::StoreError;
use crate::models::{
    AgentRow, AuditEntry, Checkpoint, Device, Episode, Job, JobRun, Knowledge, MemoryKind,
    NewEvent, NewMessage, NewSession, NewStep, Person, PersonTerms, Session, SessionGroup,
    StoredEvent, StoredMessage, StoredVector, TaskRow, TextHit, TurnEnd, UsageRecord, UsageTotals,
};

/// Shorthand for a storage call's future.
pub type StoreFuture<'a, T> = BoxFuture<'a, Result<T, StoreError>>;

/// Storage. SQLite today; a server edition could implement the same trait on Postgres.
pub trait Store: Send + Sync {
    /// Create the user row if it doesn't exist.
    fn ensure_user<'a>(&'a self, user_id: &'a str, name: &'a str, at: i64) -> StoreFuture<'a, ()>;

    /// One person, if they exist. (People are the engine's, not one user's: only the engine and
    /// its owner's admin requests use the people calls.)
    fn get_person<'a>(&'a self, user_id: &'a str) -> StoreFuture<'a, Option<Person>>;

    /// Everyone not removed, oldest first.
    fn list_people(&self) -> StoreFuture<'_, Vec<Person>>;

    /// Set a person's role (`owner` or `member`).
    fn set_role<'a>(&'a self, user_id: &'a str, role: &'a str) -> StoreFuture<'a, ()>;

    /// Rename a person. `false` when there's no such person (or they were removed).
    fn rename_person<'a>(&'a self, user_id: &'a str, name: &'a str) -> StoreFuture<'a, bool>;

    /// Set what a person's turns run on and may spend. `false` when there's no such person (or
    /// they were removed).
    fn set_person_terms<'a>(
        &'a self,
        user_id: &'a str,
        terms: &'a PersonTerms,
    ) -> StoreFuture<'a, bool>;

    /// Set what the owner allows a person (`rules`: JSON). `false` when there's no such person (or
    /// they were removed).
    fn set_person_rules<'a>(&'a self, user_id: &'a str, rules: &'a str) -> StoreFuture<'a, bool>;

    /// Remove a person: they can't connect any more, but everything they made is kept. `false`
    /// when there's no such person (or they were already removed).
    fn remove_person<'a>(&'a self, user_id: &'a str, at: i64) -> StoreFuture<'a, bool>;

    /// Sign a device in as `user_id`; `token_hash` is the SHA-256 (hex) of its token.
    fn add_device<'a>(&'a self, device: &'a Device, token_hash: &'a str) -> StoreFuture<'a, ()>;

    /// The signed-in device (not signed out) whose token hashes to `token_hash`; its last seen
    /// time becomes `at`.
    fn use_device<'a>(&'a self, token_hash: &'a str, at: i64) -> StoreFuture<'a, Option<Device>>;

    /// A person's signed-in devices, newest first.
    fn list_devices<'a>(&'a self, user_id: &'a str) -> StoreFuture<'a, Vec<Device>>;

    /// Whether a device is still signed in.
    fn device_signed_in<'a>(&'a self, device_id: &'a str) -> StoreFuture<'a, bool>;

    /// Sign a device out. `false` when it isn't `user_id`'s or was already signed out.
    fn remove_device<'a>(
        &'a self,
        user_id: &'a str,
        device_id: &'a str,
        at: i64,
    ) -> StoreFuture<'a, bool>;

    /// Sign every device of a person out (they were removed).
    fn remove_devices_of<'a>(&'a self, user_id: &'a str, at: i64) -> StoreFuture<'a, ()>;

    /// Create a session.
    fn create_session(&self, new: NewSession) -> StoreFuture<'_, Session>;

    /// One session, if it exists and belongs to the user.
    fn get_session<'a>(
        &'a self,
        user_id: &'a str,
        session_id: &'a str,
    ) -> StoreFuture<'a, Option<Session>>;

    /// The user's sessions, most recently active first.
    fn list_sessions<'a>(&'a self, user_id: &'a str, limit: u32) -> StoreFuture<'a, Vec<Session>>;

    /// Set a session's title.
    fn set_session_title<'a>(
        &'a self,
        user_id: &'a str,
        session_id: &'a str,
        title: &'a str,
    ) -> StoreFuture<'a, ()>;

    /// Set the folder a session works in.
    fn set_session_workspace<'a>(
        &'a self,
        user_id: &'a str,
        session_id: &'a str,
        workspace: &'a str,
    ) -> StoreFuture<'a, ()>;

    /// Archive a session (hidden from the list, kept in full) or bring it back.
    fn set_session_archived<'a>(
        &'a self,
        user_id: &'a str,
        session_id: &'a str,
        archived_at: Option<i64>,
    ) -> StoreFuture<'a, ()>;

    /// The user's archived sessions, most recently archived first.
    fn list_archived_sessions<'a>(
        &'a self,
        user_id: &'a str,
        limit: u32,
    ) -> StoreFuture<'a, Vec<Session>>;

    /// Make a session a page of Home, started on `day` (local `YYYY-MM-DD`). Home pages are left
    /// out of the session lists.
    fn set_session_home<'a>(
        &'a self,
        user_id: &'a str,
        session_id: &'a str,
        day: &'a str,
    ) -> StoreFuture<'a, ()>;

    /// The pages of Home, newest first.
    fn home_sessions<'a>(&'a self, user_id: &'a str) -> StoreFuture<'a, Vec<Session>>;

    /// Pin a session to the top (`Some(when)`) or unpin it.
    fn set_session_pinned<'a>(
        &'a self,
        user_id: &'a str,
        session_id: &'a str,
        pinned_at: Option<i64>,
    ) -> StoreFuture<'a, ()>;

    /// Put a session in one of the user's groups, or take it out (`None`).
    fn set_session_group<'a>(
        &'a self,
        user_id: &'a str,
        session_id: &'a str,
        group_id: Option<&'a str>,
    ) -> StoreFuture<'a, ()>;

    /// The user's session groups, in order.
    fn list_groups<'a>(&'a self, user_id: &'a str) -> StoreFuture<'a, Vec<SessionGroup>>;

    /// Add a group at the end of the list.
    fn create_group<'a>(
        &'a self,
        user_id: &'a str,
        group_id: &'a str,
        name: &'a str,
        at: i64,
    ) -> StoreFuture<'a, SessionGroup>;

    /// Rename a group.
    fn rename_group<'a>(
        &'a self,
        user_id: &'a str,
        group_id: &'a str,
        name: &'a str,
    ) -> StoreFuture<'a, ()>;

    /// Delete a group; its sessions stay, ungrouped.
    fn delete_group<'a>(&'a self, user_id: &'a str, group_id: &'a str) -> StoreFuture<'a, ()>;

    /// Put the user's groups in this order (ids not listed keep their place after these).
    fn reorder_groups<'a>(
        &'a self,
        user_id: &'a str,
        group_ids: &'a [String],
    ) -> StoreFuture<'a, ()>;

    /// Delete a session and everything stored under it (usage records are kept).
    fn delete_session<'a>(&'a self, user_id: &'a str, session_id: &'a str) -> StoreFuture<'a, ()>;

    /// Append an event and return its sequence number. Numbers are allocated atomically.
    fn append_event(&self, event: NewEvent) -> StoreFuture<'_, u64>;

    /// Events after `after_seq`, oldest first, at most `limit`.
    fn events_after<'a>(
        &'a self,
        user_id: &'a str,
        session_id: &'a str,
        after_seq: u64,
        limit: u32,
    ) -> StoreFuture<'a, Vec<StoredEvent>>;

    /// Record that a turn began.
    fn start_turn<'a>(
        &'a self,
        user_id: &'a str,
        session_id: &'a str,
        turn_id: &'a str,
        at: i64,
    ) -> StoreFuture<'a, ()>;

    /// Record how a turn ended.
    fn finish_turn(&self, end: TurnEnd) -> StoreFuture<'_, ()>;

    /// Add a message to the model's context.
    fn add_message(&self, message: NewMessage) -> StoreFuture<'_, ()>;

    /// The session's context messages that haven't been compacted, oldest first.
    fn context_messages<'a>(
        &'a self,
        user_id: &'a str,
        session_id: &'a str,
    ) -> StoreFuture<'a, Vec<StoredMessage>>;

    /// The session's most recent turn and every message the user typed in it, in order (the one
    /// that started it, then any typed while it ran).
    fn last_turn<'a>(
        &'a self,
        user_id: &'a str,
        session_id: &'a str,
    ) -> StoreFuture<'a, Option<(String, Vec<String>)>>;

    /// Take a turn back: the model stops seeing its messages (they stay stored).
    fn retract_turn<'a>(
        &'a self,
        user_id: &'a str,
        session_id: &'a str,
        turn_id: &'a str,
        at: i64,
    ) -> StoreFuture<'a, ()>;

    /// Every message the user typed in a turn, in order (empty if there's no such turn).
    fn turn_messages<'a>(
        &'a self,
        user_id: &'a str,
        session_id: &'a str,
        turn_id: &'a str,
    ) -> StoreFuture<'a, Vec<String>>;

    /// `turn_id` and every turn of the session started after it, oldest first (empty if there's
    /// no such turn).
    fn turns_from<'a>(
        &'a self,
        user_id: &'a str,
        session_id: &'a str,
        turn_id: &'a str,
    ) -> StoreFuture<'a, Vec<String>>;

    /// Keep a file's content from before its turn changed it. Only the first one per turn and
    /// path is kept (that's the content before the turn); false when there was one already.
    fn save_checkpoint(&self, checkpoint: Checkpoint) -> StoreFuture<'_, bool>;

    /// Note what a turn last left in a file (SHA-256 hex) after changing it.
    fn set_checkpoint_after<'a>(
        &'a self,
        user_id: &'a str,
        session_id: &'a str,
        turn_id: &'a str,
        path: &'a str,
        after_hash: &'a str,
    ) -> StoreFuture<'a, ()>;

    /// The checkpoints of `turn_id` and every later turn of the session, oldest turn first.
    fn checkpoints_from<'a>(
        &'a self,
        user_id: &'a str,
        session_id: &'a str,
        turn_id: &'a str,
    ) -> StoreFuture<'a, Vec<Checkpoint>>;

    /// Record one model call's usage.
    fn record_usage(&self, usage: UsageRecord) -> StoreFuture<'_, ()>;

    /// What a user has spent (USD) on usage recorded at or after `since` (Unix ms).
    fn spent_since<'a>(&'a self, user_id: &'a str, since: i64) -> StoreFuture<'a, f64>;

    /// Usage totals for a session, or for the user overall.
    fn usage_totals<'a>(
        &'a self,
        user_id: &'a str,
        session_id: Option<&'a str>,
    ) -> StoreFuture<'a, UsageTotals>;

    /// Every model call a turn made, oldest first (who made it, tokens, cost).
    fn turn_usage<'a>(
        &'a self,
        user_id: &'a str,
        turn_id: &'a str,
    ) -> StoreFuture<'a, Vec<UsageRecord>>;

    /// Record that a tool step began (status `running`).
    fn start_step(&self, step: NewStep) -> StoreFuture<'_, ()>;

    /// Record how a tool step ended.
    fn finish_step<'a>(
        &'a self,
        user_id: &'a str,
        step_id: &'a str,
        status: &'a str,
        summary: &'a str,
        at: i64,
    ) -> StoreFuture<'a, ()>;

    /// Replace a session's task list.
    fn set_tasks<'a>(
        &'a self,
        user_id: &'a str,
        session_id: &'a str,
        tasks: &'a [TaskRow],
        at: i64,
    ) -> StoreFuture<'a, ()>;

    /// A session's task list, in order.
    fn tasks<'a>(&'a self, user_id: &'a str, session_id: &'a str) -> StoreFuture<'a, Vec<TaskRow>>;

    /// Save a long-term fact.
    fn save_knowledge<'a>(
        &'a self,
        user_id: &'a str,
        id: &'a str,
        topic: &'a str,
        content: &'a str,
        at: i64,
    ) -> StoreFuture<'a, ()>;

    /// Set the companion PC a session's tools run on (`None`: this computer).
    fn set_session_pc<'a>(
        &'a self,
        user_id: &'a str,
        session_id: &'a str,
        pc: Option<&'a str>,
    ) -> StoreFuture<'a, ()>;

    /// Set a session's permission mode.
    fn set_session_mode<'a>(
        &'a self,
        user_id: &'a str,
        session_id: &'a str,
        mode: &'a str,
    ) -> StoreFuture<'a, ()>;

    /// Remember an "always" answer for a session.
    fn add_grant<'a>(
        &'a self,
        user_id: &'a str,
        session_id: &'a str,
        id: &'a str,
        tool: &'a str,
        scope: &'a str,
        at: i64,
    ) -> StoreFuture<'a, ()>;

    /// Whether an "always" answer covers `scope` in this session.
    fn has_grant<'a>(
        &'a self,
        user_id: &'a str,
        session_id: &'a str,
        scope: &'a str,
    ) -> StoreFuture<'a, bool>;

    /// Remember a conversation.
    fn add_episode(&self, user_id: &str, episode: Episode) -> StoreFuture<'_, ()>;

    /// Episodes by id.
    fn get_episodes<'a>(
        &'a self,
        user_id: &'a str,
        ids: &'a [String],
    ) -> StoreFuture<'a, Vec<Episode>>;

    /// Current facts by id.
    fn get_knowledge<'a>(
        &'a self,
        user_id: &'a str,
        ids: &'a [String],
    ) -> StoreFuture<'a, Vec<Knowledge>>;

    /// Current facts with exactly this topic (case-insensitive).
    fn knowledge_by_topic<'a>(
        &'a self,
        user_id: &'a str,
        topic: &'a str,
    ) -> StoreFuture<'a, Vec<Knowledge>>;

    /// Mark a fact as replaced by a newer one (kept for history, no longer searched).
    fn supersede_knowledge<'a>(
        &'a self,
        user_id: &'a str,
        old_id: &'a str,
        new_id: &'a str,
        at: i64,
    ) -> StoreFuture<'a, ()>;

    /// Forget a fact: it's no longer recalled or listed (kept, marked as forgotten). False when
    /// there was no such current fact.
    fn forget_knowledge<'a>(
        &'a self,
        user_id: &'a str,
        id: &'a str,
        at: i64,
    ) -> StoreFuture<'a, bool>;

    /// Full-text search over current facts and episodes, best first.
    fn search_memory_text<'a>(
        &'a self,
        user_id: &'a str,
        query: &'a str,
        limit: u32,
    ) -> StoreFuture<'a, Vec<TextHit>>;

    /// Store (or replace) a vector for a fact or episode.
    fn put_embedding<'a>(
        &'a self,
        user_id: &'a str,
        kind: MemoryKind,
        id: &'a str,
        model: &'a str,
        vector: &'a [f32],
    ) -> StoreFuture<'a, ()>;

    /// Every stored vector made with `model`.
    fn embeddings<'a>(
        &'a self,
        user_id: &'a str,
        model: &'a str,
    ) -> StoreFuture<'a, Vec<StoredVector>>;

    /// Mark context messages as compacted (summarised; no longer sent to the model).
    fn compact_messages<'a>(
        &'a self,
        user_id: &'a str,
        session_id: &'a str,
        ids: &'a [String],
    ) -> StoreFuture<'a, ()>;
    /// Record a background helper starting.
    fn add_agent(&self, user_id: &str, agent: AgentRow) -> StoreFuture<'_, ()>;

    /// Record how a helper ended and its result.
    fn finish_agent<'a>(
        &'a self,
        user_id: &'a str,
        agent_id: &'a str,
        status: &'a str,
        state: &'a str,
        at: i64,
    ) -> StoreFuture<'a, ()>;

    /// One helper.
    fn get_agent<'a>(
        &'a self,
        user_id: &'a str,
        agent_id: &'a str,
    ) -> StoreFuture<'a, Option<AgentRow>>;

    /// Add a scheduled job.
    fn add_job(&self, user_id: &str, job: Job) -> StoreFuture<'_, ()>;

    /// The user's jobs, soonest first.
    fn list_jobs<'a>(&'a self, user_id: &'a str) -> StoreFuture<'a, Vec<Job>>;

    /// Enabled jobs due at or before `now`.
    fn due_jobs<'a>(&'a self, user_id: &'a str, now: i64) -> StoreFuture<'a, Vec<Job>>;

    /// Note a run and set the next one.
    fn job_ran<'a>(
        &'a self,
        user_id: &'a str,
        job_id: &'a str,
        ran_at: i64,
        next_run_at: i64,
    ) -> StoreFuture<'a, ()>;

    /// Delete a job (and its run history). False if it didn't exist.
    fn delete_job<'a>(&'a self, user_id: &'a str, job_id: &'a str) -> StoreFuture<'a, bool>;

    /// Record a job run.
    fn add_job_run(&self, user_id: &str, run: JobRun) -> StoreFuture<'_, ()>;

    /// Append to the audit log (which can't be edited or deleted).
    fn append_audit(&self, entry: AuditEntry) -> StoreFuture<'_, ()>;

    /// Current facts whose topic or content contains every word of `query`, newest first.
    fn search_knowledge<'a>(
        &'a self,
        user_id: &'a str,
        query: &'a str,
        limit: u32,
    ) -> StoreFuture<'a, Vec<Knowledge>>;
}
