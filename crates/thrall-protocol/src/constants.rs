//! Named constants for `thrall-protocol`. Every limit, timeout and repeated string lives here.

/// Protocol major version spoken by this build (`v` on every frame).
pub const PROTOCOL_VERSION: u32 = 1;

/// Oldest protocol major this build still accepts.
pub const OLDEST_SUPPORTED_VERSION: u32 = 1;

/// Largest frame either side may send, in bytes (1 MiB).
pub const MAX_FRAME_BYTES: usize = 1024 * 1024;

/// How often a client sends `ping`, in seconds.
pub const PING_INTERVAL_SECS: u64 = 20;

/// Silence after which a connection counts as dead, in seconds.
pub const DEAD_CONNECTION_SECS: u64 = 60;

// A healthy connection must be able to miss one ping without being dropped.
const _: () = assert!(PING_INTERVAL_SECS * 2 < DEAD_CONNECTION_SECS);

/// Time a client has to send `hello` after connecting, in seconds.
pub const HELLO_TIMEOUT_SECS: u64 = 10;

/// WebSocket endpoint path.
pub const WS_PATH: &str = "/v1/ws";

/// The phone link's WebSocket path (on its own listener; every message is Noise-encrypted).
pub const REMOTE_PATH: &str = "/v1/remote";

/// Largest piece of a file in one `file.put` or `file.part`, before base64 (512 KiB, so a
/// piece stays well under `MAX_FRAME_BYTES` once encoded).
pub const FILE_CHUNK_BYTES: usize = 512 * 1024;

const _: () = assert!(FILE_CHUNK_BYTES * 4 / 3 + 1024 < MAX_FRAME_BYTES);

/// `ModeLimit.until`: Full auto ends when the task finishes (the default).
pub const MODE_UNTIL_TASK_END: &str = "task_end";

/// `ModeLimit.until`: Full auto ends after a number of minutes.
pub const MODE_UNTIL_TIME: &str = "time";

/// `ModeLimit.until`: Full auto lasts until it's switched off (or Thrall restarts).
pub const MODE_UNTIL_MANUAL: &str = "manual";

/// Longest timed Full auto, in minutes (a day).
pub const MODE_LIMIT_MAX_MINUTES: u32 = 24 * 60;

/// The committed JSON Schema, relative to this crate's folder.
pub const SCHEMA_FILE: &str = "../../docs/protocol.schema.json";

/// Set this environment variable while running the tests to rewrite the committed schema.
pub const UPDATE_SCHEMA_ENV: &str = "THRALL_UPDATE_SCHEMA";

/// The `agent` of the `agent.completed` a check-in sends when it has something to say (clients
/// notify for it; the message itself is already in Home).
pub const CHECK_IN_AGENT: &str = "check-in";
