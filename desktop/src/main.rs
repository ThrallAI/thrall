//! Thrall's desktop app: the web app in a native window, plus what a browser tab can't do. It
//! starts the engine if it isn't running and keeps it running; closing the window hides it to
//! the tray, where the living mark shows what Thrall is doing. Quit asks whether to stop the
//! engine too.

// A windowed program in every build: no console window opens beside the app (which would also
// quit the app if closed). Problems are written to Thrall's log instead.
#![windows_subsystem = "windows"]

mod constants;
mod engine;
mod mark;
mod update;
mod watch;

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::time::Instant;

use chrono::Timelike;
use thrall_core::Config;
use thrall_protocol::{Event, Request};

use tauri::image::Image;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, WindowEvent};
use tauri_plugin_dialog::{
    DialogExt, MessageDialogButtons, MessageDialogKind, MessageDialogResult,
};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_opener::OpenerExt;
use tokio::sync::mpsc;

use crate::constants::{
    DEFAULT_LOG_LEVEL, DESKTOP_QUERY, DONE_HOLD, HIDDEN_ARG, JOIN_CODE_JS, LOG_FILE_PREFIX,
    LOG_LEVEL_ENV, MAIN_WINDOW, MENU_AUTOSTART, MENU_OPEN, MENU_QUIT, MENU_UPDATE, NOTE_TAP,
    NOTES_REMEMBERED, OPEN_FAILED_TITLE, OPEN_SESSION_JS, QUIT_CANCEL, QUIT_KEEP, QUIT_PHONES_WAIT,
    QUIT_STOP, RECONNECT_DELAY, REJOIN_JS, SETUP_RECHECK, SHORTCUT_RECHECK, SHORTCUT_TAKEN,
    SHORTCUT_TAKEN_TITLE, SHOW_SETUP_JS, TITLE_LOOKUP, TRAY_FRAME, TRAY_ID, TRAY_PX,
    UPDATE_CHECK_LABEL,
};
use crate::engine::Incoming;
use crate::mark::Activity;
use crate::watch::{Watch, allowed};

/// What the app's tasks share.
#[derive(Default)]
struct Shared {
    /// Frames to the engine (for Quit's stop request), while connected.
    engine: Mutex<Option<mpsc::UnboundedSender<String>>>,
    watch: Mutex<Watch>,
    /// The session each shown notification is about, by notification id (a click opens it).
    notes: Mutex<HashMap<i32, String>>,
    next_note: AtomicI32,
    /// The show/hide shortcut in use (`None` until the first look at the settings).
    shortcut: Mutex<Option<String>>,
    /// The tray's Open item, which shows the shortcut beside it.
    open_item: Mutex<Option<MenuItem<tauri::Wry>>>,
    /// The tray's start-at-sign-in tick, re-read before the menu opens (Settings can change it).
    autostart_item: Mutex<Option<CheckMenuItem<tauri::Wry>>>,
    /// First run: the setup page saved the AI key, so the engine can start.
    key_saved: tokio::sync::Notify,
    /// Quit waiting to hear how many phones are connected.
    quit_phones: Mutex<Option<tokio::sync::oneshot::Sender<usize>>>,
    /// The tray's update item (it names a newer version once one is found).
    update_item: Mutex<Option<MenuItem<tauri::Wry>>>,
    /// The newer version on offer, once found.
    update_offered: Mutex<Option<String>>,
    /// An update is downloading or installing: the engine isn't started again meanwhile.
    updating: AtomicBool,
    /// The app's own loading page, to go back to when a companion PC has to join again.
    loading_page: Mutex<Option<tauri::Url>>,
}

/// Whether the AI key Thrall needs is there (in the environment or the system keychain).
fn has_key() -> bool {
    let provider = thrall_core::ProviderKind::OpenRouter;
    std::env::var(provider.key_env_var()).is_ok_and(|k| !k.trim().is_empty())
        || thrall_core::secrets::get_key(provider).is_ok_and(|k| k.is_some())
}

