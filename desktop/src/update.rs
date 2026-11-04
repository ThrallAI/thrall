//! Updates for an installed Thrall. The app looks for a newer version on GitHub when it starts and
//! every few hours, says so once (a notification, and the tray menu's update item), and installs
//! it in one click: Update in the tray, or in Settings (through the engine). Updates are signed;
//! one that isn't signed with Thrall's key is refused. There's no admin prompt, since Thrall is
//! installed for this user. A copy built from source (a debug build) never updates itself.

use std::sync::atomic::Ordering;

use tauri::{AppHandle, Manager};
use tauri_plugin_dialog::{
    DialogExt, MessageDialogButtons, MessageDialogKind, MessageDialogResult,
};
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_updater::UpdaterExt;

use crate::constants::{
    ENGINE_POLL, UPDATE_BUSY, UPDATE_CHECK_LABEL, UPDATE_ENGINE_STOP_WAIT, UPDATE_EVERY,
    UPDATE_FAILED_TITLE, UPDATE_FIRST_LOOK, UPDATE_FROM_SOURCE, UPDATE_LATER, UPDATE_NONE,
    UPDATE_NOW, UPDATE_READY_BODY, UPDATE_READY_LABEL, UPDATE_READY_TITLE, UPDATE_TITLE,
};
use crate::{Shared, engine};

/// Whether this copy updates itself (an installed one, not a build from source).
fn updates_itself() -> bool {
    !cfg!(debug_assertions)
}

fn say(app: &AppHandle, title: &str, body: &str) {
    let _ = app.notification().builder().title(title).body(body).show();
}

/// The newer version on offer, if there is one. It's shown in the tray, and said once.
async fn look(app: &AppHandle) -> Result<Option<String>, String> {
    let found = app
        .updater()
        .map_err(|e| e.to_string())?
        .check()
        .await
        .map_err(|e| format!("Couldn't reach GitHub: {e}"))?;
    let Some(update) = found else {
        return Ok(None);
    };
    let version = update.version.clone();
    let shared = app.state::<Shared>();
    let new = shared
        .update_offered
        .lock()
        .map(|mut offered| offered.replace(version.clone()).as_ref() != Some(&version))
        .unwrap_or(false);
    if let Some(item) = shared.update_item.lock().ok().and_then(|i| i.clone()) {
        let _ = item.set_text(UPDATE_READY_LABEL.replace("{version}", &version));
    }
    if new {
        tracing::info!(version, "a newer version is out");
        say(
            app,
            &UPDATE_READY_TITLE.replace("{version}", &version),
            UPDATE_READY_BODY,
        );
    }
    Ok(Some(version))
}

/// Look for updates for as long as the app runs.
pub async fn watch(app: AppHandle) {
    if !updates_itself() {
        return;
    }
    tokio::time::sleep(UPDATE_FIRST_LOOK).await;
    loop {
        if let Err(e) = look(&app).await {
            tracing::warn!(%e, "couldn't look for updates");
        }
        tokio::time::sleep(UPDATE_EVERY).await;
    }
}

/// The tray's update item: install the update on offer, or look for one now.
pub async fn tray_clicked(app: AppHandle) {
    if !updates_itself() {
        say(&app, UPDATE_TITLE, UPDATE_FROM_SOURCE);
        return;
    }
    let offered = app
        .state::<Shared>()
        .update_offered
        .lock()
        .ok()
        .and_then(|o| o.clone());
    if offered.is_some() {
        install(app).await;
        return;
    }
    match look(&app).await {
        Ok(Some(_)) => {}
        Ok(None) => say(&app, UPDATE_CHECK_LABEL, UPDATE_NONE),
        Err(e) => say(&app, UPDATE_FAILED_TITLE, &e),
    }
}

/// Whether Thrall is in the middle of something: the tray's mark is moving (a turn, a helper, a
/// card waiting for an answer).
fn busy(app: &AppHandle) -> bool {
    app.state::<Shared>()
        .watch
        .lock()
        .map(|mut w| {
            w.shown(std::time::Instant::now(), std::time::Duration::ZERO)
                .animated()
        })
        .unwrap_or(false)
}

/// Ask whether to update while Thrall is busy.
async fn update_anyway(app: &AppHandle) -> bool {
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .message(UPDATE_BUSY)
        .title(UPDATE_TITLE)
        .kind(MessageDialogKind::Info)
        .buttons(MessageDialogButtons::OkCancelCustom(
            UPDATE_NOW.to_owned(),
            UPDATE_LATER.to_owned(),
        ))
        .show_with_result(move |result| {
            let yes = match result {
                MessageDialogResult::Custom(s) => s == UPDATE_NOW,
                MessageDialogResult::Ok | MessageDialogResult::Yes => true,
                _ => false,
            };
            let _ = tx.send(yes);
        });
    rx.await.unwrap_or(false)
}

/// Download and install the newest version, then restart (asked from the tray or Settings).
pub async fn install(app: AppHandle) {
    if !updates_itself() {
        say(&app, UPDATE_TITLE, UPDATE_FROM_SOURCE);
        return;
    }
    let shared = app.state::<Shared>();
    if shared.updating.swap(true, Ordering::SeqCst) {
        return;
    }
    if let Err(e) = install_now(&app).await {
        tracing::warn!(%e, "update failed");
        say(&app, UPDATE_FAILED_TITLE, &e);
    }
    // Only reached when nothing was installed: installing ends or restarts the app.
    shared.updating.store(false, Ordering::SeqCst);
}

async fn install_now(app: &AppHandle) -> Result<(), String> {
    let found = app
        .updater()
        .map_err(|e| e.to_string())?
        .check()
        .await
        .map_err(|e| format!("Couldn't reach GitHub: {e}"))?;
    let Some(update) = found else {
        say(app, UPDATE_CHECK_LABEL, UPDATE_NONE);
        return Ok(());
    };
    if busy(app) && !update_anyway(app).await {
        return Ok(());
    }
    tracing::info!(version = update.version, "downloading the update");
    // Downloaded, and its signature checked, while Thrall keeps working.
    let bytes = update
        .download(|_, _| {}, || {})
        .await
        .map_err(|e| format!("The download failed: {e}"))?;
    stop_engine(app).await;
    tracing::info!(version = update.version, "installing the update");
    // On Windows the installer takes over from here and starts Thrall again.
    update
        .install(bytes)
        .map_err(|e| format!("Installing failed: {e}"))?;
    app.restart();
}

/// Stop the engine (the installer replaces it) and wait for it to go.
async fn stop_engine(app: &AppHandle) {
    let out = app
        .state::<Shared>()
        .engine
        .lock()
        .ok()
        .and_then(|e| e.clone());
    if let Some(out) = out {
        let _ = out.send(engine::stop_frame());
    }
    let Ok(paths) = engine::paths() else {
        return;
    };
    let stopped = tokio::time::timeout(UPDATE_ENGINE_STOP_WAIT, async {
        while paths.engine_file.exists() {
            tokio::time::sleep(ENGINE_POLL).await;
        }
    })
    .await;
    if stopped.is_err() {
        tracing::warn!("the engine didn't stop in time; installing anyway");
    }
}
