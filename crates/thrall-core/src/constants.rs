//! Named constants for `thrall-core`. Every limit, timeout and repeated string lives here.

/// Folder name under the OS data folder (`%APPDATA%\Thrall` on Windows).
pub const APP_DIR_NAME: &str = "Thrall";

/// Folder under Documents that new sessions work in when no other folder is picked.
pub const WORK_DIR_NAME: &str = "Thrall";

/// Environment variables Windows sets to the OneDrive folders it syncs (personal, work or school).
/// A working folder inside one would sync every project, build folders included, and OneDrive
/// locks files while it uploads them.
pub const SYNCED_FOLDER_VARS: [&str; 3] = ["OneDrive", "OneDriveConsumer", "OneDriveCommercial"];

/// Environment variable that moves Thrall's data folder somewhere else.
pub const HOME_ENV_VAR: &str = "THRALL_HOME";

/// Config file name inside the data folder.
pub const CONFIG_FILE: &str = "config.toml";

/// SQLite database file name inside the data folder.
pub const DB_FILE: &str = "thrall.db";

/// Log folder name inside the data folder.
pub const LOGS_DIR: &str = "logs";

/// How long a question card waits for an answer before Thrall goes ahead with its recommended
/// answers, unless `[agent] question_wait_minutes` says otherwise.
pub const DEFAULT_QUESTION_WAIT_MINUTES: u32 = 5;

/// How many helpers work at the same time, unless `[agent] helpers_at_once` says otherwise.
pub const DEFAULT_HELPERS_AT_ONCE: u32 = 3;

/// Most helpers that may work at the same time, whatever the setting says.
pub const MAX_HELPERS_AT_ONCE: u32 = 8;

/// What one helper may spend before it's stopped, in US cents, unless `[agent]
/// helper_cost_limit_cents` says otherwise (0: no limit).
pub const DEFAULT_HELPER_COST_LIMIT_CENTS: u32 = 100;

/// Below this much OpenRouter credit, in US cents, Thrall warns once after a turn, unless
/// `[provider] low_balance_warn_cents` says otherwise (0: never).
pub const DEFAULT_LOW_BALANCE_WARN_CENTS: u32 = 100;

/// Most steps (model calls) in one turn, unless `[agent] max_steps` says otherwise. Each step can
/// run several tools.
pub const DEFAULT_MAX_STEPS: u32 = 100;

/// Most steps in one turn in Full auto, unless `[agent] full_auto_max_steps` says otherwise.
pub const DEFAULT_FULL_AUTO_MAX_STEPS: u32 = 250;

/// Fewest steps a turn may be given, whatever the settings say.
pub const MIN_MAX_STEPS: u32 = 10;

/// Most steps a turn may be given, whatever the settings say. Stops runaway loops.
pub const MAX_MAX_STEPS: u32 = 500;

/// The engine's access token (local mode): clients read it to connect. Created at first start.
pub const TOKEN_FILE: &str = "access-token";

/// Permissions for the data folder on Linux and macOS: this account only (no one else can list or
/// open anything inside it).
#[cfg(unix)]
pub const DATA_DIR_MODE: u32 = 0o700;

/// A sign-in link is the web app's address with this, then a one-time code (the page itself
/// carries no token: a browser signs in by opening such a link).
pub const SIGNIN_FRAGMENT: &str = "#signin=";

/// Files people attach to messages are kept here, inside the data folder (one folder per file id).
pub const UPLOADS_DIR: &str = "uploads";

/// Where Home worked before 0.2.2, inside the data folder; moved to [`HOME_FOLDER`] on first use.
pub const HOME_DIR: &str = "home";

/// Home's folder in Thrall's working folder (`Documents\Thrall\Home`), where what Home makes is
/// easy to find. Anyone but the owner gets `Home - <their name>` beside it.
pub const HOME_FOLDER: &str = "Home";

/// Written by a running engine (address, process id, version) and removed when it stops; clients
/// read it to find the engine.
pub const ENGINE_FILE: &str = "engine.json";

/// Log file name prefix (a date is appended each day).
pub const LOG_FILE_PREFIX: &str = "thrall.log";

/// A config value starting with this is the name of a key in the keychain ("keychain:youtube").
pub const KEYCHAIN_REF_PREFIX: &str = "keychain:";