/// First run, from the loading page's setup form: keep the AI key in the system keychain (never
/// in a file), then let the engine start. Only the app's own loading page can call this; the
/// web app the engine serves has no access to the app's commands.
#[tauri::command]
fn save_key(app: AppHandle, key: String) -> Result<(), String> {
    let key = key.trim();
    if key.is_empty() {
        return Err("Paste your key first.".into());
    }
    if key.chars().any(char::is_whitespace) {
        return Err("That doesn't look like a key (it has spaces in it).".into());
    }
    thrall_core::secrets::set_named(thrall_core::ProviderKind::OpenRouter.as_str(), key)
        .map_err(|e| format!("Couldn't keep the key in this computer's keychain: {e}"))?;
    tracing::info!("AI key saved from the first-run setup");
    app.state::<Shared>().key_saved.notify_one();
    Ok(())
}

/// `thrall join …` with plain-line output, run without a console window.
fn join_command(args: &[&str]) -> tokio::process::Command {
    let mut command = tokio::process::Command::new(engine::engine_exe());
    command
        .arg("join")
        .args(args)
        .arg("--machine")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(thrall_core::constants::CREATE_NO_WINDOW);
    command
}

/// First run, from the loading page: the Thralls on this network that let PCs join, as
/// `[{address, name}]`.
#[tauri::command]
async fn companion_find() -> Result<Vec<serde_json::Value>, String> {
    let out = join_command(&["--find"])
        .output()
        .await
        .map_err(|e| format!("Couldn't look for Thrall: {e}"))?;
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| l.strip_prefix("found "))
        .filter_map(|l| l.split_once('\t'))
        .map(|(address, name)| serde_json::json!({ "address": address, "name": name }))
        .collect())
}

/// First run, from the loading page: ask the Thrall at `address` to let this PC join, showing the
/// code on the page while its owner decides. Once in, this PC starts as its companion.
#[tauri::command]
async fn companion_join(app: AppHandle, address: String) -> Result<(), String> {
    use tokio::io::AsyncBufReadExt;
    let mut child = join_command(&[address.trim()])
        .spawn()
        .map_err(|e| format!("Couldn't ask to join: {e}"))?;
    let stdout = child.stdout.take().ok_or("Couldn't ask to join.")?;
    let mut lines = tokio::io::BufReader::new(stdout).lines();
    let mut outcome = Err("Thrall stopped asking before an answer came.".to_owned());
    while let Ok(Some(line)) = lines.next_line().await {
        if let Some(code) = line.strip_prefix("code ") {
            if let (Some(w), Ok(js)) = (
                app.get_webview_window(MAIN_WINDOW),
                serde_json::to_string(code),
            ) {
                let _ = w.eval(format!(
                    "window.{JOIN_CODE_JS} && window.{JOIN_CODE_JS}({js});"
                ));
            }
        } else if line == "joined" {
            outcome = Ok(());
        } else if line == "declined" {
            outcome = Err("The owner said no.".to_owned());
        } else if let Some(why) = line.strip_prefix("error ") {
            outcome = Err(why.to_owned());
        }
    }
    let _ = child.wait().await;
    if outcome.is_ok() {
        tracing::info!("this PC joined Thrall on another PC");
        app.state::<Shared>().key_saved.notify_one();
    }
    outcome
}

