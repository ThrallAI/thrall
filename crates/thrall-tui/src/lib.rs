//! The terminal client: chat with streamed replies, collapsible tool steps, a task panel,
//! permission and plan cards, permission modes, slash commands, interrupts and session resume. It
//! is a client of the one engine, like the web and desktop apps: it sends protocol requests and
//! draws the events that come back.

pub mod app;
pub mod client;
pub mod constants;
pub mod markdown;
pub mod plain;
pub mod theme;
pub mod transcript;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use ratatui::crossterm::event::{self, Event as TermEvent, KeyEventKind};
use thrall_gates::Mode;
use thrall_protocol::{ErrorCode, Event, EventFrame, Reply, Request, SessionInfo, SessionState};
use tokio::sync::mpsc;

use crate::app::{Action, App, Picker, Reply as Answer};
use crate::client::{Client, Connected, Refused};
use crate::constants::{
    CLIENT_COMMANDS, INPUT_POLL, PICKER_SESSIONS, REWIND_BOTH, REWIND_CHAT, REWIND_FILES,
    REWIND_LINE_CHARS, REWIND_LIST_MAX, TICK,
};
use crate::transcript::Kind;

/// How the terminal client starts.
#[derive(Default)]
pub struct Launch {
    /// Open this session.
    pub resume: Option<String>,
    /// Open the most recent session.
    pub continue_last: bool,
    /// Start with the session picker.
    pub pick: bool,
    /// The folder new sessions work in (where `thrall` was started).
    pub workspace: Option<String>,
    /// A line to show when the app opens (e.g. the engine is a different version).
    pub notice: Option<String>,
}

/// A session being replayed: once its last stored event arrives, the screen is settled.
struct Replay {
    session: String,
    last_seq: u64,
    state: SessionState,
    cost: f64,
}

/// Run the terminal client until the user quits.
pub async fn run(connected: Connected, launch: Launch) -> anyhow::Result<()> {
    let Connected {
        client,
        mut events,
        engine,
        ..
    } = connected;
    let mut app = App::new(engine.model, plain::wanted());
    if let Ok(Reply::CommandListed { items }) = client.request(Request::CommandList {}).await {
        app.commands = items;
    }
    if let Some(notice) = launch.notice {
        app.info(notice);
    }
    let start = if let Some(id) = launch.resume {
        Some(Action::Resume(id))
    } else if launch.continue_last {
        match client
            .request(Request::SessionList {
                limit: Some(1),
                archived: false,
            })
            .await?
        {
            Reply::SessionListed { items } => {
                items.into_iter().next().map(|s| Action::Resume(s.session))
            }
            _ => None,
        }
    } else if launch.pick {
        Some(Action::OpenPicker)
    } else {
        None
    };

    let mut terminal = ratatui::init();
    // Bracketed paste keeps multi-line pastes from sending early, where the terminal supports it.
    let _ = ratatui::crossterm::execute!(
        std::io::stdout(),
        ratatui::crossterm::event::EnableBracketedPaste
    );
    let stop = Arc::new(AtomicBool::new(false));
    let mut keys = spawn_input_thread(stop.clone());
    let mut tick = tokio::time::interval(TICK);
    let mut replay: Option<Replay> = None;
    let workspace = launch.workspace;

    let mut pending = start;
    let mut dirty = true;
    let result: anyhow::Result<()> = loop {
        if let Some(action) = pending.take() {
            match handle(&client, &mut app, action, workspace.as_deref(), &mut replay).await {
                Ok(true) => break Ok(()),
                Ok(false) => {}
                Err(e) => app.error(e.to_string()),
            }
            dirty = true;
        }
        if dirty && let Err(e) = terminal.draw(|f| app.draw(f)) {
            break Err(e.into());
        }
        dirty = true;
        pending = tokio::select! {
            frame = events.recv() => match frame {
                Some(frame) => {
                    let finished = matches!(frame.event, Event::TurnCompleted { .. });
                    on_frame(&mut app, frame, &mut replay);
                    // A turn may have saved a new skill: its /name joins autocomplete.
                    finished.then_some(Action::RefreshCommands)
                }
                None => {
                    app.busy_since = None;
                    app.error(
                        "Lost the connection to Thrall's engine. Quit (Ctrl+C) and run thrall \
                         again to reconnect; your sessions are saved.",
                    );
                    // Keep the screen up so the message can be read.
                    events = mpsc::unbounded_channel().1;
                    None
                }
            },
            Some(input) = keys.recv() => match input {
                TermEvent::Key(key) if key.kind != KeyEventKind::Release => app.on_key(key),
                TermEvent::Paste(text) => {
                    app.on_paste(&text);
                    None
                }
                _ => None,
            },
            _ = tick.tick() => {
                if app.busy_since.is_some() {
                    app.tick();
                } else {
                    dirty = false;
                }
                None
            }
        };
    };

    stop.store(true, Ordering::Relaxed);
    let _ = ratatui::crossterm::execute!(
        std::io::stdout(),
        ratatui::crossterm::event::DisableBracketedPaste
    );
    ratatui::restore();
    if let Some(s) = &app.session {
        println!("Session {} · continue with: thrall -c", s.session);
    }
    result
}

