//! Named constants for `thrall-store`. Every limit, timeout and repeated string lives here.

/// Milliseconds SQLite waits for a lock before giving up.
pub const SQLITE_BUSY_TIMEOUT_MS: u64 = 5000;

/// Most pooled SQLite connections (one writer at a time anyway; readers share the rest).
pub const SQLITE_MAX_CONNECTIONS: u32 = 4;

/// Most events returned by one replay call.
pub const MAX_EVENTS_PER_PAGE: u32 = 1000;

/// Most sessions returned by one list call.
pub const MAX_SESSIONS_PER_PAGE: u32 = 200;

/// Micro-dollars in one dollar (money is stored as integers).
pub const MICROS_PER_DOLLAR: f64 = 1_000_000.0;

/// Default permission mode for new sessions.
pub const DEFAULT_SESSION_MODE: &str = "ask";

/// What a forgotten fact is marked as superseded by.
pub const FORGOTTEN: &str = "forgotten";