/// From the loading page, before joining again: stop being a companion of the Thrall that no
/// longer lets this PC in (`thrall leave` stops the runner and forgets it).
#[tauri::command]
async fn companion_leave() -> Result<(), String> {
    let mut command = tokio::process::Command::new(engine::engine_exe());
    command
        .arg("leave")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(thrall_core::constants::CREATE_NO_WINDOW);
    let out = command
        .output()
        .await
        .map_err(|e| format!("Couldn't leave: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_owned())
    }
}

/// A companion PC that isn't let in any more: back to the loading page, offering to join again,
/// until it has.
async fn offer_rejoin(app: &AppHandle, why: &str) {
    if let Some(w) = app.get_webview_window(MAIN_WINDOW) {
        let home = app
            .state::<Shared>()
            .loading_page
            .lock()
            .ok()
            .and_then(|p| p.clone());
        if let Some(home) = home
            && w.url().is_ok_and(|now| now.origin() != home.origin())
        {
            let _ = w.navigate(home);
        }
    }
    show(app);
    let Ok(js) = serde_json::to_string(why) else {
        return;
    };
    loop {
        if let Some(w) = app.get_webview_window(MAIN_WINDOW) {
            let _ = w.eval(format!("window.{REJOIN_JS} && window.{REJOIN_JS}({js});"));
        }
        let shared = app.state::<Shared>();
        tokio::select! {
            () = shared.key_saved.notified() => return,
            () = tokio::time::sleep(SETUP_RECHECK) => {}
        }
    }
}

/// The startup entry for this app, from beside it (`None` where Windows can't start it).
fn startup_line() -> Option<String> {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().and_then(thrall_core::startup::command_line))
}

/// The settings, read fresh each time (the settings screen changes the file).
fn settings(paths: &thrall_core::Paths) -> Config {
    std::fs::read_to_string(&paths.config_file)
        .ok()
        .and_then(|text| Config::parse(&text, &paths.config_file).ok())
        .unwrap_or_default()
}

/// Use `[desktop] shortcut` for showing and hiding the window, when it differs from the one in
/// use. A shortcut another program already holds is logged and left off.
fn apply_shortcut(app: &AppHandle, wanted: &str) {
    let shared = app.state::<Shared>();
    let Ok(mut current) = shared.shortcut.lock() else {
        return;
    };
    if current.as_deref() == Some(wanted) {
        return;
    }
    let shortcuts = app.global_shortcut();
    if let Err(e) = shortcuts.unregister_all() {
        tracing::warn!(%e, "couldn't release the old shortcut");
    }
    let registered = !wanted.is_empty()
        && match shortcuts.register(wanted) {
            Ok(()) => true,
            Err(e) => {
                tracing::warn!(%e, shortcut = wanted, "couldn't use the shortcut");
                let _ = app
                    .notification()
                    .builder()
                    .title(SHORTCUT_TAKEN_TITLE)
                    .body(SHORTCUT_TAKEN.replace("{shortcut}", wanted))
                    .show();
                false
            }
        };
    if let Some(item) = shared.open_item.lock().ok().and_then(|i| i.clone()) {
        let shown = registered.then_some(wanted);
        if let Err(e) = item.set_accelerator(shown) {
            tracing::debug!(%e, "the tray menu can't show this shortcut");
        }
    }
    tracing::info!(shortcut = wanted, registered, "show/hide shortcut");
    *current = Some(wanted.to_owned());
}

/// Keep the shortcut in step with the settings for as long as the app runs.
async fn follow_shortcut(app: AppHandle) {
    let paths = match engine::paths() {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(%e, "no data folder: the shortcut stays off");
            return;
        }
    };
    loop {
        let wanted = settings(&paths).desktop.shortcut.trim().to_owned();
        apply_shortcut(&app, &wanted);
        tokio::time::sleep(SHORTCUT_RECHECK).await;
    }
}

/// A notification was clicked: bring the window forward on the session it's about.
fn open_from_note(app: &AppHandle, id: i32) {
    let session = app
        .state::<Shared>()
        .notes
        .lock()
        .ok()
        .and_then(|n| n.get(&id).cloned());
    show(app);
    if let (Some(session), Some(w)) = (session, app.get_webview_window(MAIN_WINDOW))
        && let Ok(quoted) = serde_json::to_string(&session)
    {
        let _ = w.eval(format!(
            "window.{OPEN_SESSION_JS} && window.{OPEN_SESSION_JS}({quoted});"
        ));
    }
}

