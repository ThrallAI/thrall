//! Rows going into and coming out of storage.

/// A browser or PC signed in as a person.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    /// Its id.
    pub id: String,
    /// Whose it is.
    pub user_id: String,
    /// The name it was signed in under ("Edge on Windows").
    pub name: String,
    /// When it was signed in, Unix ms.
    pub created_at: i64,
    /// When it last connected, Unix ms.
    pub last_seen_at: Option<i64>,
}

/// A person who uses this Thrall.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Person {
    /// User id.
    pub id: String,
    /// Display name.
    pub name: String,
    /// `owner` (set the engine up and runs it) or `member`.
    pub role: String,
    /// `free` (a hook for paid plans later).
    pub plan: String,
    /// When they were added, Unix ms.
    pub created_at: i64,
    /// When the owner removed them, Unix ms: they can't connect any more (their things are kept).
    pub removed_at: Option<i64>,
    /// Their turns use their own AI key, not the owner's.
    pub own_key: bool,
    /// Their own model (None: the engine's).
    pub model: Option<String>,
    /// The most their turns may cost in a calendar month, in millionths of a dollar (None: no
    /// limit).
    pub monthly_limit_micros: Option<i64>,
}

/// What a person's turns run on and may spend (set by the owner).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PersonTerms {
    /// Use their own AI key.
    pub own_key: bool,
    /// Their own model (None: the engine's).
    pub model: Option<String>,
    /// Monthly limit in millionths of a dollar (None: no limit).
    pub monthly_limit_micros: Option<i64>,
}

/// A session as stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    /// Session id.
    pub id: String,
    /// Owner.
    pub user_id: String,
    /// Title ('' until the first message names it).
    pub title: String,
    /// Working folder, if any.
    pub workspace: Option<String>,
    /// Permission mode.
    pub mode: String,
    /// `idle` · `working` · `waiting_for_you`.
    pub state: String,
    /// Last event sequence number used.
    pub last_seq: u64,
    /// Created, Unix ms.
    pub created_at: i64,
    /// Last activity, Unix ms.
    pub updated_at: i64,
    /// The person's group it's in, if any.
    pub group_id: Option<String>,
    /// When it was pinned to the top, if it is.
    pub pinned_at: Option<i64>,
    /// A page of Home (Thrall's built-in conversation): the local date it started.
    pub home_day: Option<String>,
}

/// A group the person made for sessions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionGroup {
    /// Group id.
    pub id: String,
    /// Name shown in the sidebar.
    pub name: String,
    /// Order in the sidebar (lowest first).
    pub position: i64,
}

/// What's needed to create a session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewSession {
    /// Session id (from `thrall_core::ids`).
    pub id: String,
    /// Owner.
    pub user_id: String,
    /// Working folder, if any.
    pub workspace: Option<String>,
    /// Now, Unix ms.
    pub at: i64,
}

/// One stored event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredEvent {
    /// Session id.
    pub session_id: String,
    /// Sequence number within the session.
    pub seq: u64,
    /// Turn, if the event belongs to one.
    pub turn_id: Option<String>,
    /// Protocol `type`.
    pub kind: String,
    /// The whole event as JSON.
    pub payload: String,
    /// When, Unix ms.
    pub at: i64,
}

/// An event to append.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewEvent {
    /// Owner.
    pub user_id: String,
    /// Session id.
    pub session_id: String,
    /// Turn, if any.
    pub turn_id: Option<String>,
    /// Protocol `type`.
    pub kind: String,
    /// The whole event as JSON.
    pub payload: String,
    /// When, Unix ms.
    pub at: i64,
}

/// A message in the model's context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredMessage {
    /// Message id.
    pub id: String,
    /// Session id.
    pub session_id: String,
    /// Turn, if any.
    pub turn_id: Option<String>,
    /// `system` · `user` · `assistant` (tool roles arrive in phase 2).
    pub role: String,
    /// Content as JSON (shape owned by the agent).
    pub content: String,
    /// Token estimate.
    pub tokens: u64,
    /// When, Unix ms.
    pub created_at: i64,
}

/// A message to add.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewMessage {
    /// Message id.
    pub id: String,
    /// Owner.
    pub user_id: String,
    /// Session id.
    pub session_id: String,
    /// Turn, if any.
    pub turn_id: Option<String>,
    /// Role.
    pub role: String,
    /// Content as JSON.
    pub content: String,
    /// Token estimate.
    pub tokens: u64,
    /// When, Unix ms.
    pub at: i64,
}

/// How a turn ended, with its totals.
#[derive(Debug, Clone, PartialEq)]
pub struct TurnEnd {
    /// Owner.
    pub user_id: String,
    /// Turn id.
    pub turn_id: String,
    /// `done` · `interrupted` · `failed`.
    pub status: String,
    /// When, Unix ms.
    pub at: i64,
    /// Tokens sent.
    pub input_tokens: u64,
    /// Tokens written.
    pub output_tokens: u64,
    /// Cost in dollars, if known.
    pub cost_usd: Option<f64>,
}

