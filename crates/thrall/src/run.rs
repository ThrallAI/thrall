//! `thrall run` (and `thrall -p`): one message, one streamed reply, printed as it arrives. It's a
//! client of the one engine, like the terminal app: the turn runs in the engine, so every other
//! client sees it live. Tool steps show on stderr; when Thrall wants permission it asks on the
//! terminal, or says no when there's nobody to ask.

use std::io::{IsTerminal, Read, Write};
use std::process::ExitCode;

use anyhow::bail;
use thrall_core::{Config, Paths};
use thrall_gates::Mode;
use thrall_protocol::{
    ClientKind, Event, EventFrame, PermissionChoice, Reply, Request, Role, SessionInfo, StepStatus,
    TurnStatus, Usage,
};
use thrall_tui::client::{Client, Connected, connect_as};

use crate::attach;
use crate::cli::RunArgs;
use crate::constants::{
    ANSI_DIM, ANSI_RESET, EMPTY_PROMPT_HINT, EMPTY_PROMPT_IN_SESSION_HINT, EXIT_FAILURE,
    EXIT_SETUP_NEEDED, HEADLESS_CLIENT_NAME, NOTHING_TO_CONTINUE,
};

/// Run one turn from the command line.
pub async fn execute(paths: &Paths, config: Config, args: RunArgs) -> anyhow::Result<ExitCode> {
    let prompt = prompt_text(&args)?;
    let Some(found) = attach::engine(paths, &config).await? else {
        return Ok(ExitCode::from(EXIT_SETUP_NEEDED));
    };
    let Connected {
        client, mut events, ..
    } = connect_as(
        &found.ws_url,
        &attach::token(paths)?,
        ClientKind::Headless,
        HEADLESS_CLIENT_NAME,
    )
    .await?;

    let mode = match &args.mode {
        Some(name) => Some(Mode::parse(name).ok_or_else(|| {
            anyhow::anyhow!("There's no `{name}` mode. Use ask, auto, plan or full.")
        })?),
        None => None,
    };
    let session = open_session(&client, &args, mode).await?;
    if let Some(mode) = mode {
        client
            .request(Request::SessionSetMode {
                session: session.session.clone(),
                mode: mode.as_str().to_owned(),
                until: None,
                minutes: None,
            })
            .await?;
    }
    client
        .request(Request::TurnSend {
            session: session.session.clone(),
            text: prompt,
            files: Vec::new(),
        })
        .await?;

    // Questions can only be asked when a person is at the keyboard and the message didn't come
    // through stdin.
    let can_ask = std::io::stdin().is_terminal() && !args.prompt.is_empty() && !args.json;
    let mut printer = Printer::new(args.json);
    let mut outcome = None;
    while let Some(frame) = events.recv().await {
        // Opening an existing session replays its history first: skip what was already there.
        if frame.session != session.session || frame.seq.is_some_and(|s| s <= session.last_seq) {
            continue;
        }
        if let Some(done) = printer.on_event(&client, frame, can_ask).await? {
            outcome = Some(done);
            break;
        }
    }
    let Some((status, usage)) = outcome else {
        bail!("Lost the connection to Thrall's engine before the reply finished.");
    };
    printer.finish()?;

    if args.json {
        let steps: Vec<serde_json::Value> = printer
            .steps
            .iter()
            .map(|s| {
                serde_json::json!({
                    "tool": s.tool, "title": s.title, "status": s.status, "summary": s.summary
                })
            })
            .collect();
        let out = serde_json::json!({
            "session": session.session,
            "status": status,
            "reply": printer.reply,
            "error": printer.error,
            "steps": steps,
            "usage": usage,
        });
        println!("{out}");
    } else {
        if let Some(message) = &printer.error {
            eprintln!("{message}");
        }
        // anstream turns the dim codes on in Windows consoles and strips them when stderr isn't
        // a terminal, so no raw `[2m` ever shows.
        anstream::eprintln!(
            "{ANSI_DIM}{}{ANSI_RESET}",
            shown(&footer(&session.session, &usage))
        );
    }
    Ok(match status {
        TurnStatus::Done => ExitCode::SUCCESS,
        _ => ExitCode::from(EXIT_FAILURE),
    })
}