/// Write this app's log beside the engine's (logs/desktop.<date>), since it has no console.
fn start_logging() -> Option<tracing_appender::non_blocking::WorkerGuard> {
    let paths = engine::paths().ok()?;
    std::fs::create_dir_all(&paths.logs_dir).ok()?;
    let file = tracing_appender::rolling::daily(&paths.logs_dir, LOG_FILE_PREFIX);
    let (writer, guard) = tracing_appender::non_blocking(file);
    tracing_subscriber::fmt()
        .with_writer(writer)
        .with_ansi(false)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env(LOG_LEVEL_ENV)
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(DEFAULT_LOG_LEVEL)),
        )
        .try_init()
        .ok()?;
    Some(guard)
}

fn main() {
    let _log = start_logging();
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "desktop app started");
    let result = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show(app)
        }))
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_opener::init())
        .plugin(
            // The shortcut itself comes from the settings (follow_shortcut).
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _shortcut, event| {
                    if event.state() == ShortcutState::Pressed {
                        toggle(app);
                    }
                })
                .build(),
        )
        .manage(Shared::default())
        .invoke_handler(tauri::generate_handler![
            save_key,
            companion_find,
            companion_join,
            companion_leave
        ])
        .setup(|app| {
            build_tray(app.handle())?;
            if let Some(w) = app.get_webview_window(MAIN_WINDOW)
                && let Ok(url) = w.url()
                && let Ok(mut page) = app.state::<Shared>().loading_page.lock()
            {
                *page = Some(url);
            }
            let clicked = app.handle().clone();
            let _ = app.notification().on_action(move |performed| {
                if performed.action_id() == NOTE_TAP
                    && let Some(n) = performed.notification()
                {
                    open_from_note(&clicked, n.id());
                }
            });
            if !std::env::args().any(|a| a == HIDDEN_ARG) {
                show(app.handle());
            }
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(run_engine(handle.clone()));
            tauri::async_runtime::spawn(follow_shortcut(handle.clone()));
            tauri::async_runtime::spawn(update::watch(handle.clone()));
            tauri::async_runtime::spawn(animate_tray(handle));
            Ok(())
        })
        .on_window_event(|window, event| {
            // Closing the window keeps Thrall running in the tray.
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .run(tauri::generate_context!());
    if let Err(e) = result {
        tracing::error!(%e, "the desktop app couldn't start");
    }
}

fn show(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(MAIN_WINDOW) {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

fn toggle(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(MAIN_WINDOW) {
        let in_front = w.is_visible().unwrap_or(false) && w.is_focused().unwrap_or(false);
        if in_front {
            let _ = w.hide();
        } else {
            show(app);
        }
    }
}

fn in_front(app: &AppHandle) -> bool {
    app.get_webview_window(MAIN_WINDOW)
        .is_some_and(|w| w.is_visible().unwrap_or(false) && w.is_focused().unwrap_or(false))
}

fn icon(activity: Activity, t: f32) -> Image<'static> {
    Image::new_owned(mark::frame(activity, t, TRAY_PX), TRAY_PX, TRAY_PX)
}

fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, MENU_OPEN, "Open Thrall", true, None::<&str>)?;
    if let Ok(mut item) = app.state::<Shared>().open_item.lock() {
        *item = Some(open.clone());
    }
    let autostart = CheckMenuItem::with_id(
        app,
        MENU_AUTOSTART,
        thrall_core::constants::STARTUP_LABEL,
        startup_line().is_some(),
        thrall_core::startup::is_on().unwrap_or(false),
        None::<&str>,
    )?;
    if let Ok(mut item) = app.state::<Shared>().autostart_item.lock() {
        *item = Some(autostart.clone());
    }
    let update = MenuItem::with_id(app, MENU_UPDATE, UPDATE_CHECK_LABEL, true, None::<&str>)?;
    if let Ok(mut item) = app.state::<Shared>().update_item.lock() {
        *item = Some(update.clone());
    }
    let menu = Menu::with_items(
        app,
        &[
            &open,
            &PredefinedMenuItem::separator(app)?,
            &autostart,
            &update,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, MENU_QUIT, "Quit Thrall…", true, None::<&str>)?,
        ],
    )?;
    TrayIconBuilder::with_id(TRAY_ID)
        .icon(icon(Activity::Idle, 0.0))
        .tooltip("Thrall · Starting")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            MENU_OPEN => show(app),
            MENU_AUTOSTART => {
                let on = thrall_core::startup::is_on().unwrap_or(false);
                let line = startup_line();
                let line = if on { None } else { line.as_deref() };
                if let Err(e) = thrall_core::startup::set(line) {
                    tracing::warn!(%e, "couldn't change starting with Windows");
                }
                refresh_autostart(app);
            }
            MENU_UPDATE => {
                tauri::async_runtime::spawn(update::tray_clicked(app.clone()));
            }
            MENU_QUIT => start_quit(app),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            // The menu opens on a right click: show the tick as it is now.
            if let TrayIconEvent::Click {
                button: MouseButton::Right,
                button_state: MouseButtonState::Down,
                ..
            } = event
            {
                refresh_autostart(tray.app_handle());
            }
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                toggle(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(())
}

