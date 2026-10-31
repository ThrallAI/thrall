//! Home: Thrall's own built-in conversation with the person, apart from their project sessions.
//! It's kept in pages: each page is a session marked with the local day it started, and a fresh
//! page starts each day or week (`home.fresh_start`). Earlier pages stay readable for
//! `home.keep_days`, then they're deleted. Home pages never appear in the session list. Home works
//! in its own folder inside the data folder, away from Thrall's database and settings.

use chrono::{Datelike, Local, NaiveDate};
use thrall_core::config::FreshStart;
use thrall_core::constants::HOME_DIR;
use thrall_core::ids::now_ms;
use thrall_store::Session;

use crate::constants::{HOME_DATE_FORMAT, HOME_DAY_FORMAT, HOME_TITLE, MS_PER_DAY};
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

/// The page to use now, starting a fresh one when the last has had its day (one at a time, so
/// only one fresh page starts).
pub async fn current(state: &AppState) -> Result<Session, AgentError> {
    let _one_at_a_time = state.home_lock.lock().await;
    let fresh = state.config.home.fresh_start;
    let today = today();
    let pages = state.store.home_sessions(&state.user_id).await?;
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
            let workspace = state.my_dir().map(|d| {
                let dir = d.join(HOME_DIR);
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