/// The session to send to: the one named, the latest, or a new one in this folder.
async fn open_session(
    client: &Client,
    args: &RunArgs,
    mode: Option<Mode>,
) -> anyhow::Result<SessionInfo> {
    let existing = match (&args.session, args.continue_last) {
        (Some(id), _) => Some(id.clone()),
        (None, true) => {
            let latest = match client
                .request(Request::SessionList {
                    limit: Some(1),
                    archived: false,
                })
                .await?
            {
                Reply::SessionListed { items } => items.into_iter().next().map(|s| s.session),
                _ => None,
            };
            if latest.is_none() {
                eprintln!("{NOTHING_TO_CONTINUE}");
            }
            latest
        }
        (None, false) => None,
    };
    let reply = match existing {
        Some(session) => {
            client
                .request(Request::SessionOpen {
                    session,
                    after_seq: None,
                })
                .await?
        }
        None => {
            client
                .request(Request::SessionCreate {
                    workspace: current_dir(),
                    title: None,
                    mode: mode.map(|m| m.as_str().to_owned()),
                })
                .await?
        }
    };
    match reply {
        Reply::SessionOpened(info) => Ok(info),
        other => bail!("Thrall's engine sent {other:?} instead of a session."),
    }
}

/// The message from the arguments, or from piped input when there are none.
fn prompt_text(args: &RunArgs) -> anyhow::Result<String> {
    let mut text = args.prompt.join(" ");
    if text.trim().is_empty() && !std::io::stdin().is_terminal() {
        std::io::stdin().read_to_string(&mut text)?;
    }
    let text = text.trim().to_owned();
    if text.is_empty() {
        if args.continue_last || args.session.is_some() {
            bail!(EMPTY_PROMPT_IN_SESSION_HINT);
        }
        bail!(EMPTY_PROMPT_HINT);
    }
    Ok(text)
}

fn current_dir() -> Option<String> {
    std::env::current_dir()
        .ok()
        .map(|p| p.to_string_lossy().into_owned())
}

/// One tool step, for JSON output.
struct StepSeen {
    id: String,
    tool: String,
    title: String,
    status: Option<StepStatus>,
    summary: String,
}

/// Prints Thrall's reply as it streams in; steps and questions go to stderr. In JSON mode
/// nothing is printed: steps and the reply are only collected.
struct Printer {
    quiet: bool,
    reply_id: Option<String>,
    streamed_any: bool,
    last_char: char,
    /// Thrall's final reply in this turn.
    reply: String,
    /// The user-facing error, if the turn failed.
    error: Option<String>,
    /// Every tool step, in order.
    steps: Vec<StepSeen>,
}

impl Printer {
    fn new(quiet: bool) -> Self {
        Self {
            quiet,
            reply_id: None,
            streamed_any: false,
            last_char: '\n',
            reply: String::new(),
            error: None,
            steps: Vec::new(),
        }
    }

