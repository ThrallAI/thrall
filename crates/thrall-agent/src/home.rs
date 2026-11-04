//! Home: Thrall's own built-in conversation with the person, apart from their project sessions.
//! It's kept in pages: each page is a session marked with the local day it started, and a fresh
//! page starts each day or week (`home.fresh_start`). Earlier pages stay readable for
//! `home.keep_days`, then they're deleted. Home pages never appear in the session list. Home works
//! in its own folder in Thrall's working folder (`Documents\Thrall\Home`), where what it makes is
//! easy to find; before 0.2.2 that folder was inside the data folder, and it's moved on first use.

use std::path::{Path, PathBuf};

use chrono::{Datelike, Local, NaiveDate};
use thrall_core::config::FreshStart;
use thrall_core::constants::{HOME_DIR, HOME_FOLDER};
use thrall_core::ids::now_ms;
use thrall_store::Session;

use crate::constants::{
    HOME_DATE_FORMAT, HOME_DAY_FORMAT, HOME_NAME_KEEPS, HOME_TITLE, MS_PER_DAY,
};
use crate::error::AgentError;
use crate::session;
use crate::state::AppState;

/// Today, on this computer's clock.
pub fn today() -> NaiveDate {
    Local::now().date_naive()
}

/// Whether a page that started on `day` (`YYYY-MM-DD`) is still the one to use on `today`.
pub fn is_current(day: &str, fresh: FreshStart, today: NaiveDate) -> bool {
    let Ok(started) = NaiveDate::parse_from_str(day, HOME_DAY_FORMAT) else {
        return false;
    };
    match fresh {
        FreshStart::Daily => started == today,
        FreshStart::Weekly => started.iso_week() == today.iso_week(),
        FreshStart::Never => true,
    }
}

/// A new page's title.
pub fn title(day: NaiveDate, fresh: FreshStart) -> String {
    match fresh {
        FreshStart::Daily => format!("{HOME_TITLE} · {}", day.format(HOME_DATE_FORMAT)),
        FreshStart::Weekly => {
            let monday = day - chrono::Days::new(u64::from(day.weekday().num_days_from_monday()));
            format!("{HOME_TITLE} · week of {}", monday.format(HOME_DATE_FORMAT))
        }
        FreshStart::Never => HOME_TITLE.to_owned(),
    }
}

/// A person's name as a folder name: letters, digits, spaces, `-` and `_` only.
pub fn folder_name(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_alphanumeric() || HOME_NAME_KEEPS.contains(c))
        .collect::<String>()
        .trim()
        .to_owned()
}

/// Where Home works for whoever `state` acts for: `Home` in Thrall's working folder for the owner,
/// `Home - <name>` beside it for anyone else. With no working folder, inside the person's data
/// folder (as before 0.2.2).
async fn folder(state: &AppState) -> Result<Option<PathBuf>, AgentError> {
    let Some(work) = &state.work_dir else {
        return Ok(state.my_dir().map(|d| d.join(HOME_DIR)));
    };
    if state.user_id == state.owner_id {
        return Ok(Some(work.join(HOME_FOLDER)));
    }
    let name = state
        .store
        .get_person(&state.user_id)
        .await?
        .map(|p| folder_name(&p.name))
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| state.user_id.clone());
    Ok(Some(work.join(format!("{HOME_FOLDER} - {name}"))))
}

/// Move Home's old folder in the data folder to `to`, and point the `pages` that worked there at
/// it. Whether it moved: it stays (and those pages keep working there) when `to` is already
/// there, a page is busy, or the move fails (another drive, a file open).
async fn move_old_folder(
    state: &AppState,
    pages: &[Session],
    to: &Path,
) -> Result<bool, AgentError> {
    let Some(old) = state.my_dir().map(|d| d.join(HOME_DIR)) else {
        return Ok(false);
    };
    if old == to || !old.is_dir() || to.exists() || pages.iter().any(|p| state.is_running(&p.id)) {
        return Ok(false);
    }
    if let Some(parent) = to.parent()
        && let Err(e) = std::fs::create_dir_all(parent)
    {
        tracing::warn!(%e, "couldn't make the folder for Home's files");
        return Ok(false);
    }
    if let Err(e) = std::fs::rename(&old, to) {
        tracing::warn!(%e, "couldn't move Home's folder out of the data folder");
        return Ok(false);
    }
    let (old, to) = (old.display().to_string(), to.display().to_string());
    for page in pages
        .iter()
        .filter(|p| p.workspace.as_deref() == Some(old.as_str()))
    {
        state
            .store
            .set_session_workspace(&state.user_id, &page.id, &to)
            .await?;
    }
    tracing::info!(to, "moved Home's folder out of the data folder");
    Ok(true)
}

