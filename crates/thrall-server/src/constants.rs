//! Named constants for `thrall-server`. Every limit, timeout and repeated string lives here.

use std::time::Duration;

/// The engine listens on this address only: nothing outside the computer can reach it.
pub const LOCAL_HOST: &str = "127.0.0.1";

/// Port the engine listens on unless told otherwise.
pub const DEFAULT_PORT: u16 = 7417;

/// Origins of the desktop app's window (Tauri on Windows, and on other systems), which may
/// connect besides the engine's own web app.
pub const DESKTOP_ORIGINS: &[&str] = &["http://tauri.localhost", "tauri://localhost"];

/// What this engine can do, sent in `welcome`.
pub const CAPABILITIES: &[&str] = &[
    "streaming",
    "diffs",
    "plan_mode",
    "tasks",
    "commands",
    "status",
];

/// Health check path.
pub const HEALTH_PATH: &str = "/v1/health";

/// Stored events read per page when replaying a session.
pub const REPLAY_PAGE: u32 = 500;

/// Sessions listed when a client doesn't say how many.
pub const DEFAULT_SESSION_LIST: u32 = 50;

/// Random bytes in a new access token.
pub const TOKEN_BYTES: usize = 32;

/// How often the engine checks whether a connection has gone quiet.
pub const SILENCE_CHECK: Duration = Duration::from_secs(5);

/// Upload path (protocol §2).
pub const FILES_PATH: &str = "/v1/files";

/// Where a client fetches a file attached to a message (`GET`, same token as uploads).
pub const FILE_PATH: &str = "/v1/files/{id}";

/// Largest file someone can attach, in bytes (25 MiB).
pub const MAX_UPLOAD_BYTES: usize = 25 * 1024 * 1024;

/// Header carrying an upload's file name (URL-encoded).
pub const FILE_NAME_HEADER: &str = "x-file-name";

/// Longest file name kept for an upload.
pub const FILE_NAME_MAX_CHARS: usize = 120;

/// Name for an upload that arrives without a usable one.
pub const FALLBACK_FILE_NAME: &str = "file";

/// The web app's page.
pub const INDEX_FILE: &str = "index.html";

/// Where the web app's built files live (their names carry content hashes).
pub const ASSETS_PREFIX: &str = "assets/";

/// The arguments hint autocomplete shows for a skill's `/name`.
pub const SKILL_ARGS_HINT: &str = "[what to do]";

/// Noise pattern for pairing a phone: the phone knows the engine's key (from the QR) and proves
/// it holds the one-time secret in its first message, so a wrong secret fails right there.
pub const NOISE_PAIR: &str = "Noise_IKpsk1_25519_ChaChaPoly_BLAKE2s";

/// Noise pattern for a paired phone connecting: both sides know each other's key.
pub const NOISE_CONNECT: &str = "Noise_IK_25519_ChaChaPoly_BLAKE2s";

/// Bound into every handshake, so a handshake made for anything else can't be replayed here.
pub const NOISE_PROLOGUE: &[u8] = b"thrall-remote-v1";

/// First byte of a phone's first message: pairing with a code.
pub const LINK_PAIR: u8 = 1;

/// First byte of a phone's first message: a paired phone connecting.
pub const LINK_CONNECT: u8 = 2;

/// Bytes in a pairing code's id (sent in the clear, to pick the code).
pub const PAIR_ID_BYTES: usize = 16;

/// Bytes in a pairing secret (Noise pre-shared key size).
pub const PAIR_SECRET_BYTES: usize = 32;

/// How long a pairing code works.
pub const PAIR_LIFETIME: Duration = Duration::from_secs(5 * 60);

/// Wrong tries before a pairing code stops working.
pub const PAIR_MAX_TRIES: u32 = 5;

/// Pairing codes open at once (older ones are dropped).
pub const PAIR_MAX_OPEN: usize = 3;

/// Largest Noise message (the Noise limit).
pub const NOISE_MAX_MESSAGE: usize = 65_535;

/// Bytes Noise adds to each message (the authentication tag).
pub const NOISE_TAG: usize = 16;