    /// Handle one event. Returns how the turn ended once it has.
    async fn on_event(
        &mut self,
        client: &Client,
        frame: EventFrame,
        can_ask: bool,
    ) -> anyhow::Result<Option<(TurnStatus, Usage)>> {
        match frame.event {
            Event::TurnCompleted { status, usage, .. } => return Ok(Some((status, usage))),
            Event::MessageStarted {
                message,
                role: Role::Assistant,
                ..
            } => {
                self.reply_id = Some(message);
                self.streamed_any = false;
            }
            Event::MessageDelta { message, text } if self.reply_id.as_deref() == Some(&message) => {
                if !self.quiet {
                    let mut out = std::io::stdout().lock();
                    write!(out, "{text}")?;
                    out.flush()?;
                }
                self.streamed_any = true;
                self.last_char = text.chars().last().unwrap_or(self.last_char);
            }
            Event::MessageCompleted { message, text, .. }
                if self.reply_id.as_deref() == Some(&message) =>
            {
                if !self.quiet && !self.streamed_any && !text.is_empty() {
                    print!("{text}");
                    self.last_char = text.chars().last().unwrap_or(self.last_char);
                }
                if !text.is_empty() {
                    self.reply = text;
                }
            }
            Event::StepStarted {
                step, tool, title, ..
            } => {
                if !self.quiet {
                    if self.last_char != '\n' {
                        println!();
                        self.last_char = '\n';
                    }
                    anstream::eprintln!("{ANSI_DIM}{}{ANSI_RESET}", shown(&format!("  ○ {title}")));
                }
                self.steps.push(StepSeen {
                    id: step,
                    tool,
                    title,
                    status: None,
                    summary: String::new(),
                });
            }
            Event::StepCompleted {
                step,
                status,
                summary,
                ..
            } => {
                if !self.quiet {
                    let mark = match status {
                        StepStatus::Ok => "✓",
                        StepStatus::Error => "✗",
                        StepStatus::Warn => "!",
                        StepStatus::Denied | StepStatus::Cancelled => "⊘",
                    };
                    anstream::eprintln!(
                        "{ANSI_DIM}{}{ANSI_RESET}",
                        shown(&format!("    {mark} {summary}"))
                    );
                }
                if let Some(s) = self.steps.iter_mut().find(|s| s.id == step) {
                    s.status = Some(status);
                    s.summary = summary;
                }
            }
            Event::PermissionRequest {
                request, summary, ..
            } => {
                let choice = if can_ask {
                    ask(&summary).await
                } else {
                    if !self.quiet {
                        eprintln!("  Thrall wanted to: {summary} (no terminal to ask on, so: no)");
                    }
                    PermissionChoice::No
                };
                // Another device may have answered first; that's fine.
                let _ = client
                    .request(Request::PermissionAnswer { request, choice })
                    .await;
            }
            Event::PlanProposed { plan, markdown } => {
                let approve = if can_ask {
                    eprintln!("\n{markdown}\n");
                    ask("approve this plan").await == PermissionChoice::Yes
                } else {
                    if !self.quiet {
                        eprintln!(
                            "  Thrall proposed a plan (no terminal to approve it on, so: no)"
                        );
                    }
                    false
                };
                let _ = client
                    .request(Request::PlanDecide {
                        plan,
                        approve,
                        auto_edits: false,
                    })
                    .await;
            }
            // Thrall's recommended answers: a script has nobody to ask (a person at a terminal
            // can still answer from the desktop app or phone, which beats this).
            Event::QuestionAsk {
                question, items, ..
            } => {
                if !self.quiet {
                    for q in &items {
                        eprintln!("  Thrall asked: {} (going with: {})", q.text, q.recommended);
                    }
                }
                let answers = items.into_iter().map(|q| q.recommended).collect();
                let _ = client
                    .request(Request::QuestionAnswer { question, answers })
                    .await;
            }
            Event::Error { message, .. } => self.error = Some(message),
            _ => {}
        }
        Ok(None)
    }

    fn finish(&mut self) -> anyhow::Result<()> {
        if !self.quiet && self.last_char != '\n' {
            println!();
        }
        std::io::stdout().flush()?;
        Ok(())
    }
}

/// Ask yes/no on the terminal.
async fn ask(summary: &str) -> PermissionChoice {
    let question = format!("  Thrall wants to: {summary}\n  Allow? [y/N] ");
    let answer = tokio::task::spawn_blocking(move || {
        eprint!("{question}");
        let _ = std::io::stderr().flush();
        let mut line = String::new();
        let _ = std::io::stdin().read_line(&mut line);
        line
    })
    .await
    .unwrap_or_default();
    if matches!(answer.trim().to_lowercase().as_str(), "y" | "yes") {
        PermissionChoice::Yes
    } else {
        PermissionChoice::No
    }
}

/// Text for stderr: symbols become ASCII on consoles that can't show them.
fn shown(text: &str) -> String {
    if thrall_tui::plain::wanted() {
        thrall_tui::plain::text(text).into_owned()
    } else {
        text.to_owned()
    }
}

/// `s_… · 1840 in · 96 out · $0.0031`.
fn footer(session_id: &str, usage: &Usage) -> String {
    let mut line = format!(
        "{session_id} · {} in · {} out",
        usage.input_tokens, usage.output_tokens
    );
    if let Some(cost) = usage.cost_usd {
        line.push_str(&format!(" · ${cost:.4}"));
    }
    line
}