/// The page to use now, starting a fresh one when the last has had its day (one at a time, so
/// only one fresh page starts).
pub async fn current(state: &AppState) -> Result<Session, AgentError> {
    let _one_at_a_time = state.home_lock.lock().await;
    let fresh = state.config.home.fresh_start;
    let today = today();
    let dir = folder(state).await?;
    let mut pages = state.store.home_sessions(&state.user_id).await?;
    if let Some(dir) = &dir
        && move_old_folder(state, &pages, dir).await?
    {
        pages = state.store.home_sessions(&state.user_id).await?;
    }
    if let Some(page) = pages.first()
        && page
            .home_day
            .as_deref()
            .is_some_and(|d| is_current(d, fresh, today))
    {
        return Ok(page.clone());
    }
    let day = today.format(HOME_DAY_FORMAT).to_string();
    // A page with nothing on it (or only check-ins that were taken back) becomes today's, so
    // quiet days don't leave empty pages behind.
    let unused = match pages.first() {
        Some(p) => state
            .store
            .context_messages(&state.user_id, &p.id)
            .await?
            .is_empty(),
        None => false,
    };
    let page = match pages.first().filter(|_| unused) {
        Some(unused) => unused.clone(),
        None => {
            let workspace = dir.map(|dir| {
                if let Err(e) = std::fs::create_dir_all(&dir) {
                    tracing::warn!(%e, "couldn't make Home's folder");
                }
                dir.display().to_string()
            });
            session::create(state, workspace).await?
        }
    };
    state
        .store
        .set_session_home(&state.user_id, &page.id, &day)
        .await?;
    state
        .store
        .set_session_title(&state.user_id, &page.id, &title(today, fresh))
        .await?;
    prune(state, &page.id).await?;
    session::get(state, &page.id).await
}

/// Delete earlier pages older than `home.keep_days` (never `current`, nor one Thrall is busy in).
async fn prune(state: &AppState, current: &str) -> Result<(), AgentError> {
    let keep = state.config.home.keep_days;
    if keep == 0 {
        return Ok(());
    }
    let cutoff = now_ms() - i64::from(keep) * MS_PER_DAY;
    for page in state.store.home_sessions(&state.user_id).await? {
        if page.id != current && page.updated_at < cutoff && !state.is_running(&page.id) {
            state.store.delete_session(&state.user_id, &page.id).await?;
        }
    }
    Ok(())
}

/// Earlier pages, newest first (every page but `current`).
pub async fn earlier(state: &AppState, current: &str) -> Result<Vec<Session>, AgentError> {
    let mut pages = state.store.home_sessions(&state.user_id).await?;
    pages.retain(|p| p.id != current);
    Ok(pages)
}

/// Delete every page (refused while Thrall is working in one). The next `current` starts afresh.
pub async fn clear(state: &AppState) -> Result<(), AgentError> {
    let _one_at_a_time = state.home_lock.lock().await;
    let pages = state.store.home_sessions(&state.user_id).await?;
    if pages.iter().any(|p| state.is_running(&p.id)) {
        return Err(AgentError::Busy);
    }
    for page in pages {
        state.store.delete_session(&state.user_id, &page.id).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, HOME_DAY_FORMAT).unwrap()
    }

    #[test]
    fn pages_last_a_day_a_week_or_for_good() {
        let fri = day("2026-10-16");
        assert!(is_current("2026-10-16", FreshStart::Daily, fri));
        assert!(!is_current("2026-10-15", FreshStart::Daily, fri));
        // Monday 12 Oct to Sunday 18 Oct is one week.
        assert!(is_current("2026-10-12", FreshStart::Weekly, fri));
        assert!(!is_current("2026-10-11", FreshStart::Weekly, fri));
        assert!(is_current("2020-01-01", FreshStart::Never, fri));
        assert!(!is_current("not a day", FreshStart::Never, fri));
    }

    #[test]
    fn titles_name_the_day_or_week() {
        let fri = day("2026-10-16");
        assert_eq!(title(fri, FreshStart::Daily), "Home · Fri 16 Oct");
        assert_eq!(title(fri, FreshStart::Weekly), "Home · week of Mon 12 Oct");
        assert_eq!(title(fri, FreshStart::Never), "Home");
    }
}
