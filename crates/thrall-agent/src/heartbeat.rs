//! Check-ins (the heartbeat): now and then Thrall goes through the person's checklist
//! (`HEARTBEAT.md` in the data folder) in Home, with read-only tools and nobody to ask. If nothing
//! needs them it answers HEARTBEAT_OK and the check-in is taken back, so they never see it;
//! otherwise its message stays in Home and clients notify (an `agent.completed` with the
//! check-in's id). Off until switched on (`heartbeat.enabled`); only within `active_hours`.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use chrono::{Local, Timelike};
use thrall_core::config::in_hours;
use thrall_core::constants::HEARTBEAT_FILE;
use thrall_core::ids::now_ms;
use thrall_protocol::constants::CHECK_IN_AGENT;
use thrall_protocol::{Event, EventFrame, NoticeLevel, TurnStatus};
use tokio::sync::mpsc;

use crate::constants::{
    CHECK_IN_NOTE, CHECK_IN_SUMMARY_CHARS, CHECK_IN_TEXT, CHECK_IN_TITLE, EVENT_CHANNEL_CAPACITY,
    HEARTBEAT_OK, HEARTBEAT_OK_SLACK_CHARS, MS_PER_MINUTE,
};
use crate::error::AgentError;
use crate::runners::announce;
use crate::state::{AppState, CheckIn};

/// How a check-in went.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// There's no checklist, so there was nothing to do.
    NoChecklist,
    /// Thrall was already working in Home; it'll try at the next one.
    Busy,
    /// Nothing needed the person: the check-in was taken back.
    AllClear,
    /// Thrall had something to say: its message stays in Home.
    Posted(String),
    /// The model call failed (the error shows in Home).
    Failed,
}

/// The person's checklist ("" when there isn't one).
pub fn checklist(state: &AppState) -> String {
    state
        .data_dir
        .as_ref()
        .and_then(|d| std::fs::read_to_string(d.join(HEARTBEAT_FILE)).ok())
        .map(|t| t.trim().to_owned())
        .unwrap_or_default()
}

/// Save the checklist (empty removes it, which stops check-ins).
pub fn save_checklist(state: &AppState, text: &str) -> Result<(), AgentError> {
    let Some(dir) = state.data_dir.as_ref() else {
        return Err(AgentError::InputRefused(
            "This engine has no data folder to keep a checklist in.".to_owned(),
        ));
    };
    let path = dir.join(HEARTBEAT_FILE);
    let result = if text.trim().is_empty() {
        match std::fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    } else {
        std::fs::write(&path, format!("{}\n", text.trim()))
    };
    result.map_err(|e| AgentError::InputRefused(format!("Couldn't save the checklist: {e}")))
}

/// When the next check-in is due, Unix ms (`None` while they're off).
pub fn next_due(state: &AppState) -> Option<i64> {
    let h = &state.config.heartbeat;
    h.enabled.then(|| {
        state.last_beat.load(Ordering::SeqCst) + i64::from(h.every_minutes) * MS_PER_MINUTE
    })
}

/// Whether a reply is an all-clear: HEARTBEAT_OK at its start or end, with little else.
pub fn is_all_clear(reply: &str) -> bool {
    let reply = reply.trim();
    let rest = if let Some(r) = reply.strip_prefix(HEARTBEAT_OK) {
        r
    } else if let Some(r) = reply.strip_suffix(HEARTBEAT_OK) {
        r
    } else {
        return false;
    };
    rest.trim().chars().count() <= HEARTBEAT_OK_SLACK_CHARS
}

/// Start a check-in if one is due (called on each scheduler tick).
pub fn maybe_beat(state: &Arc<AppState>) {
    let h = &state.config.heartbeat;
    let now = now_ms();
    let due = next_due(state).is_some_and(|at| now >= at);
    let minute = {
        let t = Local::now();
        t.hour() * 60 + t.minute()
    };
    if !due || !in_hours(&h.active_hours, minute) || checklist(state).is_empty() {
        return;
    }
    // Booked before it runs, so the next tick can't start a second one.
    state.last_beat.store(now, Ordering::SeqCst);
    let st = state.clone();
    tokio::spawn(async move {
        if let Err(e) = beat(&st, false).await {
            tracing::warn!(%e, "check-in failed");
        }
    });
}

/// One check-in now, in Home. `asked`: the person pressed Check now, so an all-clear says so.
pub async fn beat(state: &AppState, asked: bool) -> Result<Outcome, AgentError> {
    let list = checklist(state);
    if list.is_empty() {
        return Ok(Outcome::NoChecklist);
    }
    let page = crate::home::current(state).await?;
    if state.is_running(&page.id) {
        return Ok(Outcome::Busy);
    }
    state.last_beat.store(now_ms(), Ordering::SeqCst);
    let time = Local::now().format("%A %-d %B, %H:%M").to_string();
    state.set_unattended(&page.id, true);
    state.set_check_in(
        &page.id,
        Some(CheckIn {
            model: state.config.heartbeat.model.clone(),
            note: format!("{}{list}", CHECK_IN_NOTE.replace("{time}", &time)),
        }),
    );
    // The check-in's events reach clients like any turn's.
    let (tx, mut rx) = mpsc::channel::<EventFrame>(EVENT_CHANNEL_CAPACITY);
    let notices = state.notices.clone();
    let forward = tokio::spawn(async move {
        while let Some(frame) = rx.recv().await {
            let _ = notices.send(frame);
        }
    });
    let result = crate::turn::run(state, &page.id, CHECK_IN_TEXT, &tx).await;
    state.set_check_in(&page.id, None);
    state.set_unattended(&page.id, false);
    drop(tx);
    let _ = forward.await;

    let summary = result?;
    if summary.status != TurnStatus::Done {
        return Ok(Outcome::Failed);
    }
    if is_all_clear(&summary.reply) {
        state
            .store
            .retract_turn(&state.user_id, &page.id, &summary.turn_id, now_ms())
            .await?;
        announce(
            state,
            &page.id,
            Event::TurnRetracted {
                turn: summary.turn_id.clone(),
            },
        )
        .await;
        if asked {
            announce(
                state,
                &page.id,
                Event::Notice {
                    level: NoticeLevel::Info,
                    text: format!(
                        "Checked at {}: nothing needs you.",
                        Local::now().format("%H:%M")
                    ),
                },
            )
            .await;
        }
        return Ok(Outcome::AllClear);
    }
    let line: String = summary
        .reply
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or_default()
        .chars()
        .take(CHECK_IN_SUMMARY_CHARS)
        .collect();
    announce(
        state,
        &page.id,
        Event::AgentCompleted {
            agent: CHECK_IN_AGENT.to_owned(),
            title: CHECK_IN_TITLE.to_owned(),
            status: "done".to_owned(),
            summary: line.clone(),
        },
    )
    .await;
    Ok(Outcome::Posted(line))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_clears_are_recognised() {
        assert!(is_all_clear("HEARTBEAT_OK"));
        assert!(is_all_clear("  HEARTBEAT_OK\n"));
        assert!(is_all_clear("All quiet. HEARTBEAT_OK"));
        assert!(!is_all_clear("Your build failed overnight."));
        assert!(!is_all_clear(
            "The word HEARTBEAT_OK in the middle is not one."
        ));
        assert!(!is_all_clear(&format!("HEARTBEAT_OK {}", "x".repeat(400))));
    }
}