/// Apply one event from the engine.
fn on_frame(app: &mut App, frame: EventFrame, replay: &mut Option<Replay>) {
    let here = app
        .session
        .as_ref()
        .is_some_and(|s| s.session == frame.session);
    if !here {
        // Other sessions: only word that background work finished.
        if matches!(frame.event, Event::AgentCompleted { .. }) {
            app.on_event(frame.event);
        }
        return;
    }
    if let Event::Status { state, .. } = &frame.event {
        set_busy(app, *state);
    }
    let (session, seq) = (frame.session, frame.seq);
    app.on_event(frame.event);
    let settled = replay
        .as_ref()
        .is_some_and(|r| r.session == session && seq.is_some_and(|s| s >= r.last_seq));
    if settled && let Some(r) = replay.take() {
        settle(app, &r);
    }
}

/// A replayed session is on screen: drop a card left over from a turn that's no longer waiting,
/// and show the true cost.
fn settle(app: &mut App, r: &Replay) {
    if r.state != SessionState::WaitingForYou {
        app.permission = None;
    }
    app.cost = r.cost;
    set_busy(app, r.state);
}

fn set_busy(app: &mut App, state: SessionState) {
    match state {
        SessionState::Idle => app.busy_since = None,
        SessionState::Working | SessionState::WaitingForYou => {
            if app.busy_since.is_none() {
                app.busy_since = Some(Instant::now());
            }
        }
    }
}