/// Set the tray's Start with Windows tick from the startup entry itself.
fn refresh_autostart(app: &AppHandle) {
    if let Ok(item) = app.state::<Shared>().autostart_item.lock()
        && let Some(item) = item.as_ref()
    {
        let _ = item.set_checked(thrall_core::startup::is_on().unwrap_or(false));
    }
}

/// Quit first asks the engine which phones are connected (stopping it would cut them off), then
/// asks the person. Without an answer in time it asks without naming them.
fn start_quit(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let shared = app.state::<Shared>();
        let (tx, rx) = tokio::sync::oneshot::channel();
        // Ready for the answer before the question goes out.
        if let Ok(mut wait) = shared.quit_phones.lock() {
            *wait = Some(tx);
        }
        let asked = shared
            .engine
            .lock()
            .ok()
            .and_then(|e| e.clone())
            .is_some_and(|out| out.send(engine::phones_frame()).is_ok());
        let phones = if asked {
            tokio::time::timeout(QUIT_PHONES_WAIT, rx)
                .await
                .ok()
                .and_then(Result::ok)
                .unwrap_or(0)
        } else {
            0
        };
        ask_to_quit(&app, phones);
    });
}

/// Quit asks whether to stop the engine too (it runs sessions, scheduled jobs and the other
/// clients' connections, phones included).
fn ask_to_quit(app: &AppHandle, phones: usize) {
    let handle = app.clone();
    app.dialog()
        .message(engine::quit_message(phones))
        .title("Quit Thrall")
        .kind(MessageDialogKind::Info)
        .buttons(MessageDialogButtons::YesNoCancelCustom(
            QUIT_STOP.to_owned(),
            QUIT_KEEP.to_owned(),
            QUIT_CANCEL.to_owned(),
        ))
        .show_with_result(move |result| {
            let label = match result {
                MessageDialogResult::Custom(s) => s,
                MessageDialogResult::Yes => QUIT_STOP.to_owned(),
                MessageDialogResult::No => QUIT_KEEP.to_owned(),
                _ => return,
            };
            if label == QUIT_STOP {
                let engine = handle
                    .state::<Shared>()
                    .engine
                    .lock()
                    .ok()
                    .and_then(|e| e.clone());
                if let Some(out) = engine {
                    let _ = out.send(engine::stop_frame());
                }
                // Give the request a moment to go out before the process ends.
                std::thread::sleep(std::time::Duration::from_millis(300));
                handle.exit(0);
            } else if label == QUIT_KEEP {
                handle.exit(0);
            }
        });
}