/// Service name used for every entry Thrall keeps in the OS keychain.
pub const KEYCHAIN_SERVICE: &str = "Thrall";

/// Windows' program that lists Credential Manager entries (`/list`), for `stored_names`.
pub const CREDENTIAL_LIST_EXE: &str = "cmdkey";

/// How Credential Manager names one of Thrall's keys: `LegacyGeneric:target=<name>.Thrall`.
pub const CREDENTIAL_TARGET_PREFIX: &str = "LegacyGeneric:target=";

/// Most characters kept in one keychain entry for a long secret (Windows allows 1,280 UTF-16
/// characters per entry; a sign-in's tokens can be longer, so they're split across entries).
pub const LONG_SECRET_CHUNK_CHARS: usize = 1_000;

/// Most keychain entries one long secret may use.
pub const LONG_SECRET_MAX_PARTS: usize = 32;

/// Id of the single user in local mode.
pub const LOCAL_USER_ID: &str = "u_local";

/// Display name of the single user in local mode.
pub const LOCAL_USER_NAME: &str = "You";

/// Under the data folder: each person's own folder (`people/<id>`) for everyone but the owner,
/// whose things stay in the data folder itself.
pub const PEOPLE_DIR: &str = "people";

/// Longest name a person can be given (characters).
pub const PERSON_NAME_MAX_CHARS: usize = 40;

/// This engine's edition: free, for a household. (A hook: other editions may come later.)
pub const EDITION: &str = "home";

/// A person's role: what they may do on this engine (stored as text, so roles can be added).
pub mod person_role {
    /// Set the engine up and runs it: provides the keys, manages people.
    pub const OWNER: &str = "owner";
    /// Uses the engine.
    pub const MEMBER: &str = "member";
}

/// Who made a model call, as its usage row records it (stored as text).
pub mod usage_role {
    /// An ordinary call: the chat, a helper, a title, a summary.
    pub const CHAT: &str = "chat";
    /// Split mode: the model that plans, briefs and checks.
    pub const THINKER: &str = "thinker";
    /// Split mode: the model that does the work.
    pub const DOER: &str = "doer";
    /// Split mode: the Thinker answering the Doer's question.
    pub const ASK: &str = "ask";
}

/// Default AI model (matches what the Python Thrall uses today).
pub const DEFAULT_MODEL: &str = "xiaomi/mimo-v2.5-pro";

/// Default picture model: reads every attached picture and describes it for the chat model.
pub const DEFAULT_VISION_MODEL: &str = "google/gemini-2.5-flash";

/// Default model for reading text aloud (`speak`); it streams audio.
pub const DEFAULT_SPEECH_MODEL: &str = "openai/gpt-audio-mini";

/// Default voice for `speak`.
pub const DEFAULT_SPEECH_VOICE: &str = "alloy";

/// Default model for turning recordings into text.
pub const DEFAULT_TRANSCRIBE_MODEL: &str = "google/gemini-2.5-flash";

/// fal.ai's job queue (video generation).
pub const DEFAULT_FAL_BASE_URL: &str = "https://queue.fal.run";

/// Default model for making and editing pictures (`make_image`).
pub const DEFAULT_IMAGE_MODEL: &str = "google/gemini-3.1-flash-image";

/// Reads pictures when the picture model fails (another company's, so one busy provider isn't fatal).
pub const DEFAULT_VISION_BACKUP_MODEL: &str = "openai/gpt-4.1-mini";

/// Default OpenRouter API base URL.
pub const DEFAULT_OPENROUTER_BASE_URL: &str = "https://openrouter.ai/api/v1";

/// Where `local/<model>` models are served unless `[provider] local_base_url` says otherwise:
/// Ollama's OpenAI-compatible address (LM Studio's is http://localhost:1234/v1).
pub const DEFAULT_LOCAL_BASE_URL: &str = "http://localhost:11434/v1";

/// Where `program/sd-webui` pictures are made unless `[provider] sd_webui_url` says otherwise:
/// AUTOMATIC1111's and Forge's usual address.
pub const DEFAULT_SD_WEBUI_URL: &str = "http://127.0.0.1:7860";

