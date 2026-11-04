//! Named constants for the desktop app. Every limit, timeout and repeated string lives here.

use std::time::Duration;

/// The main window's label (tauri.conf.json).
pub const MAIN_WINDOW: &str = "main";

/// Added to the web app's address so the page knows it's inside the desktop app (it then
/// names answers `desktop` and shows Thrall's own menus instead of the browser's).
pub const DESKTOP_QUERY: &str = "?client=desktop";

/// The tray icon's id.
pub const TRAY_ID: &str = "thrall";

/// How often to look at `[desktop] shortcut` for a change made on the settings screen.
pub const SHORTCUT_RECHECK: Duration = Duration::from_secs(3);

/// Shown when the show/hide shortcut can't be used ({shortcut}).
pub const SHORTCUT_TAKEN_TITLE: &str = "Shortcut not available";
pub const SHORTCUT_TAKEN: &str = "{shortcut} is already used by another program, so Thrall left it \
off. Pick a different one in Settings → Startup → Show or hide shortcut.";

/// This app's log file name in Thrall's logs folder (a date is appended each day).
pub const LOG_FILE_PREFIX: &str = "desktop";

/// Overrides the log level (same variable as the engine's).
pub const LOG_LEVEL_ENV: &str = "THRALL_LOG";

/// Log level unless THRALL_LOG says otherwise.
pub const DEFAULT_LOG_LEVEL: &str = "info";

/// Passed when Windows starts the app at sign-in: start in the tray, window hidden. The engine's
/// Settings write the same startup entry, so it lives in thrall-core.
pub const HIDDEN_ARG: &str = thrall_core::constants::DESKTOP_HIDDEN_ARG;

/// The engine program, next to this one.
#[cfg(windows)]
pub const ENGINE_EXE: &str = "thrall.exe";
#[cfg(not(windows))]
pub const ENGINE_EXE: &str = "thrall";

/// Points at the engine program when it isn't next to this one.
pub const ENGINE_EXE_ENV: &str = "THRALL_EXE";

/// How long to wait for an engine this app started to come up.
pub const ENGINE_START_TIMEOUT: Duration = Duration::from_secs(20);

/// How often to look for the engine while waiting for it.
pub const ENGINE_POLL: Duration = Duration::from_millis(250);

/// Wait before reconnecting after the engine connection drops.
pub const RECONNECT_DELAY: Duration = Duration::from_secs(2);

/// Heartbeat (protocol §2).
pub const PING_INTERVAL: Duration = Duration::from_secs(20);

/// Tray animation frame time.
pub const TRAY_FRAME: Duration = Duration::from_millis(150);

/// How long the tray shows Done before resting.
pub const DONE_HOLD: Duration = Duration::from_millis(1600);

/// Tray icon size in pixels; the mark uses the 15×15 grid at this size (design system).
pub const TRAY_PX: u32 = 32;
pub const TRAY_GRID: usize = 15;

/// Sessions read at start, to name sessions in notifications.
pub const TITLE_LOOKUP: u32 = 100;

/// What the notification plugin calls a click on a notification itself.
pub const NOTE_TAP: &str = "tap";

/// Notifications whose session a click can still open (older ones are forgotten).
pub const NOTES_REMEMBERED: usize = 200;

/// The web app's function that opens a session (or Home) by id.
pub const OPEN_SESSION_JS: &str = "thrallOpen";

/// The loading page's function that shows the first-run setup (asking for the AI key).
pub const SHOW_SETUP_JS: &str = "thrallSetup";

/// First run: how often the loading page is asked to show the setup until it's there.
pub const SETUP_RECHECK: std::time::Duration = std::time::Duration::from_secs(1);

/// Windows process flags for the engine: no console window, and it keeps running if this app
/// quits and the person chose to leave it running (DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP).
#[cfg(windows)]
pub const DETACHED_FLAGS: u32 = 0x0000_0008 | 0x0000_0200;

/// Menu item ids.
pub const MENU_OPEN: &str = "open";
pub const MENU_AUTOSTART: &str = "autostart";
pub const MENU_UPDATE: &str = "update";
pub const MENU_QUIT: &str = "quit";

/// Updates: the first look after the app starts, then how often after that.
pub const UPDATE_FIRST_LOOK: Duration = Duration::from_secs(120);
pub const UPDATE_EVERY: Duration = Duration::from_secs(6 * 60 * 60);
/// How long an update waits for the engine to stop before installing anyway.
pub const UPDATE_ENGINE_STOP_WAIT: Duration = Duration::from_secs(20);
/// The tray's update item, before and after a newer version is found ({version}).
pub const UPDATE_CHECK_LABEL: &str = "Check for updates";
pub const UPDATE_READY_LABEL: &str = "Update to Thrall {version}";
/// Said once per new version ({version}).
pub const UPDATE_READY_TITLE: &str = "Thrall {version} is ready";
pub const UPDATE_READY_BODY: &str = "Click Update in Settings or the tray menu to install it. \
Thrall restarts by itself.";
pub const UPDATE_NONE: &str = "You have the newest version of Thrall.";
pub const UPDATE_FROM_SOURCE: &str = "This copy of Thrall was built from source, so it doesn't \
update itself.";
pub const UPDATE_FAILED_TITLE: &str = "Thrall couldn't update";
/// Asked when Thrall is in the middle of something.
pub const UPDATE_TITLE: &str = "Update Thrall";
pub const UPDATE_BUSY: &str = "Thrall is working on something right now. Update anyway? What \
it's doing stops, and Thrall restarts in about a minute.";
pub const UPDATE_NOW: &str = "Update now";
pub const UPDATE_LATER: &str = "Later";

/// Quit dialog buttons.
pub const QUIT_STOP: &str = "Quit and stop Thrall";
pub const QUIT_KEEP: &str = "Quit, keep Thrall running";
pub const QUIT_CANCEL: &str = "Cancel";
pub const QUIT_MESSAGE: &str = "Thrall's engine runs your sessions and scheduled jobs. Stop it as \
well? If you stop it, the terminal and web app disconnect until Thrall starts again.";
/// How long Quit waits for the engine to say which phones are connected before asking anyway.
pub const QUIT_PHONES_WAIT: Duration = Duration::from_millis(1500);
/// Request id of Quit's look at the connected phones.
pub const QUIT_PHONES_ID: &str = "d-quit-phones";

/// Colours for the tray mark (design system, dark values; they read on light taskbars too).
pub const ACCENT: [u8; 3] = [0x8b, 0x6c, 0xff];
pub const OK: [u8; 3] = [0x4a, 0xde, 0x80];
pub const DANGER: [u8; 3] = [0xff, 0x6b, 0x6b];
pub const INK: [u8; 3] = [0xc8, 0xc6, 0xd4];
pub const UNLIT: [u8; 4] = [0x8c, 0x8c, 0x9b, 0x55];