/// One model call's usage.
#[derive(Debug, Clone, PartialEq)]
pub struct UsageRecord {
    /// Record id.
    pub id: String,
    /// Owner.
    pub user_id: String,
    /// Session, if any.
    pub session_id: Option<String>,
    /// Turn, if any.
    pub turn_id: Option<String>,
    /// Provider id.
    pub provider: String,
    /// Model id.
    pub model: String,
    /// Who made the call (`thrall_core::constants::usage_role`).
    pub role: String,
    /// Tokens sent.
    pub input_tokens: u64,
    /// Of those, tokens the provider had cached.
    pub cached_tokens: u64,
    /// Tokens written.
    pub output_tokens: u64,
    /// Cost in dollars, if known.
    pub cost_usd: Option<f64>,
    /// When, Unix ms.
    pub at: i64,
}

/// A tool step to record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewStep {
    /// Step id.
    pub id: String,
    /// Owner.
    pub user_id: String,
    /// Turn it belongs to.
    pub turn_id: String,
    /// Tool name.
    pub tool: String,
    /// Plain-language title.
    pub title: String,
    /// Arguments as JSON.
    pub args: String,
    /// When, Unix ms.
    pub at: i64,
}

/// One entry of a session's task list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskRow {
    /// Task id.
    pub id: String,
    /// What it is.
    pub text: String,
    /// `pending` · `in_progress` · `done`.
    pub status: String,
}

/// A long-term fact.
#[derive(Debug, Clone, PartialEq)]
pub struct Knowledge {
    /// Fact id.
    pub id: String,
    /// Short topic.
    pub topic: String,
    /// The fact.
    pub content: String,
    /// Last change, Unix ms.
    pub updated_at: i64,
}

/// A remembered conversation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Episode {
    /// Episode id.
    pub id: String,
    /// The session it came from, if it still exists.
    pub session_id: Option<String>,
    /// One line about it.
    pub summary: String,
    /// What was said (shortened).
    pub content: String,
    /// When, Unix ms.
    pub created_at: i64,
}

/// What a memory search hit points at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MemoryKind {
    /// A fact (`knowledge`).
    Knowledge,
    /// A past conversation (`episodes`).
    Episode,
}

impl MemoryKind {
    /// The stored name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Knowledge => "knowledge",
            Self::Episode => "episode",
        }
    }

    /// From the stored name.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "knowledge" => Some(Self::Knowledge),
            "episode" => Some(Self::Episode),
            _ => None,
        }
    }
}

/// A full-text search hit: what it is, its id, and its rank (lower is better).
#[derive(Debug, Clone, PartialEq)]
pub struct TextHit {
    /// Fact or episode.
    pub kind: MemoryKind,
    /// Its id.
    pub id: String,
    /// BM25 rank (lower is a better match).
    pub rank: f64,
}

/// A stored vector.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredVector {
    /// Fact or episode.
    pub kind: MemoryKind,
    /// Its id.
    pub id: String,
    /// The vector.
    pub vector: Vec<f32>,
}

/// A background helper.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentRow {
    /// Helper id.
    pub id: String,
    /// Session it works for.
    pub session_id: String,
    /// What it's doing.
    pub title: String,
    /// `running` · `done` · `failed`.
    pub status: String,
    /// Its result (JSON).
    pub state: String,
    /// Started, Unix ms.
    pub started_at: i64,
}

/// A scheduled job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Job {
    /// Job id.
    pub id: String,
    /// The session its runs go into.
    pub session_id: Option<String>,
    /// Name.
    pub title: String,
    /// When it runs (cron expression or "every 30m").
    pub schedule: String,
    /// What Thrall is asked each time.
    pub prompt: String,
    /// Whether it runs.
    pub enabled: bool,
    /// Last run, Unix ms.
    pub last_run_at: Option<i64>,
    /// Next run, Unix ms.
    pub next_run_at: i64,
}

/// One finished run of a scheduled job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobRun {
    /// Run id.
    pub id: String,
    /// The job.
    pub job_id: String,
    /// Started, Unix ms.
    pub started_at: i64,
    /// Ended, Unix ms.
    pub ended_at: i64,
    /// How it went.
    pub status: String,
    /// One line about it.
    pub summary: String,
}

/// One gate decision for the audit log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditEntry {
    /// Entry id.
    pub id: String,
    /// Owner.
    pub user_id: String,
    /// When, Unix ms.
    pub at: i64,
    /// Which gate.
    pub gate: String,
    /// What was being done ("shell", "edit_file", "message"…).
    pub action: String,
    /// What the gate decided.
    pub decision: String,
    /// Details as JSON (already checked for secrets).
    pub detail: String,
}

/// Usage totals.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct UsageTotals {
    /// Tokens sent.
    pub input_tokens: u64,
    /// Tokens written.
    pub output_tokens: u64,
    /// Cost in dollars, counting only calls whose cost was reported.
    pub cost_usd: f64,
}

/// A file as it was before a turn first changed it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checkpoint {
    /// Session id.
    pub session_id: String,
    /// The turn that changed the file.
    pub turn_id: String,
    /// Owner.
    pub user_id: String,
    /// Absolute path.
    pub path: String,
    /// Whether the file existed before the turn changed it.
    pub existed: bool,
    /// Its content then (`None` when it didn't exist, or was too big to keep).
    pub content: Option<Vec<u8>>,
    /// It existed but was too big to keep, so it can't be put back.
    pub too_big: bool,
    /// SHA-256 (hex) of what the turn last left in the file, once it has changed it.
    pub after_hash: Option<String>,
    /// When it was taken, Unix ms.
    pub at: i64,
}