/// Most plain bytes in one encrypted piece: the Noise limit less the tag and the piece marker.
pub const LINK_PIECE: usize = NOISE_MAX_MESSAGE - NOISE_TAG - 1;

/// Piece marker: more pieces of this frame follow.
pub const PIECE_MORE: u8 = 1;

/// Piece marker: this piece ends the frame.
pub const PIECE_LAST: u8 = 0;

/// Failed phone handshakes (of any kind) allowed per window before pairing pauses.
pub const LINK_MAX_FAILS: usize = 20;

/// The window failed handshakes are counted over, and how long pairing then stays paused.
pub const LINK_FAIL_WINDOW: Duration = Duration::from_secs(60);

/// Longest phone name kept.
pub const PHONE_NAME_MAX_CHARS: usize = 60;

/// Name for a phone that didn't give one.
pub const PHONE_FALLBACK_NAME: &str = "Phone";

/// The paired phones, in the data folder.
pub const PHONES_FILE: &str = "phones.json";

/// The engine's long-term phone-link key pair, in the data folder (private then public, hex).
pub const REMOTE_KEY_FILE: &str = "remote-key";

/// How often the engine checks whether the phone listener should be running.
pub const REMOTE_CHECK: Duration = Duration::from_secs(2);

/// Most entries one `folder.list` returns (the rest are left out, and the reply says so).
pub const FOLDER_LIST_MAX: usize = 500;

/// How many folders down `folder.read` looks for a file named without its folder.
pub const FIND_DEPTH: usize = 4;

/// Most entries that search looks at before giving up.
pub const FIND_SCAN: usize = 5_000;

/// Folders that search skips (build output and dependencies, never what Thrall wrote).
pub const FIND_SKIP: &[&str] = &[
    "node_modules",
    "target",
    "__pycache__",
    "venv",
    "dist",
    "build",
];

/// Most recent folders `folder.places` offers.
pub const PLACES_MAX: usize = 8;

/// How many recent sessions `folder.places` looks through for their folders.
pub const PLACES_SCAN: u32 = 200;

/// Longest name `folder.create` accepts.
pub const FOLDER_NAME_MAX_CHARS: usize = 100;

/// Characters Windows doesn't allow in a folder name (besides control characters).
pub const FOLDER_NAME_BAD_CHARS: &[char] = &['\\', '/', ':', '*', '?', '"', '<', '>', '|'];

/// Names Windows keeps for devices: not usable as a folder, with or without an extension.
pub const RESERVED_NAMES: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// The phone app offered as an update, in the data folder: the APK and a description of it.
pub const PHONE_APP_DIR: &str = "phone-app";

/// That description (`version_code`, `version_name`, `file`, `sha256`, `notes`), written by
/// `scripts/release-android.ps1`.
pub const PHONE_APP_RELEASE_FILE: &str = "release.json";

/// What `file.part` calls the phone app.
pub const PHONE_APP_FILE_ID: &str = "app";

/// The scheme of the pairing link a QR holds.
pub const PAIR_LINK_PREFIX: &str = "thrall://pair?";

/// Where a browser trades a one-time sign-in code for its own device token (`POST`).
pub const SIGNIN_PATH: &str = "/v1/signin";

/// Random bytes in a sign-in code (hex in the link).
pub const SIGNIN_CODE_BYTES: usize = 16;

/// How long a sign-in link works.
pub const SIGNIN_CODE_LIFETIME: Duration = Duration::from_secs(30 * 60);

/// Sign-in links waiting to be used at once (the oldest goes first).
pub const SIGNIN_MAX_OPEN: usize = 20;

/// Longest name kept for a signed-in browser.
pub const DEVICE_NAME_MAX_CHARS: usize = 60;

/// Name for a browser that signs in without giving one.
pub const FALLBACK_DEVICE_NAME: &str = "A browser";

/// Longest model id a person's terms accept.
pub const MODEL_ID_MAX_CHARS: usize = 120;

/// Highest monthly spending limit a person can be given, in US dollars.
pub const MONTHLY_LIMIT_MAX_USD: f64 = 100_000.0;

/// Millionths of a dollar in a dollar (limits are stored as whole millionths).
pub const MICROS_PER_DOLLAR: f64 = 1_000_000.0;
