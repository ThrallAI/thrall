//! Reading the person's other sessions: `sessions_list` (what there is and what each is doing) and
//! `session_read` (what was said and done in one). Both only read. What a session is doing comes
//! from its stored events, so it's right whichever client started it.

use futures::future::BoxFuture;
use serde::Deserialize;
use serde_json::{Value, json};
use thrall_protocol::{Event, Role};
use thrall_store::{Session, Store, StoredEvent};

use crate::constants::{
    SESSION_READ_DEFAULT_LAST, SESSION_READ_ENTRY_MAX_CHARS, SESSION_READ_MAX_CHARS,
    SESSION_SEARCH_MAX_HITS, SESSION_STATE_TAIL_EVENTS, SESSIONS_LIST_DEFAULT, SESSIONS_LIST_MAX,
};
use crate::tool::{Tool, ToolContext, ToolOutput, arg_str, cap, parse_args};
use crate::tools::schedule::local_time;

/// What a session is doing, from its latest events.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Doing {
    /// Nothing running.
    Idle,
    /// A turn is running.
    Working,
    /// Waiting for the person to answer a question or approve a plan, since (Unix ms).
    Waiting(i64),
}

/// What a session is doing, from events oldest first.
pub fn doing(events: &[StoredEvent]) -> Doing {
    let mut now = Doing::Idle;
    for e in events {
        now = match e.kind.as_str() {
            "turn.started" | "permission.resolved" | "plan.resolved" => Doing::Working,
            "turn.completed" => Doing::Idle,
            "permission.request" | "plan.proposed" => Doing::Waiting(e.at),
            _ => continue,
        };
    }
    now
}

/// One thing that happened in a session, for reading back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// The turn it belongs to.
    pub turn: String,
    /// When, Unix ms.
    pub at: i64,
    /// "You", "Thrall", "Step" or "Asked".
    pub who: &'static str,
    /// What was said or done.
    pub text: String,
}

/// A session's conversation from its events, oldest first: messages, steps and questions, without
/// turns that were taken back (rewind, edit, retry).
pub fn transcript(events: &[StoredEvent]) -> Vec<Entry> {
    let mut roles: std::collections::HashMap<String, (String, Role)> = Default::default();
    let mut steps: std::collections::HashMap<String, (String, String)> = Default::default();
    let mut out: Vec<Entry> = Vec::new();
    for e in events {
        let Ok(event) = serde_json::from_str::<Event>(&e.payload) else {
            continue;
        };
        match event {
            Event::MessageStarted {
                turn,
                message,
                role,
            } => {
                roles.insert(message, (turn, role));
            }
            Event::MessageCompleted { message, text, .. } => {
                if let Some((turn, role)) = roles.remove(&message)
                    && !text.trim().is_empty()
                {
                    out.push(Entry {
                        turn,
                        at: e.at,
                        who: match role {
                            Role::User => "You",
                            Role::Assistant => "Thrall",
                        },
                        text,
                    });
                }
            }
            Event::StepStarted {
                turn, step, title, ..
            } => {
                steps.insert(step, (turn, title));
            }
            Event::StepCompleted { step, summary, .. } => {
                if let Some((turn, title)) = steps.remove(&step) {
                    let text = if summary.is_empty() {
                        title
                    } else {
                        format!("{title} · {summary}")
                    };
                    out.push(Entry {
                        turn,
                        at: e.at,
                        who: "Step",
                        text,
                    });
                }
            }
            Event::PermissionRequest { turn, summary, .. } => out.push(Entry {
                turn,
                at: e.at,
                who: "Asked",
                text: summary,
            }),
            Event::TurnRetracted { turn } => {
                if let Some(first) = out.iter().position(|x| x.turn == turn) {
                    out.truncate(first);
                }
            }
            _ => {}
        }
    }
    out
}

/// Every stored event of a session, oldest first.
async fn all_events(
    store: &dyn Store,
    user_id: &str,
    session_id: &str,
) -> Result<Vec<StoredEvent>, String> {
    let mut events = Vec::new();
    loop {
        let after = events.last().map_or(0, |e: &StoredEvent| e.seq);
        let page = store
            .events_after(user_id, session_id, after, u32::MAX)
            .await
            .map_err(|e| format!("couldn't read the session: {e}"))?;
        if page.is_empty() {
            return Ok(events);
        }
        events.extend(page);
    }
}

/// The person's sessions and Home pages, newest first.
async fn every_session(ctx: &ToolContext, limit: u32) -> Result<Vec<Session>, String> {
    let fail = |e: thrall_store::StoreError| format!("couldn't list sessions: {e}");
    let mut all = ctx
        .store
        .list_sessions(&ctx.user_id, limit)
        .await
        .map_err(fail)?;
    all.extend(ctx.store.home_sessions(&ctx.user_id).await.map_err(fail)?);
    all.sort_by_key(|s| std::cmp::Reverse(s.updated_at));
    Ok(all)
}