/// Model ids starting with this run on the local model server (free, no key needed).
pub const LOCAL_MODEL_PREFIX: &str = "local/";

/// Default model for embeddings (searching memory by meaning).
pub const DEFAULT_EMBEDDING_MODEL: &str = "openai/text-embedding-3-small";

/// Default context size before older messages are summarised (tokens).
pub const DEFAULT_CONTEXT_LIMIT_TOKENS: usize = 100_000;

/// Default cap on tokens the model may write in one reply. Writing a whole file in one step
/// needs more than 8k; most current models allow 16k, and some suppliers refuse a cap above
/// what the model allows.
pub const DEFAULT_MAX_TOKENS: u32 = 16_384;

/// Default log level.
pub const DEFAULT_LOG_LEVEL: &str = "info";
/// Minutes between check-ins unless the person changes it.
pub const DEFAULT_HEARTBEAT_MINUTES: u32 = 30;
/// Check-ins can't come more often than this (each is a model call).
pub const HEARTBEAT_MIN_MINUTES: u32 = 5;
/// When check-ins happen unless the person changes it.
pub const DEFAULT_HEARTBEAT_HOURS: &str = "08:00-22:00";
/// The check-in checklist, in the data folder.
pub const HEARTBEAT_FILE: &str = "HEARTBEAT.md";
/// Days Home keeps earlier pages unless the person changes it.
pub const DEFAULT_HOME_KEEP_DAYS: u32 = 30;

/// Characters kept at the start of a key when showing a preview (`sk-…a1b2`).
pub const KEY_PREVIEW_HEAD: usize = 3;

/// Characters kept at the end of a key when showing a preview.
pub const KEY_PREVIEW_TAIL: usize = 4;

/// Id prefixes, so an id says what it points at (`s_…` is a session).
pub mod id_prefix {
    /// Session ids.
    pub const SESSION: &str = "s";
    /// Session group ids.
    pub const GROUP: &str = "g";
    /// Uploaded file ids.
    pub const FILE: &str = "f";
    /// Turn ids.
    pub const TURN: &str = "t";
    /// Message ids.
    pub const MESSAGE: &str = "m";
    /// Usage record ids.
    pub const USAGE: &str = "u";
    /// Tool step ids.
    pub const STEP: &str = "p";
    /// Task list entries.
    pub const TASK: &str = "k";
    /// Remembered facts.
    pub const KNOWLEDGE: &str = "f";
    /// Permission requests.
    pub const REQUEST: &str = "r";
    /// Audit log entries.
    pub const AUDIT: &str = "a";
    /// "Always" grants.
    pub const GRANT: &str = "g";
    /// Scheduled jobs.
    pub const JOB: &str = "j";
    /// Job runs.
    pub const JOB_RUN: &str = "jr";
    /// Helpers (sub-agents and background tasks).
    pub const AGENT: &str = "w";
    /// Remembered conversations.
    pub const EPISODE: &str = "e";
    /// Paired phones.
    pub const PHONE: &str = "ph";
    /// People (the first person, there before people existed, is `u_local`).
    pub const PERSON: &str = "pe";
    /// Signed-in browsers and PCs.
    pub const DEVICE: &str = "dv";
    /// Lessons in a person's experience log.
    pub const LESSON: &str = "ls";
}

/// Settings the settings screen may change (dotted `config.toml` keys). Lists and tables (MCP
/// servers, hooks) get their own editors.
pub const EDITABLE_SETTINGS: &[&str] = &[
    "provider.model",
    "provider.max_tokens",
    "provider.vision_model",
    "provider.vision_backup_model",
    "provider.image_model",
    "provider.speech_model",
    "provider.speech_voice",
    "provider.transcribe_model",
    "provider.base_url",
    "provider.fal_base_url",
    "provider.local_base_url",
    "provider.sd_webui_url",
    "provider.low_balance_warn_cents",
    "agent.verify",
    "agent.claim_check",
    "agent.questions",
    "agent.question_wait_minutes",
    "agent.helpers_at_once",
    "agent.helper_cost_limit_cents",
    "agent.max_steps",
    "agent.full_auto_max_steps",
    "memory.remember_conversations",
    "memory.context_limit_tokens",
    "memory.embedding_model",
    "memory.save_facts",
    "memory.trim_old_tool_results",
    "writing.humanise",
    "writing.detector_aware",
    "home.fresh_start",
    "home.keep_days",
    "heartbeat.enabled",
    "heartbeat.every_minutes",
    "heartbeat.active_hours",
    "heartbeat.model",
    "notifications.needs_you",
    "notifications.done",
    "notifications.failed",
    "notifications.background",
    "notifications.check_ins",
    "notifications.quiet_hours",
    "notifications.only_when_away",
    "desktop.shortcut",
    "remote.enabled",
    "remote.address",
    "remote.pcs",
    "folders.new_in",
    "logging.level",
];

