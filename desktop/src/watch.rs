//! What every session is doing, from the engine's events: the tray shows the most urgent state,
//! and some events become notifications when the window isn't in front. Pure, so it's tested.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use thrall_core::config::{NotificationsConfig, in_hours};
use thrall_protocol::constants::CHECK_IN_AGENT;
use thrall_protocol::{Event, EventFrame, SessionInfo, SessionState, TurnStatus};

use crate::mark::Activity;

/// What a notification is about (each kind can be switched off in Settings).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Thrall is waiting for an answer or a plan decision.
    NeedsYou,
    /// A turn the person started finished.
    Done,
    /// A turn the person started failed.
    Failed,
    /// A helper or scheduled job finished.
    Background,
    /// A check-in found something worth saying.
    CheckIn,
}

/// A notification to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Note {
    pub kind: Kind,
    /// The session it's about (clicking it opens that session).
    pub session: String,
    pub title: String,
    pub body: String,
}

/// Whether to show a notification of `kind` now: its switch is on, it isn't quiet hours (only
/// "needs you" comes through then), and Thrall isn't in front (when that's asked for).
pub fn allowed(kind: Kind, n: &NotificationsConfig, minute: u32, in_front: bool) -> bool {
    let on = match kind {
        Kind::NeedsYou => n.needs_you,
        Kind::Done => n.done,
        Kind::Failed => n.failed,
        Kind::Background => n.background,
        Kind::CheckIn => n.check_ins,
    };
    let quiet = !n.quiet_hours.trim().is_empty() && in_hours(&n.quiet_hours, minute);
    on && (kind == Kind::NeedsYou || !quiet) && !(n.only_when_away && in_front)
}

#[derive(Default)]
pub struct Watch {
    sessions: HashMap<String, (Activity, Instant)>,
    titles: HashMap<String, String>,
    /// Sessions whose running turn nobody started (a scheduled job or a check-in).
    scheduled: HashSet<String>,
}

impl Watch {
    /// Learn session titles (and current states) from a session list.
    pub fn sessions(&mut self, items: &[SessionInfo]) {
        for s in items {
            if !s.title.is_empty() {
                self.titles.insert(s.session.clone(), s.title.clone());
            }
            let activity = match s.state {
                SessionState::Working => Activity::Thinking,
                SessionState::WaitingForYou => Activity::Waiting,
                SessionState::Idle => continue,
            };
            self.sessions
                .insert(s.session.clone(), (activity, Instant::now()));
        }
    }

    /// Whether `session`'s title is known (one the app hasn't heard of: ask for the lists).
    pub fn knows(&self, session: &str) -> bool {
        self.titles.contains_key(session)
    }

    fn title(&self, session: &str) -> String {
        self.titles
            .get(session)
            .cloned()
            .unwrap_or_else(|| "Your session".to_owned())
    }

    fn set(&mut self, session: &str, activity: Activity) {
        self.sessions
            .insert(session.to_owned(), (activity, Instant::now()));
    }

    /// Apply one event; returns a notification when it's worth one.
    pub fn apply(&mut self, frame: &EventFrame) -> Option<Note> {
        let s = frame.session.as_str();
        let note = |kind: Kind, title: String, body: String| Note {
            kind,
            session: s.to_owned(),
            title,
            body,
        };
        match &frame.event {
            Event::TurnStarted { scheduled, .. } => {
                if *scheduled {
                    self.scheduled.insert(s.to_owned());
                } else {
                    self.scheduled.remove(s);
                }
                self.set(s, Activity::Thinking);
            }
            Event::StepStarted { .. } => self.set(s, Activity::Tool),
            Event::StepCompleted { .. }
            | Event::PermissionResolved { .. }
            | Event::PlanResolved { .. }
            | Event::QuestionResolved { .. } => self.set(s, Activity::Thinking),
            Event::PermissionRequest { summary, .. } => {
                self.set(s, Activity::Waiting);
                return Some(note(
                    Kind::NeedsYou,
                    format!("Thrall needs you · {}", self.title(s)),
                    summary.replace('`', ""),
                ));
            }
            Event::PlanProposed { .. } => {
                self.set(s, Activity::Waiting);
                return Some(note(
                    Kind::NeedsYou,
                    format!("Thrall needs you · {}", self.title(s)),
                    "A plan is ready for you to approve.".to_owned(),
                ));
            }
            Event::QuestionAsk { items, .. } => {
                self.set(s, Activity::Waiting);
                let first = items.first().map(|q| q.text.clone()).unwrap_or_default();
                return Some(note(
                    Kind::NeedsYou,
                    format!("Thrall has a question · {}", self.title(s)),
                    first,
                ));
            }
            Event::SessionUpdated { title: Some(t), .. } => {
                self.titles.insert(s.to_owned(), t.clone());
            }
            Event::Status {
                state: SessionState::Idle,
                ..
            } => {
                let finished = self
                    .sessions
                    .get(s)
                    .is_some_and(|(a, _)| matches!(a, Activity::Done | Activity::Failed));
                if !finished {
                    self.sessions.remove(s);
                }
            }
            Event::TurnCompleted { status, .. } => {
                let (activity, shown) = match status {
                    TurnStatus::Done => (
                        Activity::Done,
                        Some(note(Kind::Done, "Thrall is done".to_owned(), self.title(s))),
                    ),
                    TurnStatus::Failed => (
                        Activity::Failed,
                        Some(note(
                            Kind::Failed,
                            "Thrall couldn't finish".to_owned(),
                            self.title(s),
                        )),
                    ),
                    TurnStatus::Interrupted => (Activity::Idle, None),
                };
                self.set(s, activity);
                // A job's or a check-in's turn says so in its own notification.
                if self.scheduled.remove(s) {
                    return None;
                }
                return shown;
            }
            Event::AgentCompleted {
                agent,
                title,
                summary,
                ..
            } => {
                let kind = if agent == CHECK_IN_AGENT {
                    Kind::CheckIn
                } else {
                    Kind::Background
                };
                return Some(note(kind, title.clone(), summary.clone()));
            }
            _ => {}
        }
        None
    }