/// The session `wanted` names: its id, or words from its title (one match only).
async fn find(ctx: &ToolContext, wanted: &str) -> Result<Session, String> {
    let wanted = wanted.trim();
    if let Ok(Some(s)) = ctx.store.get_session(&ctx.user_id, wanted).await {
        return Ok(s);
    }
    let lower = wanted.to_lowercase();
    let mut matches: Vec<Session> = every_session(ctx, SESSIONS_LIST_MAX)
        .await?
        .into_iter()
        .filter(|s| s.title.to_lowercase().contains(&lower))
        .collect();
    match matches.len() {
        1 => Ok(matches.swap_remove(0)),
        0 => Err(format!(
            "no session is called or titled \"{wanted}\" (sessions_list shows them)"
        )),
        _ => Err(format!(
            "\"{wanted}\" matches {} sessions; use the id: {}",
            matches.len(),
            matches
                .iter()
                .map(|s| format!("{} ({})", s.id, s.title))
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

// ---------- sessions_list ----------

/// List the person's sessions and what each is doing.
pub struct SessionsList;

#[derive(Deserialize)]
struct ListArgs {
    #[serde(default)]
    limit: Option<u32>,
}

impl Tool for SessionsList {
    fn name(&self) -> &'static str {
        "sessions_list"
    }
    fn description(&self) -> &'static str {
        "List the person's sessions (and Home pages), newest first: id, title, folder, last \
         activity and what each is doing now (working, waiting for the person, idle). Read one \
         with session_read."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "limit": { "type": "integer", "description": "How many (default 30)" }
            }
        })
    }
    fn title(&self, _args: &Value) -> String {
        "Looking at your sessions".to_owned()
    }
    fn run<'a>(&'a self, ctx: &'a ToolContext, args: Value) -> BoxFuture<'a, ToolOutput> {
        Box::pin(async move {
            let a: ListArgs = match parse_args(self.name(), args) {
                Ok(a) => a,
                Err(e) => return ToolOutput::error(e),
            };
            let limit = a
                .limit
                .unwrap_or(SESSIONS_LIST_DEFAULT)
                .clamp(1, SESSIONS_LIST_MAX);
            let sessions = match every_session(ctx, limit).await {
                Ok(s) => s,
                Err(e) => return ToolOutput::error(e),
            };
            let mut lines = Vec::new();
            for s in sessions.iter().take(limit as usize) {
                let after = s.last_seq.saturating_sub(SESSION_STATE_TAIL_EVENTS);
                let tail = ctx
                    .store
                    .events_after(&ctx.user_id, &s.id, after, u32::MAX)
                    .await
                    .unwrap_or_default();
                let doing = match doing(&tail) {
                    Doing::Idle => "idle".to_owned(),
                    Doing::Working => "working".to_owned(),
                    Doing::Waiting(since) => {
                        format!("waiting for the person since {}", local_time(since))
                    }
                };
                let marks = [
                    (s.id == ctx.session_id).then_some("this session"),
                    s.home_day.is_some().then_some("Home"),
                ]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(", ");
                let marks = if marks.is_empty() {
                    String::new()
                } else {
                    format!(" [{marks}]")
                };
                lines.push(format!(
                    "- {} · {}{marks} · folder {} · last active {} · {doing}",
                    s.id,
                    if s.title.is_empty() {
                        "(untitled)"
                    } else {
                        &s.title
                    },
                    s.workspace.as_deref().unwrap_or("none"),
                    local_time(s.updated_at),
                ));
            }
            if lines.is_empty() {
                return ToolOutput::ok("No sessions yet.", "No sessions");
            }
            let count = lines.len();
            ToolOutput::ok(lines.join("\n"), format!("{count} sessions"))
        })
    }
}

// ---------- session_read ----------

/// Read what was said and done in another session.
pub struct SessionRead;

#[derive(Deserialize)]
struct ReadArgs {
    session: String,
    #[serde(default)]
    last: Option<usize>,
    #[serde(default)]
    search: Option<String>,
}

fn line(entry: &Entry) -> String {
    let text: String = entry
        .text
        .chars()
        .take(SESSION_READ_ENTRY_MAX_CHARS)
        .collect();
    let more = if entry.text.chars().count() > SESSION_READ_ENTRY_MAX_CHARS {
        " …"
    } else {
        ""
    };
    format!("[{}] {}: {text}{more}", local_time(entry.at), entry.who)
}

