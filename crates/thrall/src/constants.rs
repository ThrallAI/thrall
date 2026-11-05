//! Named constants for the `thrall` program.

/// Product name shown to users.
pub const APP_NAME: &str = "Thrall";

/// Exit code when something failed.
pub const EXIT_FAILURE: u8 = 1;

/// Exit code when setup is needed first (for example, no API key yet).
pub const EXIT_SETUP_NEEDED: u8 = 2;

/// Environment variable that overrides the log level from the config file.
pub const LOG_LEVEL_ENV_VAR: &str = "THRALL_LOG";

/// Sessions shown by `thrall sessions` unless `--limit` says otherwise.
pub const DEFAULT_SESSION_LIST_LIMIT: u32 = 20;

/// ANSI codes for de-emphasised text on a terminal.
pub const ANSI_DIM: &str = "\x1b[2m";
/// See [`ANSI_DIM`].
pub const ANSI_RESET: &str = "\x1b[0m";

/// Shown when `thrall run` gets nothing to do.
pub const EMPTY_PROMPT_HINT: &str =
    "Give Thrall something to do, for example: thrall run \"summarise this folder\"";

/// Shown when `thrall run -c` or `-s <id>` gets no message.
pub const EMPTY_PROMPT_IN_SESSION_HINT: &str =
    "Add your message after the session, for example: thrall run -c \"carry on\"";

/// How `thrall run` introduces itself to the engine.
pub const HEADLESS_CLIENT_NAME: &str = "Thrall Run";

/// How `thrall open` introduces itself to the engine.
pub const OPEN_CLIENT_NAME: &str = "Thrall Open";

/// Sets the engine's port when `--port` isn't given (a second, test copy of Thrall uses it).
pub const PORT_ENV_VAR: &str = "THRALL_PORT";

/// How long to wait for a running engine's health check.
pub const HEALTH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

/// How long `thrall` waits for an engine it started to come up.
pub const ENGINE_START_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

/// How often `thrall` checks whether the engine it started is up yet.
pub const ENGINE_START_POLL: std::time::Duration = std::time::Duration::from_millis(200);

/// An engine started on demand stops after this long with no clients and nothing running.
pub const IDLE_EXIT: std::time::Duration = std::time::Duration::from_secs(10 * 60);

/// How often an engine started on demand checks whether it's idle.
pub const IDLE_CHECK: std::time::Duration = std::time::Duration::from_secs(15);

/// Windows process flags for an engine started on demand: no console window, and not stopped
/// when the terminal that started it closes (DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP).
#[cfg(windows)]
pub const DETACHED_FLAGS: u32 = 0x0000_0008 | 0x0000_0200;

/// Shown when `thrall run -c` has no session to continue.
pub const NOTHING_TO_CONTINUE: &str =
    "There's no earlier session to continue, so this starts a new one.";

/// Keys Thrall's own tools use (besides the AI provider), with what for: shown by `thrall key status`.
pub const SERVICE_KEYS: &[(&str, &str)] = &[("fal", "making videos with make_video")];

/// The desktop app, looked for next to `thrall`; the terminal starts it (in the tray) to run the engine.
pub const DESKTOP_EXE: &str = if cfg!(windows) {
    "thrall-desktop.exe"
} else {
    "thrall-desktop"
};

/// Starts the desktop app in the tray, without opening its window.
pub const DESKTOP_HIDDEN_ARG: &str = "--hidden";

/// Starts the desktop app on Windows (`Start-Process`), so it doesn't inherit the terminal's handles.
pub const POWERSHELL_EXE: &str = "powershell.exe";

/// CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP: a console program with a hidden console.
#[cfg(windows)]
pub const HIDDEN_CONSOLE_FLAGS: u32 = 0x0800_0000 | 0x0000_0200;

/// What `thrall remove-everything` asks the person to type before it removes anything.
pub const REMOVE_CONFIRM: &str = "remove everything";

/// How long `thrall join` listens for Thrall answering on the network.
pub const COMPANION_FIND_WAIT: std::time::Duration = std::time::Duration::from_secs(3);

/// A companion's first wait before reconnecting after its link drops.
pub const COMPANION_RETRY_MIN: std::time::Duration = std::time::Duration::from_secs(2);

/// Its longest wait between tries (it doubles up to this while Thrall can't be reached).
pub const COMPANION_RETRY_MAX: std::time::Duration = std::time::Duration::from_secs(60);

/// How many times `thrall leave` checks that the companion's runner has stopped.
pub const COMPANION_STOP_TRIES: u32 = 20;

/// How long between those checks.
pub const COMPANION_STOP_POLL: std::time::Duration = std::time::Duration::from_millis(250);