/// Windows process creation flag: run a console program without giving it a console window.
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// The desktop app's program name (without `.exe`), installed beside the engine's.
pub const DESKTOP_PROGRAM: &str = "thrall-desktop";

/// Passed to the desktop app when Windows starts it at sign-in: start in the tray, window hidden.
pub const DESKTOP_HIDDEN_ARG: &str = "--hidden";

/// Where Windows keeps the programs it starts when this person signs in.
pub const STARTUP_RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

/// Task Manager's on/off switch for each of those programs ("Startup apps").
pub const STARTUP_APPROVED_KEY: &str =
    r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";

/// Thrall's entry in both: the name the desktop app's tray menu has always used.
pub const STARTUP_ENTRY: &str = "Thrall";

/// Task Manager's "Enabled" value. A disabled entry has the time it was disabled in its last 8
/// bytes instead of zeros.
pub const STARTUP_APPROVED_ON: [u8; 12] = [2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];

/// Linux: the folder, inside the person's settings folder (`$XDG_CONFIG_HOME`, else `~/.config`),
/// whose `.desktop` files the desktop starts at log-in (XDG autostart).
pub const AUTOSTART_DIR: &str = "autostart";

/// Linux: Thrall's file there.
pub const AUTOSTART_FILE: &str = "thrall.desktop";

/// macOS: the person's launch agents, which start at log-in (inside their home folder).
pub const LAUNCH_AGENTS_DIR: &str = "Library/LaunchAgents";

/// macOS: Thrall's launch agent, named after its label.
pub const LAUNCH_AGENT_LABEL: &str = "dev.thrall.desktop";

/// What the setting is called on this system (the tray menu and the Settings switch).
pub const STARTUP_LABEL: &str = if cfg!(windows) {
    "Start with Windows"
} else if cfg!(target_os = "macos") {
    "Open at login"
} else {
    "Start when I log in"
};

/// Port of the phone listener on 127.0.0.1 unless `[remote] port` says otherwise.
pub const DEFAULT_REMOTE_PORT: u16 = 7419;

/// On a companion PC, the Thrall it joined and its own key pair, in the data folder (the desktop
/// app starts `thrall runner` instead of the engine while it's there).
pub const COMPANION_FILE: &str = "companion.json";

/// What a companion PC says when the Thrall it joined doesn't let it in any more (removed there,
/// or that Thrall was set up again). The desktop app looks for it to offer joining again.
pub const NOT_JOINED: &str = "This PC isn't joined to that Thrall any more";

/// Port PCs reach this Thrall on to join as companions unless `[remote] pc_port` says otherwise.
pub const DEFAULT_PC_PORT: u16 = 7420;

/// UDP port a companion's "where is Thrall?" broadcast goes to; the engine answers from it.
pub const PC_DISCOVERY_PORT: u16 = 7421;

/// The desktop app's show/hide shortcut unless `[desktop] shortcut` says otherwise. Alt keeps it
/// clear of game controls (Ctrl+Shift+Space is a common fly/crouch-jump combination);
/// Ctrl+Alt+Space was already taken by another program on the first PC it was tried on.
pub const DEFAULT_DESKTOP_SHORTCUT: &str = "Ctrl+Alt+T";

/// Modifier key names a shortcut may start with (lower case, as the desktop app accepts them).
pub const SHORTCUT_MODIFIERS: &[&str] = &[
    "ctrl",
    "control",
    "alt",
    "option",
    "shift",
    "super",
    "cmd",
    "command",
    "commandorcontrol",
    "commandorctrl",
    "cmdorctrl",
    "cmdorcontrol",
];