impl Tool for SessionRead {
    fn name(&self) -> &'static str {
        "session_read"
    }
    fn description(&self) -> &'static str {
        "Read another session (or an earlier Home page): what the person and Thrall said and the \
         steps taken, oldest first. session is its id or words from its title. By default the \
         last 40 entries; last for more or fewer, or search for entries containing words."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "session": { "type": "string", "description": "Session id, or words from its title" },
                "last": { "type": "integer", "description": "How many entries from the end (default 40)" },
                "search": { "type": "string", "description": "Only entries containing these words" }
            },
            "required": ["session"]
        })
    }
    fn title(&self, args: &Value) -> String {
        format!("Reading session {}", arg_str(args, "session"))
    }
    fn run<'a>(&'a self, ctx: &'a ToolContext, args: Value) -> BoxFuture<'a, ToolOutput> {
        Box::pin(async move {
            let a: ReadArgs = match parse_args(self.name(), args) {
                Ok(a) => a,
                Err(e) => return ToolOutput::error(e),
            };
            let s = match find(ctx, &a.session).await {
                Ok(s) => s,
                Err(e) => return ToolOutput::error(e),
            };
            let events = match all_events(ctx.store.as_ref(), &ctx.user_id, &s.id).await {
                Ok(e) => e,
                Err(e) => return ToolOutput::error(e),
            };
            let entries = transcript(&events);
            let picked: Vec<&Entry> = match a.search.as_deref().map(str::trim) {
                Some(words) if !words.is_empty() => {
                    let words: Vec<String> =
                        words.split_whitespace().map(str::to_lowercase).collect();
                    entries
                        .iter()
                        .filter(|e| {
                            let text = e.text.to_lowercase();
                            words.iter().all(|w| text.contains(w))
                        })
                        .take(SESSION_SEARCH_MAX_HITS)
                        .collect()
                }
                _ => {
                    let last = a.last.unwrap_or(SESSION_READ_DEFAULT_LAST).max(1);
                    entries
                        .iter()
                        .skip(entries.len().saturating_sub(last))
                        .collect()
                }
            };
            let head = format!(
                "{} · {} · folder {} · {} entries in all",
                s.id,
                if s.title.is_empty() {
                    "(untitled)"
                } else {
                    &s.title
                },
                s.workspace.as_deref().unwrap_or("none"),
                entries.len()
            );
            if picked.is_empty() {
                return ToolOutput::ok(format!("{head}\nNothing matched."), "Nothing matched");
            }
            let body = picked
                .iter()
                .map(|e| line(e))
                .collect::<Vec<_>>()
                .join("\n");
            let shown = picked.len();
            ToolOutput::ok(
                cap(format!("{head}\n{body}"), SESSION_READ_MAX_CHARS),
                format!("{shown} entries"),
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(seq: u64, at: i64, event: serde_json::Value) -> StoredEvent {
        StoredEvent {
            session_id: "s".into(),
            seq,
            turn_id: None,
            kind: event["type"].as_str().unwrap_or_default().to_owned(),
            payload: event.to_string(),
            at,
        }
    }

    fn turn(n: u64, id: &str, said: &str, reply: &str) -> Vec<StoredEvent> {
        let at = n as i64 * 100;
        vec![
            ev(n * 10, at, json!({"type": "turn.started", "turn": id})),
            ev(
                n * 10 + 1,
                at,
                json!({"type": "message.started", "turn": id, "message": format!("{id}u"), "role": "user"}),
            ),
            ev(
                n * 10 + 2,
                at,
                json!({"type": "message.completed", "message": format!("{id}u"), "text": said}),
            ),
            ev(
                n * 10 + 3,
                at,
                json!({"type": "step.started", "turn": id, "step": format!("{id}s"), "tool": "read_file", "title": "Reading a.md"}),
            ),
            ev(
                n * 10 + 4,
                at,
                json!({"type": "step.completed", "step": format!("{id}s"), "status": "ok", "summary": "12 lines"}),
            ),
            ev(
                n * 10 + 5,
                at,
                json!({"type": "message.started", "turn": id, "message": format!("{id}a"), "role": "assistant"}),
            ),
            ev(
                n * 10 + 6,
                at,
                json!({"type": "message.completed", "message": format!("{id}a"), "text": reply}),
            ),
            ev(
                n * 10 + 7,
                at,
                json!({"type": "turn.completed", "turn": id, "status": "done", "usage": {"input_tokens": 1, "output_tokens": 1, "cost_usd": 0.0}}),
            ),
        ]
    }

    #[test]
    fn transcripts_keep_messages_and_steps_and_drop_taken_back_turns() {
        let mut events = turn(1, "t1", "hello", "hi");
        events.extend(turn(2, "t2", "oops", "undone"));
        let entries = transcript(&events);
        assert_eq!(entries.len(), 6);
        assert_eq!(entries[0].who, "You");
        assert_eq!(entries[1].text, "Reading a.md · 12 lines");
        assert_eq!(entries[2].who, "Thrall");
        events.push(ev(99, 999, json!({"type": "turn.retracted", "turn": "t2"})));
        let entries = transcript(&events);
        assert_eq!(entries.len(), 3, "the retracted turn is gone");
        assert!(entries.iter().all(|e| e.turn == "t1"));
    }

    #[test]
    fn what_a_session_is_doing_comes_from_its_last_events() {
        let mut events = turn(1, "t1", "hi", "hello");
        assert_eq!(doing(&events), Doing::Idle);
        events.push(ev(50, 500, json!({"type": "turn.started", "turn": "t2"})));
        assert_eq!(doing(&events), Doing::Working);
        events.push(ev(
            51,
            600,
            json!({"type": "plan.proposed", "plan": "p", "markdown": "x"}),
        ));
        assert_eq!(doing(&events), Doing::Waiting(600));
        events.push(ev(
            52,
            700,
            json!({"type": "plan.resolved", "plan": "p", "approved": true, "answered_by": "web"}),
        ));
        assert_eq!(doing(&events), Doing::Working);
    }
}