/// Open a file a step wrote with its usual program, or show it selected in its folder. A file that
/// can't be opened (no program for it, gone since) is said in a notification.
fn open_file(app: &AppHandle, path: &str, reveal: bool) {
    let opener = app.opener();
    let done = if reveal {
        opener.reveal_item_in_dir(path)
    } else {
        opener.open_path(path, None::<&str>)
    };
    if let Err(e) = done {
        tracing::warn!(%e, reveal, "couldn't open a file");
        let _ = app
            .notification()
            .builder()
            .title(OPEN_FAILED_TITLE)
            .body(format!("{path}: {e}"))
            .show();
    }
}

/// Show a message on the window's loading page (before the web app takes over).
fn loading_message(app: &AppHandle, text: &str) {
    if let Some(w) = app.get_webview_window(MAIN_WINDOW)
        && let Ok(js) = serde_json::to_string(text)
    {
        let _ = w.eval(format!(
            "var m=document.getElementById('msg'); if (m) m.textContent = {js};"
        ));
    }
}

/// Keep a connection to the engine (starting it when needed), point the window at its web app,
/// and turn events into tray state and notifications.
async fn run_engine(app: AppHandle) {
    let paths = match engine::paths() {
        Ok(p) => p,
        Err(e) => {
            loading_message(&app, &format!("Thrall couldn't find its data folder: {e}"));
            return;
        }
    };
    // First run: the engine can't start without an AI key, so ask for one here (unless an
    // engine is already running, set up some other way).
    let mut shown = false;
    // A local model (`local/…`) needs no key.
    let local = settings(&paths)
        .provider
        .model
        .starts_with(thrall_core::constants::LOCAL_MODEL_PREFIX);
    // A companion PC uses the AI of the Thrall it joined: it needs no key of its own.
    while !local
        && !has_key()
        && !engine::companion(&paths)
        && engine::connect(&paths).await.is_err()
    {
        if !shown {
            show(&app);
            shown = true;
        }
        // Asked again until the page is there to show it (it may still be loading).
        if let Some(w) = app.get_webview_window(MAIN_WINDOW) {
            let _ = w.eval(format!(
                "window.{SHOW_SETUP_JS} && window.{SHOW_SETUP_JS}();"
            ));
        }
        let shared = app.state::<Shared>();
        tokio::select! {
            () = shared.key_saved.notified() => {}
            () = tokio::time::sleep(SETUP_RECHECK) => {}
        }
    }
    loop {
        // The installer is about to replace the engine: don't start it again.
        if app.state::<Shared>().updating.load(Ordering::SeqCst) {
            tokio::time::sleep(RECONNECT_DELAY).await;
            continue;
        }
        match engine::connect_or_start(&paths).await {
            Ok(mut conn) => {
                if let Ok(mut e) = app.state::<Shared>().engine.lock() {
                    *e = Some(conn.sender());
                }
                // Signed in with a one-time link: the page itself carries no token.
                let page = format!("{}{DESKTOP_QUERY}", conn.web_url);
                let page = match &conn.signin {
                    Some(code) => thrall_core::discovery::signin_url(&page, code),
                    None => page,
                };
                if let (Some(w), Ok(url)) = (
                    app.get_webview_window(MAIN_WINDOW),
                    page.parse::<tauri::Url>(),
                ) {
                    let already = w.url().is_ok_and(|now| now.origin() == url.origin());
                    if !already {
                        let _ = w.navigate(url);
                    }
                }
                while let Some(msg) = conn.incoming.recv().await {
                    if let Incoming::PhonesConnected(n) = msg {
                        let wait = app
                            .state::<Shared>()
                            .quit_phones
                            .lock()
                            .ok()
                            .and_then(|mut w| w.take());
                        if let Some(tx) = wait {
                            let _ = tx.send(n);
                        }
                        continue;
                    }
                    // The owner pressed Update in Settings.
                    if let Incoming::Event(frame) = &msg
                        && matches!(frame.event, Event::UpdateRequested {})
                    {
                        tauri::async_runtime::spawn(update::install(app.clone()));
                        continue;
                    }
                    // Open on a file card: the engine found the file from the step that wrote it.
                    if let Incoming::Event(frame) = &msg
                        && let Event::FileOpen { path, reveal } = &frame.event
                    {
                        open_file(&app, path, *reveal);
                        continue;
                    }
                    let (note, unknown) = match app.state::<Shared>().watch.lock() {
                        Ok(mut watch) => match &msg {
                            Incoming::Sessions(items) => {
                                watch.sessions(items);
                                (None, false)
                            }
                            Incoming::Event(frame) => {
                                let unknown = !watch.knows(&frame.session);
                                (watch.apply(frame), unknown)
                            }
                            Incoming::PhonesConnected(_) => (None, false),
                        },
                        Err(_) => (None, false),
                    };
                    // A session started since the lists came (a new Home page): learn its title.
                    if unknown {
                        conn.send(Request::SessionList {
                            limit: Some(TITLE_LOOKUP),
                            archived: false,
                        });
                        conn.send(Request::HomeGet {});
                    }
                    let Some(note) = note else { continue };
                    let minute = {
                        let t = chrono::Local::now();
                        t.hour() * 60 + t.minute()
                    };
                    let wanted = settings(&paths).notifications;
                    if !allowed(note.kind, &wanted, minute, in_front(&app)) {
                        tracing::debug!(kind = ?note.kind, "notification held back by the settings");
                        continue;
                    }
                    tracing::info!(kind = ?note.kind, session = note.session, "notification");
                    let shared = app.state::<Shared>();
                    let id = shared.next_note.fetch_add(1, Ordering::SeqCst) + 1;
                    if let Ok(mut notes) = shared.notes.lock() {
                        if notes.len() >= NOTES_REMEMBERED {
                            notes.clear();
                        }
                        notes.insert(id, note.session.clone());
                    }
                    let _ = app
                        .notification()
                        .builder()
                        .id(id)
                        .title(note.title)
                        .body(note.body)
                        .show();
                }
                if let Ok(mut e) = app.state::<Shared>().engine.lock() {
                    *e = None;
                }
                if let Some(tray) = app.tray_by_id(TRAY_ID) {
                    let _ = tray.set_tooltip(Some("Thrall · Reconnecting"));
                }
            }
            // A companion PC its Thrall doesn't let in any more: offer to join again instead of
            // trying the same thing over and over.
            Err(e)
                if engine::companion(&paths)
                    && e.to_string().contains(thrall_core::constants::NOT_JOINED) =>
            {
                tracing::warn!(%e, "the Thrall this PC joined doesn't let it in");
                offer_rejoin(&app, thrall_core::constants::NOT_JOINED).await;
            }
            Err(e) => {
                loading_message(&app, &format!("{e}. Trying again…"));
                tracing::warn!(%e, "couldn't reach the engine");
            }
        }
        tokio::time::sleep(RECONNECT_DELAY).await;
    }
}

/// Redraw the tray icon: on every frame while Thrall works, otherwise only when the state changes.
async fn animate_tray(app: AppHandle) {
    let mut tick = tokio::time::interval(TRAY_FRAME);
    let mut current = Activity::Idle;
    let mut since = Instant::now();
    let mut first = true;
    loop {
        tick.tick().await;
        let now = Instant::now();
        let shown = match app.state::<Shared>().watch.lock() {
            Ok(mut w) => w.shown(now, DONE_HOLD),
            Err(_) => continue,
        };
        let changed = shown != current || first;
        if changed {
            current = shown;
            since = now;
            first = false;
        }
        if !(changed || current.animated()) {
            continue;
        }
        if let Some(tray) = app.tray_by_id(TRAY_ID) {
            let t = now.duration_since(since).as_secs_f32();
            let _ = tray.set_icon(Some(icon(current, t)));
            if changed {
                let _ = tray.set_tooltip(Some(format!("Thrall · {}", current.label())));
            }
        }
    }
}