/// Read terminal input on its own thread (crossterm's reads block) and forward it.
fn spawn_input_thread(stop: Arc<AtomicBool>) -> mpsc::UnboundedReceiver<TermEvent> {
    let (tx, rx) = mpsc::unbounded_channel();
    std::thread::spawn(move || {
        while !stop.load(Ordering::Relaxed) {
            match event::poll(INPUT_POLL) {
                Ok(true) => match event::read() {
                    Ok(ev) => {
                        if tx.send(ev).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                },
                Ok(false) => {}
                Err(_) => break,
            }
        }
    });
    rx
}

fn opened(reply: Reply) -> anyhow::Result<SessionInfo> {
    match reply {
        Reply::SessionOpened(info) => Ok(info),
        other => anyhow::bail!("Thrall's engine sent {other:?} instead of a session."),
    }
}

/// Carry out an action. `Ok(true)` means quit.
async fn handle(
    client: &Client,
    app: &mut App,
    action: Action,
    workspace: Option<&str>,
    replay: &mut Option<Replay>,
) -> anyhow::Result<bool> {
    match action {
        Action::Quit => return Ok(true),
        Action::RefreshCommands => {
            if let Ok(Reply::CommandListed { items }) =
                client.request(Request::CommandList {}).await
            {
                app.commands = items;
            }
        }
        Action::Send(text) => {
            let session = match &app.session {
                Some(s) => s.session.clone(),
                None => {
                    let s = opened(
                        client
                            .request(Request::SessionCreate {
                                workspace: workspace.map(str::to_owned),
                                title: None,
                                mode: Some(app.mode.as_str().to_owned()),
                            })
                            .await?,
                    )?;
                    let id = s.session.clone();
                    app.session = Some(s);
                    id
                }
            };
            if app.title.is_empty() {
                app.title = text.lines().next().unwrap_or_default().to_owned();
            }
            let busy = app.busy_since.is_some();
            client
                .request(Request::TurnSend {
                    session,
                    text,
                    files: Vec::new(),
                })
                .await?;
            if busy {
                // The engine adds it to the running turn.
                app.info("Added: Thrall will see it at its next step.");
            } else {
                app.busy_since = Some(Instant::now());
            }
        }
        Action::Command { name, args } => run_command(client, app, &name, &args).await?,
        Action::SetMode(mode) => {
            if let Some(s) = &app.session {
                client
                    .request(Request::SessionSetMode {
                        session: s.session.clone(),
                        mode: mode.as_str().to_owned(),
                        until: None,
                        minutes: None,
                    })
                    .await?;
            }
            app.mode = mode;
            let when = if app.busy_since.is_some() {
                " (from Thrall's next step)"
            } else {
                ""
            };
            app.info(format!("Mode: {}{when}. {}", mode.label(), mode.covers()));
        }
        Action::Interrupt => {
            if let Some(s) = &app.session {
                client
                    .request(Request::TurnInterrupt {
                        session: s.session.clone(),
                    })
                    .await?;
                app.info("Stopping…");
            }
        }
        Action::NewSession => {
            if app.busy_since.is_some() {
                app.info("Wait for Thrall to finish this turn first (or press Esc).");
            } else {
                detach(client, app).await;
                app.reset();
                app.info("New session. It's saved once you send the first message.");
            }
        }
        Action::OpenPicker => {
            if app.busy_since.is_some() {
                app.info("Wait for Thrall to finish this turn first (or press Esc).");
            } else if let Reply::SessionListed { items } = client
                .request(Request::SessionList {
                    limit: Some(PICKER_SESSIONS),
                    archived: false,
                })
                .await?
            {
                if items.is_empty() {
                    app.info("No sessions yet.");
                } else {
                    app.picker = Some(Picker {
                        sessions: items,
                        selected: 0,
                    });
                }
            }
        }
        Action::Resume(id) => {
            if app.busy_since.is_some() {
                app.info("Wait for Thrall to finish this turn first (or press Esc).");
            } else {
                open_session(client, app, id, replay).await?;
            }
        }
        Action::Home => {
            if app.busy_since.is_some() {
                app.info("Wait for Thrall to finish this turn first (or press Esc).");
            } else if let Reply::HomeShown { page, .. } =
                client.request(Request::HomeGet {}).await?
            {
                open_session(client, app, page.session, replay).await?;
            }
        }
        Action::Answer(request, reply) => {
            let req = match reply {
                Answer::Permission(choice) => Request::PermissionAnswer { request, choice },
                Answer::Plan {
                    approve,
                    auto_edits,
                } => Request::PlanDecide {
                    plan: request,
                    approve,
                    auto_edits,
                },
            };
            if let Err(e) = client.request(req).await {
                // Answered elsewhere, or the turn is gone: the card has nothing left to do.
                app.permission = None;
                if !e
                    .downcast_ref::<Refused>()
                    .is_some_and(|r| r.code == ErrorCode::AlreadyAnswered)
                {
                    return Err(e);
                }
            }
        }
    }
    Ok(false)
}

/// Stop receiving the current session's events (it keeps running in the engine).
/// Leave the current session and load `id`, replaying what it already holds.
async fn open_session(
    client: &Client,
    app: &mut App,
    id: String,
    replay: &mut Option<Replay>,
) -> anyhow::Result<()> {
    detach(client, app).await;
    app.reset();
    let s = opened(
        client
            .request(Request::SessionOpen {
                session: id,
                after_seq: None,
            })
            .await?,
    )?;
    app.title = s.title.clone();
    app.mode = Mode::parse(&s.mode).unwrap_or_default();
    app.info(if s.home {
        format!("{}: your own conversation with Thrall.", s.title)
    } else {
        format!("Resumed “{}”.", s.title)
    });
    let r = Replay {
        session: s.session.clone(),
        last_seq: s.last_seq,
        state: s.state,
        cost: s.cost_usd,
    };
    app.session = Some(s);
    if r.last_seq == 0 {
        settle(app, &r);
    } else {
        *replay = Some(r);
    }
    Ok(())
}

async fn detach(client: &Client, app: &App) {
    if let Some(s) = &app.session {
        let _ = client
            .request(Request::SessionDetach {
                session: s.session.clone(),
            })
            .await;
    }
}

/// `/rewind` lists recent messages; `/rewind N [files|chat|both]` goes back to before the Nth
/// most recent one. Files someone changed since Thrall did are left as they are.
async fn rewind(client: &Client, app: &mut App, args: &str) -> anyhow::Result<()> {
    let Some(session) = app.session.as_ref().map(|s| s.session.clone()) else {
        app.info("Send a message first; there's nothing to rewind yet.");
        return Ok(());
    };
    if app.busy_since.is_some() {
        app.info("Wait for Thrall to finish this turn first (or press Esc).");
        return Ok(());
    }
    let mut words = args.split_whitespace();
    let pick = words.next().and_then(|w| w.parse::<usize>().ok());
    let what = words.next().unwrap_or(REWIND_BOTH);
    let Some(n) = pick.filter(|n| *n >= 1 && *n <= app.turns.len()) else {
        if app.turns.is_empty() {
            app.info("There's nothing to rewind in this session yet.");
            return Ok(());
        }
        let mut list = String::from("Rewind to before which message? (1 is the latest)\n");
        for (i, (_, _, text)) in app.turns.iter().rev().take(REWIND_LIST_MAX).enumerate() {
            let line: String = text
                .lines()
                .next()
                .unwrap_or("")
                .chars()
                .take(REWIND_LINE_CHARS)
                .collect();
            list.push_str(&format!("\n  {}. {line}", i + 1));
        }
        list.push_str(
            "\n\nThen /rewind N, or /rewind N files / chat to put back only files or only the \
             conversation.",
        );
        app.info(list);
        return Ok(());
    };
    let (files, conversation) = match what {
        REWIND_FILES => (true, false),
        REWIND_CHAT => (false, true),
        _ => (true, true),
    };
    let turn = app.turns[app.turns.len() - n].0.clone();
    let mut keep = Vec::new();
    if files
        && let Reply::CheckpointShown { files: planned, .. } = client
            .request(Request::CheckpointPreview {
                session: session.clone(),
                turn: turn.clone(),
            })
            .await?
    {
        keep = planned
            .into_iter()
            .filter(|f| f.changed)
            .map(|f| f.path)
            .collect();
    }
    let reply = client
        .request(Request::TurnRewind {
            session,
            turn,
            files,
            conversation,
            keep,
        })
        .await?;
    // The engine posts what was done as a notice; the message goes back in the box.
    if let Reply::TurnRewound { text, .. } = reply
        && conversation
    {
        app.set_input(&text);
    }
    Ok(())
}

async fn run_command(client: &Client, app: &mut App, name: &str, args: &str) -> anyhow::Result<()> {
    // Retry and edit take back the last turn (and put back the files it changed) and send again.
    if name == "retry" || name == "edit" {
        let Some(session) = app.session.as_ref().map(|s| s.session.clone()) else {
            app.info("Send a message first; there's nothing to retry yet.");
            return Ok(());
        };
        if app.busy_since.is_some() {
            app.info("Wait for Thrall to finish this turn first (or press Esc).");
            return Ok(());
        }
        let request = if name == "retry" {
            Request::TurnRetry { session }
        } else if args.trim().is_empty() {
            app.info("Type the new message after /edit, e.g. /edit summarise it in one line.");
            return Ok(());
        } else {
            Request::TurnEdit {
                session,
                text: args.trim().to_owned(),
            }
        };
        client.request(request).await?;
        app.busy_since = Some(Instant::now());
        app.info("Took back the last turn and put back the files it changed.");
        return Ok(());
    }
    if name == "rewind" {
        return rewind(client, app, args).await;
    }
    if name == "forget" {
        let fact = args.trim();
        if fact.is_empty() {
            app.info("Say which fact: /forget <id> (the id is shown when Thrall remembers one).");
            return Ok(());
        }
        match client
            .request(Request::MemoryForget {
                fact: fact.to_owned(),
            })
            .await
        {
            Ok(_) => app.info("Forgotten: Thrall won't recall it any more."),
            Err(e) => app.error(e.to_string()),
        }
        return Ok(());
    }
    let needs_session = matches!(name, "cost" | "title" | "tasks" | "status" | "compact");
    let session = app.session.as_ref().map(|s| s.session.clone());
    if needs_session && session.is_none() {
        app.info("Send a message first; that starts the session.");
        return Ok(());
    }
    // Before the first message, /mode only changes what the new session will start with.
    if name == "mode" && session.is_none() {
        match Mode::parse(args) {
            Some(mode) if !args.trim().is_empty() => {
                app.mode = mode;
                app.info(format!("Mode: {}. {}", mode.label(), mode.covers()));
            }
            _ => app.info(format!(
                "Mode: {}. Choose with /mode ask, auto, plan or full, or Shift+Tab.",
                app.mode.label()
            )),
        }
        return Ok(());
    }
    let reply = client
        .request(Request::CommandRun {
            session,
            name: name.to_owned(),
            args: args.to_owned(),
        })
        .await?;
    let Reply::CommandResult { mut markdown } = reply else {
        return Ok(());
    };
    if name == "help" {
        for (client_name, summary) in CLIENT_COMMANDS {
            markdown.push_str(&format!("- `/{client_name}`: {summary}\n"));
        }
        markdown.push_str(
            "\n**Keys:** Shift+Tab mode · Esc stop · Ctrl+O step details · PgUp/PgDn \
             scroll · Ctrl+C quit",
        );
    }
    if name == "model" && !args.trim().is_empty() {
        app.model = args.trim().to_owned();
    }
    app.transcript.push(Kind::Command(markdown));
    app.scroll_up = 0;
    Ok(())
}