    /// The tray's state: the most urgent thing any session is doing. Done and Failed show
    /// briefly, then rest.
    pub fn shown(&mut self, now: Instant, done_hold: Duration) -> Activity {
        self.sessions.retain(|_, (a, at)| {
            !(matches!(a, Activity::Done | Activity::Failed | Activity::Idle)
                && now.duration_since(*at) > done_hold)
        });
        let rank = |a: Activity| match a {
            Activity::Waiting => 6,
            Activity::Failed => 5,
            Activity::Tool => 4,
            Activity::Thinking => 3,
            Activity::Done => 2,
            Activity::Idle => 1,
        };
        self.sessions
            .values()
            .map(|(a, _)| *a)
            .max_by_key(|a| rank(*a))
            .unwrap_or(Activity::Idle)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use thrall_protocol::{PermissionChoice, Risk, Usage};

    fn frame(session: &str, event: Event) -> EventFrame {
        EventFrame::new(session, Some(1), 0, event)
    }

    #[test]
    fn the_most_urgent_session_wins_and_cards_notify() {
        let mut w = Watch::default();
        w.apply(&frame(
            "a",
            Event::TurnStarted {
                turn: "t".into(),
                scheduled: false,
            },
        ));
        let note = w.apply(&frame(
            "b",
            Event::PermissionRequest {
                request: "r".into(),
                turn: "t".into(),
                step: "p".into(),
                tool: "shell".into(),
                summary: "Run `cargo test`".into(),
                risk: Risk::Medium,
                diff: None,
                choices: vec![PermissionChoice::Yes, PermissionChoice::No],
            },
        ));
        assert_eq!(note.map(|n| n.body), Some("Run cargo test".to_owned()));
        let now = Instant::now();
        assert_eq!(w.shown(now, Duration::from_secs(2)), Activity::Waiting);
        w.apply(&frame(
            "b",
            Event::PermissionResolved {
                request: "r".into(),
                choice: PermissionChoice::Yes,
                answered_by: "web".into(),
            },
        ));
        assert_eq!(w.shown(now, Duration::from_secs(2)), Activity::Thinking);
    }

    #[test]
    fn done_shows_briefly_with_the_session_title() {
        let mut w = Watch::default();
        w.apply(&frame(
            "a",
            Event::SessionUpdated {
                title: Some("Fix the tests".into()),
                mode: None,
                mode_limit: None,
                workspace: None,
            },
        ));
        let note = w.apply(&frame(
            "a",
            Event::TurnCompleted {
                turn: "t".into(),
                status: TurnStatus::Done,
                usage: Usage::default(),
            },
        ));
        assert_eq!(note.map(|n| n.body), Some("Fix the tests".to_owned()));
        let now = Instant::now();
        assert_eq!(w.shown(now, Duration::from_secs(2)), Activity::Done);
        assert_eq!(
            w.shown(now + Duration::from_secs(3), Duration::from_secs(2)),
            Activity::Idle
        );
    }

    fn done(session: &str) -> EventFrame {
        frame(
            session,
            Event::TurnCompleted {
                turn: "t".into(),
                status: TurnStatus::Done,
                usage: Usage::default(),
            },
        )
    }

    #[test]
    fn scheduled_turns_and_check_ins_get_their_own_notes() {
        let mut w = Watch::default();
        w.apply(&frame(
            "home",
            Event::TurnStarted {
                turn: "t".into(),
                scheduled: true,
            },
        ));
        assert_eq!(
            w.apply(&done("home")),
            None,
            "no 'Thrall is done' for a check-in"
        );
        let note = w
            .apply(&frame(
                "home",
                Event::AgentCompleted {
                    agent: CHECK_IN_AGENT.into(),
                    title: "Check-in".into(),
                    status: "done".into(),
                    summary: "Your build failed.".into(),
                },
            ))
            .unwrap();
        assert_eq!((note.kind, note.session.as_str()), (Kind::CheckIn, "home"));
        // The person's own next turn is announced again.
        w.apply(&frame(
            "home",
            Event::TurnStarted {
                turn: "u".into(),
                scheduled: false,
            },
        ));
        assert_eq!(w.apply(&done("home")).map(|n| n.kind), Some(Kind::Done));
    }

    #[test]
    fn switches_quiet_hours_and_the_window_decide() {
        let mut n = NotificationsConfig::default();
        assert!(allowed(Kind::Done, &n, 600, false));
        assert!(
            !allowed(Kind::Done, &n, 600, true),
            "not while Thrall is in front"
        );
        n.only_when_away = false;
        assert!(allowed(Kind::Done, &n, 600, true));
        n.done = false;
        assert!(!allowed(Kind::Done, &n, 600, false));
        n.quiet_hours = "22:00-07:00".into();
        assert!(!allowed(Kind::CheckIn, &n, 23 * 60, false));
        assert!(
            allowed(Kind::NeedsYou, &n, 23 * 60, false),
            "needs you comes through"
        );
        assert!(allowed(Kind::CheckIn, &n, 12 * 60, false));
    }
}
