//! The terminal app's state, what keys and events do to it, and how it's drawn. Async work (turns,
//! commands, loading sessions) is returned as an `Action` for the run loop to carry out.

use std::collections::VecDeque;
use std::time::Instant;

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap,
};
use ratatui_textarea::TextArea;
use thrall_core::ids::now_ms;
use thrall_gates::Mode;
use thrall_protocol::constants::CHECK_IN_AGENT;
use thrall_protocol::{
    CommandItem, Diff, Event, NoticeLevel, PermissionChoice, Risk, Role, SessionInfo as Session,
    SkillDetail, TaskItem, TaskStatus,
};

use crate::constants::{
    CARD_SETTLE, CARD_TYPING_GAP, CLIENT_COMMANDS, HELPER_STEP_MARK, HINTS, INPUT_MAX_LINES,
    INPUT_PLACEHOLDER, PAGE_SCROLL_LINES, PASTE_ENTER_WINDOW, PERMISSION_DIFF_MAX_LINES,
    PERMISSION_SUMMARY_MAX_LINES, QUESTION_TIMEOUT, QUESTION_TYPE_HINT, SUGGESTIONS_MAX,
    TASKS_MAX_SHOWN,
};
use crate::transcript::{Kind, Transcript};
use crate::{plain, theme};

/// Async work for the run loop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Send a message to Thrall.
    Send(String),
    /// Fetch the slash commands again (a turn may have saved a skill).
    RefreshCommands,
    /// Run an engine command.
    Command {
        /// Name without the slash.
        name: String,
        /// Everything after the name.
        args: String,
    },
    /// Start a new session.
    NewSession,
    /// Show the session picker.
    OpenPicker,
    /// Load a session.
    Resume(String),
    /// Load Home's current page.
    Home,
    /// Answer a permission request or a plan.
    Answer(String, Reply),
    /// Change the permission mode.
    SetMode(Mode),
    /// Stop the running turn (Esc).
    Interrupt,
    /// Leave.
    Quit,
}

/// An answer to a card.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    /// A permission answer.
    Permission(PermissionChoice),
    /// A plan decision.
    Plan {
        /// Go ahead with the plan.
        approve: bool,
        /// And accept edits without asking from now on.
        auto_edits: bool,
    },
    /// Answers to a question card, one per question.
    Question(Vec<String>),
}

/// A permission request or plan on screen, waiting for an answer.
pub struct Permission {
    /// Request (or plan) id.
    pub request: String,
    /// What Thrall wants to do.
    pub summary: String,
    /// How risky.
    pub risk: Risk,
    /// The change, for edits.
    pub diff: Option<Diff>,
    /// Whether "always" is on offer.
    pub always: bool,
    /// A plan to approve rather than a single action.
    pub plan: bool,
    /// A question card: Thrall's recommended answers, one per question.
    pub picks: Option<Vec<String>>,
}

/// The session picker.
pub struct Picker {
    /// Sessions, newest first.
    pub sessions: Vec<Session>,
    /// Highlighted row.
    pub selected: usize,
}

/// Everything on screen.
pub struct App {
    /// Current session (created on the first message).
    pub session: Option<Session>,
    /// Session title shown in the header.
    pub title: String,
    /// Model shown in the header.
    pub model: String,
    /// Money spent in this session.
    pub cost: f64,
    /// The conversation.
    pub transcript: Transcript,
    /// The task list.
    pub tasks: Vec<TaskItem>,
    /// Waiting for a yes or no.
    pub permission: Option<Permission>,
    /// A question card being answered by typing: the next message is the answers.
    pub answering: Option<String>,
    /// Choosing a session.
    pub picker: Option<Picker>,
    /// When the running turn began.
    pub busy_since: Option<Instant>,
    /// The permission mode.
    pub mode: Mode,
    /// Messages typed while Thrall was working, sent in order afterwards.
    pub queue: VecDeque<String>,
    /// Rows scrolled up from the bottom (0 = following the conversation).
    pub scroll_up: usize,
    /// The engine's slash commands, for suggestions.
    pub commands: Vec<CommandItem>,
    last_total: usize,
    input: TextArea<'static>,
    history: Vec<String>,
    history_pos: Option<usize>,
    suggestion: usize,
    last_key: Instant,
    quit_armed: bool,
    spinner: usize,
    user_message: Option<String>,
    /// Draw with ASCII only (consoles that can't show symbols).
    ascii: bool,
    /// When the current card appeared.
    card_shown_at: Instant,
    /// The session's turns: id, where its entries start, and the message that started it, so a
    /// retry, edit or rewind can take turns back.
    pub turns: Vec<(String, usize, String)>,
    /// A safer version of a skill `/skill <name> safe` showed, kept until it's saved.
    pub safer: Option<SkillDetail>,
    /// Helpers' step ids in the order they started, so each helper's steps share a number.
    helpers: Vec<String>,
}

fn new_input() -> TextArea<'static> {
    let mut input = TextArea::default();
    input.set_placeholder_text(INPUT_PLACEHOLDER);
    input.set_placeholder_style(theme::dim());
    input.set_cursor_line_style(Style::default());
    input
}

impl App {
    /// A fresh screen.
    pub fn new(model: String, ascii: bool) -> Self {
        Self {
            session: None,
            title: String::new(),
            model,
            cost: 0.0,
            transcript: Transcript::new(ascii),
            tasks: Vec::new(),
            permission: None,
            answering: None,
            picker: None,
            busy_since: None,
            mode: Mode::Ask,
            queue: VecDeque::new(),
            scroll_up: 0,
            commands: Vec::new(),
            last_total: 0,
            input: new_input(),
            history: Vec::new(),
            history_pos: None,
            suggestion: 0,
            last_key: Instant::now(),
            quit_armed: false,
            spinner: 0,
            user_message: None,
            ascii,
            card_shown_at: Instant::now(),
            turns: Vec::new(),
            safer: None,
            helpers: Vec::new(),
        }
    }

    /// Forget the current conversation (new session or resume).
    pub fn reset(&mut self) {
        self.session = None;
        self.title.clear();
        self.cost = 0.0;
        self.transcript.clear();
        self.tasks.clear();
        self.permission = None;
        self.answering = None;
        self.scroll_up = 0;
        self.last_total = 0;
        self.turns.clear();
        self.helpers.clear();
    }

    /// Which helper of this session a step belongs to, counting from 1 in the order they started.
    fn helper_number(&mut self, helper: String) -> usize {
        match self.helpers.iter().position(|h| *h == helper) {
            Some(i) => i + 1,
            None => {
                self.helpers.push(helper);
                self.helpers.len()
            }
        }
    }

    /// Show a line of information.
    pub fn info(&mut self, text: impl Into<String>) {
        self.transcript.push(Kind::Notice {
            warning: false,
            text: text.into(),
        });
        self.scroll_up = 0;
    }

    /// Show an error.
    pub fn error(&mut self, text: impl Into<String>) {
        self.transcript.push(Kind::Error(text.into()));
        self.scroll_up = 0;
    }

    /// Advance the spinner.
    pub fn tick(&mut self) {
        self.spinner = (self.spinner + 1) % plain::spinner(self.ascii).len();
    }

    // ---------- protocol events ----------

    /// Apply one event (live or replayed).
    pub fn on_event(&mut self, event: Event) {
        match event {
            Event::TurnStarted { turn, .. } => {
                self.turns
                    .push((turn, self.transcript.len(), String::new()));
            }
            // The turn goes, with every later one.
            Event::TurnRetracted { turn } => {
                if let Some(i) = self.turns.iter().position(|(t, _, _)| *t == turn) {
                    self.transcript.truncate(self.turns[i].1);
                    self.turns.truncate(i);
                    self.scroll_up = 0;
                }
            }
            Event::Status { model, .. } => self.model = model,
            Event::MessageStarted {
                role: Role::User, ..
            } => self.user_message = Some(String::new()),
            Event::MessageStarted { message, .. } => self.transcript.push(Kind::Assistant {
                id: message,
                text: String::new(),
                thinking: false,
                streaming: true,
            }),
            Event::MessageDelta { message, text } => {
                self.transcript.update(
                    |k| matches!(k, Kind::Assistant { id, .. } if *id == message),
                    |k| {
                        if let Kind::Assistant { text: t, .. } = k {
                            t.push_str(&text);
                        }
                    },
                );
            }
            Event::ReasoningDelta { message, .. } => {
                self.transcript.update(
                    |k| matches!(k, Kind::Assistant { id, thinking: false, .. } if *id == message),
                    |k| {
                        if let Kind::Assistant { thinking, .. } = k {
                            *thinking = true;
                        }
                    },
                );
            }
            Event::MessageCompleted {
                message,
                text,
                files,
            } => {
                if self.user_message.take().is_some() {
                    if let Some((_, _, first)) = self.turns.last_mut()
                        && first.is_empty()
                    {
                        first.clone_from(&text);
                    }
                    let mut text = text;
                    if !files.is_empty() {
                        let names: Vec<&str> = files.iter().map(|f| f.name.as_str()).collect();
                        text.push_str(&format!("\n\nAttached: {}", names.join(", ")));
                    }
                    self.transcript.push(Kind::User(text));
                    self.scroll_up = 0;
                    return;
                }
                self.transcript.update(
                    |k| matches!(k, Kind::Assistant { id, .. } if *id == message),
                    |k| {
                        if let Kind::Assistant {
                            text: t, streaming, ..
                        } = k
                        {
                            *t = text;
                            *streaming = false;
                        }
                    },
                );
            }
            Event::StepStarted {
                step,
                title,
                helper,
                ..
            } => {
                // A helper's steps are marked as its own, numbered by helper, since several can
                // work side by side and their steps interleave.
                let title = match helper {
                    Some(h) => format!("{HELPER_STEP_MARK}{} · {title}", self.helper_number(h)),
                    None => title,
                };
                self.transcript.push(Kind::Step {
                    id: step,
                    title,
                    status: None,
                    summary: String::new(),
                    output: String::new(),
                    diff: None,
                });
            }
            Event::StepOutput { step, text } => {
                self.transcript.update(
                    |k| matches!(k, Kind::Step { id, .. } if *id == step),
                    |k| {
                        if let Kind::Step { output, .. } = k {
                            output.push_str(&text);
                        }
                    },
                );
            }
            Event::StepCompleted {
                step,
                status,
                summary,
                diff,
                files,
            } => {
                // The terminal can't show pictures: name what the step made.
                let mut summary = summary;
                if !files.is_empty() {
                    let names: Vec<&str> = files.iter().map(|f| f.name.as_str()).collect();
                    summary.push_str(&format!(" · made {}", names.join(", ")));
                }
                self.transcript.update(
                    |k| matches!(k, Kind::Step { id, .. } if *id == step),
                    |k| {
                        if let Kind::Step {
                            status: s,
                            summary: m,
                            diff: d,
                            ..
                        } = k
                        {
                            *s = Some(status);
                            *m = summary;
                            *d = diff;
                        }
                    },
                );
            }
            Event::TasksUpdated { items } => self.tasks = items,
            Event::PermissionRequest {
                request,
                summary,
                risk,
                diff,
                choices,
                ..
            } => {
                self.card_shown_at = Instant::now();
                self.permission = Some(Permission {
                    request,
                    summary,
                    risk,
                    diff,
                    always: choices.contains(&PermissionChoice::Always),
                    plan: false,
                    picks: None,
                });
            }
            Event::PlanProposed { plan, markdown } => {
                self.transcript
                    .push(Kind::Command(format!("**Plan**\n\n{markdown}")));
                self.scroll_up = 0;
                self.card_shown_at = Instant::now();
                self.permission = Some(Permission {
                    request: plan,
                    summary: "Approve this plan?".to_owned(),
                    risk: Risk::Low,
                    diff: None,
                    always: false,
                    plan: true,
                    picks: None,
                });
            }
            Event::QuestionAsk {
                question,
                items,
                why,
                ..
            } => {
                let mut text = String::from("**Thrall asks**");
                if !why.is_empty() {
                    text.push_str(&format!("\n\n{why}"));
                }
                for (i, q) in items.iter().enumerate() {
                    text.push_str(&format!("\n\n{}. {}", i + 1, q.text));
                    if !q.options.is_empty() {
                        text.push_str(&format!("\n   Choices: {}", q.options.join(" · ")));
                    }
                    text.push_str(&format!("\n   Thrall's pick: {}", q.recommended));
                }
                self.transcript.push(Kind::Command(text));
                self.scroll_up = 0;
                self.card_shown_at = Instant::now();
                self.permission = Some(Permission {
                    request: question,
                    summary: "Go with Thrall's picks, or type your own answers?".to_owned(),
                    risk: Risk::Low,
                    diff: None,
                    always: false,
                    plan: false,
                    picks: Some(items.into_iter().map(|q| q.recommended).collect()),
                });
            }
            Event::QuestionResolved {
                question,
                answers,
                answered_by,
            } => {
                if self
                    .permission
                    .as_ref()
                    .is_some_and(|p| p.request == question)
                {
                    self.permission = None;
                }
                if self.answering.as_deref() == Some(question.as_str()) {
                    self.answering = None;
                }
                let how = if answered_by == QUESTION_TIMEOUT {
                    "No answer in time, so Thrall went with its picks"
                } else {
                    "Answered"
                };
                self.info(format!("{how}: {}", answers.join(" · ")));
            }
            Event::PermissionResolved { request, .. }
            | Event::PlanResolved { plan: request, .. } => {
                if self
                    .permission
                    .as_ref()
                    .is_some_and(|p| p.request == request)
                {
                    self.permission = None;
                }
            }
            Event::SessionUpdated { title, mode, .. } => {
                if let Some(title) = title {
                    self.title = title.clone();
                    if let Some(s) = &mut self.session {
                        s.title = title;
                    }
                }
                if let Some(mode) = mode.as_deref().and_then(Mode::parse) {
                    self.mode = mode;
                }
            }
            Event::Notice { level, text } => self.transcript.push(Kind::Notice {
                warning: level == NoticeLevel::Warning,
                text,
            }),
            Event::MemorySaved {
                fact,
                topic,
                content,
            } => self.transcript.push(Kind::Notice {
                warning: false,
                text: format!("Remembered “{topic}”: {content}  (/forget {fact} to forget it)"),
            }),
            Event::AgentStarted { title, .. } => self.transcript.push(Kind::Notice {
                warning: false,
                text: format!("Background task started: {title}"),
            }),
            // A check-in's message is already here.
            Event::AgentCompleted { agent, .. } if agent == CHECK_IN_AGENT => {}
            // Only the desktop app installs updates.
            Event::UpdateRequested {} => {}
            // PCs asking to join are answered in the desktop app's Settings.
            Event::PcAsked {} => {}
            // Only a companion PC's runner gets these.
            Event::RunnerCall { .. } => {}
            Event::AgentCompleted {
                title,
                status,
                summary,
                ..
            } => self.transcript.push(Kind::Notice {
                warning: status != "done",
                text: format!("Background task {status}: {title} · {summary}"),
            }),
            Event::Error { message, .. } => self.transcript.push(Kind::Error(message)),
            Event::TurnCompleted { usage, .. } => {
                self.cost += usage.cost_usd.unwrap_or(0.0);
                self.transcript.push(Kind::TurnEnd(usage));
            }
        }
    }

    // ---------- keys ----------

    /// A key press. Returns async work for the run loop, if any.
    pub fn on_key(&mut self, key: KeyEvent) -> Option<Action> {
        let since_last = self.last_key.elapsed();
        let pasting = since_last < PASTE_ENTER_WINDOW;
        self.last_key = Instant::now();
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

        if ctrl && key.code == KeyCode::Char('c') {
            return self.on_ctrl_c();
        }
        self.quit_armed = false;

        if let Some(p) = &self.permission {
            // A card only takes a deliberate key press: keys typed in a burst (someone typing a
            // message) or just as it appeared are ignored, so "Start-Sleep" can't answer it.
            let typing = since_last < CARD_TYPING_GAP;
            let just_shown = self.card_shown_at.elapsed() < CARD_SETTLE;
            if (typing || just_shown) && key.code != KeyCode::Esc {
                return None;
            }
            let request = p.request.clone();
            if let Some(picks) = &p.picks {
                return match key.code {
                    KeyCode::Char('y' | 'Y') | KeyCode::Enter => {
                        Some(Action::Answer(request, Reply::Question(picks.clone())))
                    }
                    // Type the answers into the message box; the next Enter sends them.
                    KeyCode::Char('t' | 'T') => {
                        self.permission = None;
                        self.answering = Some(request);
                        self.info(QUESTION_TYPE_HINT);
                        None
                    }
                    _ => None,
                };
            }
            let reply = if p.plan {
                // Approving goes on in Auto edits; a keeps asking before each change.
                match key.code {
                    KeyCode::Char('y' | 'Y') => Some(Reply::Plan {
                        approve: true,
                        auto_edits: true,
                    }),
                    KeyCode::Char('a' | 'A') => Some(Reply::Plan {
                        approve: true,
                        auto_edits: false,
                    }),
                    KeyCode::Char('n' | 'N') | KeyCode::Esc => Some(Reply::Plan {
                        approve: false,
                        auto_edits: false,
                    }),
                    _ => None,
                }
            } else {
                match key.code {
                    KeyCode::Char('y' | 'Y') => Some(Reply::Permission(PermissionChoice::Yes)),
                    KeyCode::Char('a' | 'A') if p.always => {
                        Some(Reply::Permission(PermissionChoice::Always))
                    }
                    KeyCode::Char('n' | 'N') | KeyCode::Esc => {
                        Some(Reply::Permission(PermissionChoice::No))
                    }
                    _ => None,
                }
            };
            return reply.map(|r| Action::Answer(request, r));
        }
        if self.picker.is_some() {
            return self.on_picker_key(key);
        }

        match key.code {
            KeyCode::Char('o') if ctrl => {
                self.transcript.expanded = !self.transcript.expanded;
                None
            }
            KeyCode::Char('d') if ctrl && self.input_text().is_empty() => Some(Action::Quit),
            KeyCode::Char('j') if ctrl => {
                self.input.insert_newline();
                None
            }
            KeyCode::Enter
                if pasting
                    || key
                        .modifiers
                        .intersects(KeyModifiers::SHIFT | KeyModifiers::ALT) =>
            {
                self.input.insert_newline();
                None
            }
            KeyCode::Enter => self.submit(),
            KeyCode::Tab => {
                self.complete();
                None
            }
            KeyCode::Up if !self.suggestions().is_empty() => {
                self.suggestion = self.suggestion.saturating_sub(1);
                None
            }
            KeyCode::Down if !self.suggestions().is_empty() => {
                let n = self.suggestions().len();
                self.suggestion = (self.suggestion + 1).min(n.saturating_sub(1));
                None
            }
            KeyCode::Up if self.input.lines().len() <= 1 && !self.history.is_empty() => {
                let pos = match self.history_pos {
                    Some(p) => p.saturating_sub(1),
                    None => self.history.len() - 1,
                };
                self.history_pos = Some(pos);
                self.set_input(&self.history[pos].clone());
                None
            }
            KeyCode::Down if self.history_pos.is_some() && self.input.lines().len() <= 1 => {
                let next = self
                    .history_pos
                    .map(|p| p + 1)
                    .filter(|p| *p < self.history.len());
                self.history_pos = next;
                let text = next.map(|p| self.history[p].clone()).unwrap_or_default();
                self.set_input(&text);
                None
            }
            KeyCode::PageUp => {
                self.scroll_up += PAGE_SCROLL_LINES;
                None
            }
            KeyCode::PageDown => {
                self.scroll_up = self.scroll_up.saturating_sub(PAGE_SCROLL_LINES);
                None
            }
            KeyCode::Up if key.modifiers.contains(KeyModifiers::SHIFT) => {
                self.scroll_up += 1;
                None
            }
            KeyCode::Down if key.modifiers.contains(KeyModifiers::SHIFT) => {
                self.scroll_up = self.scroll_up.saturating_sub(1);
                None
            }
            KeyCode::End if ctrl => {
                self.scroll_up = 0;
                None
            }
            KeyCode::BackTab => Some(Action::SetMode(self.mode.next())),
            KeyCode::Esc if self.busy_since.is_some() && self.suggestions().is_empty() => {
                Some(Action::Interrupt)
            }
            KeyCode::Esc => {
                self.suggestion = 0;
                None
            }
            _ => {
                self.input.input(key);
                self.suggestion = 0;
                None
            }
        }
    }

    /// Pasted text (terminals with bracketed paste).
    pub fn on_paste(&mut self, text: &str) {
        if self.permission.is_none() && self.picker.is_none() {
            self.input
                .insert_str(text.replace("\r\n", "\n").replace('\r', "\n"));
        }
    }

    fn on_ctrl_c(&mut self) -> Option<Action> {
        if !self.input_text().is_empty() {
            self.input = new_input();
            return None;
        }
        if self.picker.take().is_some() {
            return None;
        }
        if self.busy_since.is_some() && !self.quit_armed {
            self.quit_armed = true;
            self.info("Thrall is still working. Press Ctrl+C again to quit anyway.");
            return None;
        }
        Some(Action::Quit)
    }

    fn on_picker_key(&mut self, key: KeyEvent) -> Option<Action> {
        let picker = self.picker.as_mut()?;
        match key.code {
            KeyCode::Up => picker.selected = picker.selected.saturating_sub(1),
            KeyCode::Down => {
                picker.selected =
                    (picker.selected + 1).min(picker.sessions.len().saturating_sub(1));
            }
            KeyCode::Enter => {
                let id = picker
                    .sessions
                    .get(picker.selected)
                    .map(|s| s.session.clone());
                self.picker = None;
                return id.map(Action::Resume);
            }
            KeyCode::Esc => self.picker = None,
            _ => {}
        }
        None
    }

    fn input_text(&self) -> String {
        self.input.lines().join("\n")
    }

    pub fn set_input(&mut self, text: &str) {
        self.input = new_input();
        self.input.insert_str(text);
    }

    fn submit(&mut self) -> Option<Action> {
        if let Some(name) = self
            .suggestions()
            .get(self.suggestion)
            .map(|(n, _)| n.clone())
        {
            // Enter on a highlighted suggestion that isn't typed out yet completes it first.
            let typed = self.input_text();
            if typed.trim() != format!("/{name}") && name.starts_with(typed.trim_start_matches('/'))
            {
                self.set_input(&format!("/{name}"));
            }
        }
        let text = self.input_text().trim().to_owned();
        if text.is_empty() {
            return None;
        }
        self.input = new_input();
        self.history.push(text.clone());
        self.history_pos = None;
        self.suggestion = 0;
        self.scroll_up = 0;

        // Answers to a question card, one per question, separated by |.
        if let Some(question) = self.answering.take() {
            let answers = text.split('|').map(|a| a.trim().to_owned()).collect();
            return Some(Action::Answer(question, Reply::Question(answers)));
        }
        if let Some(command) = text.strip_prefix('/') {
            let (name, args) = command.split_once(' ').unwrap_or((command, ""));
            let name = name.to_lowercase();
            // A skill runs as a message: Thrall sees `/name …` and uses the skill.
            if self
                .commands
                .iter()
                .any(|c| c.skill && c.name.eq_ignore_ascii_case(&name))
            {
                return Some(Action::Send(text));
            }
            return match name.as_str() {
                "quit" | "exit" => Some(Action::Quit),
                "new" | "clear" => Some(Action::NewSession),
                "home" => Some(Action::Home),
                "resume" | "sessions" if args.trim().is_empty() => Some(Action::OpenPicker),
                "resume" | "sessions" => Some(Action::Resume(args.trim().to_owned())),
                "expand" => {
                    self.transcript.expanded = !self.transcript.expanded;
                    None
                }
                _ => Some(Action::Command {
                    name,
                    args: args.to_owned(),
                }),
            };
        }
        // While Thrall works, the run loop hands this to the running turn.
        Some(Action::Send(text))
    }

    /// Commands matching what's typed after `/`.
    fn suggestions(&self) -> Vec<(String, String)> {
        let lines = self.input.lines();
        let Some(typed) = lines.first().and_then(|l| l.strip_prefix('/')) else {
            return Vec::new();
        };
        if lines.len() > 1 || typed.contains(' ') {
            return Vec::new();
        }
        let typed = typed.to_lowercase();
        let mut all: Vec<(String, String)> = self
            .commands
            .iter()
            .map(|c| (c.name.clone(), c.summary.clone()))
            .chain(
                CLIENT_COMMANDS
                    .iter()
                    .map(|(n, s)| ((*n).to_owned(), (*s).to_owned())),
            )
            .filter(|(name, _)| name.starts_with(typed.as_str()))
            .collect();
        all.sort();
        all.truncate(SUGGESTIONS_MAX);
        all
    }

    fn complete(&mut self) {
        if let Some((name, _)) = self.suggestions().get(self.suggestion).cloned() {
            self.set_input(&format!("/{name} "));
            self.suggestion = 0;
        }
    }

    // ---------- drawing ----------

    /// Draw everything.
    pub fn draw(&mut self, frame: &mut Frame<'_>) {
        let area = frame.area();
        let input_lines = u16::try_from(self.input.lines().len())
            .unwrap_or(INPUT_MAX_LINES)
            .clamp(1, INPUT_MAX_LINES);
        let bottom_height = match &self.permission {
            Some(p) => {
                // Room for the whole summary (up to a limit) inside the borders.
                let width = usize::from(area.width.saturating_sub(4)).max(10);
                let rows = p
                    .summary
                    .lines()
                    .map(|l| l.chars().count().div_ceil(width).max(1))
                    .sum::<usize>();
                let diff_rows = p.diff.as_ref().map_or(0, |d| {
                    u16::try_from(d.unified.lines().count().min(PERMISSION_DIFF_MAX_LINES))
                        .unwrap_or(0)
                });
                u16::try_from(rows)
                    .unwrap_or(PERMISSION_SUMMARY_MAX_LINES)
                    .clamp(1, PERMISSION_SUMMARY_MAX_LINES)
                    + diff_rows
                    + 2
            }
            None => input_lines + 2,
        };
        let tasks_height = if self.tasks.is_empty() {
            0
        } else {
            u16::try_from(self.tasks.len().min(TASKS_MAX_SHOWN)).unwrap_or(0) + 1
        };
        let [header, body, tasks, status, bottom, hints] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(3),
            Constraint::Length(tasks_height),
            Constraint::Length(1),
            Constraint::Length(bottom_height),
            Constraint::Length(1),
        ])
        .areas(area);

        self.draw_header(frame, header);
        self.draw_transcript(frame, body);
        if tasks_height > 0 {
            self.draw_tasks(frame, tasks);
        }
        self.draw_status(frame, status);
        if let Some(p) = &self.permission {
            draw_permission(frame, bottom, p);
        } else {
            let border = if self.busy_since.is_some() {
                theme::dim()
            } else {
                theme::accent()
            };
            self.input.set_block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(border),
            );
            frame.render_widget(&self.input, bottom);
            self.draw_suggestions(frame, body, bottom);
        }
        frame.render_widget(Paragraph::new(Span::styled(HINTS, theme::dim())), hints);
        if let Some(picker) = &self.picker {
            draw_picker(frame, body, picker);
        }
        if self.ascii {
            plain::buffer(frame.buffer_mut());
        }
    }

    fn draw_header(&self, frame: &mut Frame<'_>, area: Rect) {
        let title = if self.title.is_empty() {
            "New session"
        } else {
            &self.title
        };
        let right = format!("{} · ${:.4} ", self.model, self.cost);
        let [left_area, right_area] = Layout::horizontal([
            Constraint::Min(10),
            Constraint::Length(u16::try_from(right.chars().count()).unwrap_or(40)),
        ])
        .areas(area);
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(" Thrall", theme::strong_accent()),
                Span::styled(" · ", theme::dim()),
                Span::raw(title.to_owned()),
            ])),
            left_area,
        );
        frame.render_widget(
            Paragraph::new(Span::styled(right, theme::dim())),
            right_area,
        );
    }

    fn draw_transcript(&mut self, frame: &mut Frame<'_>, area: Rect) {
        let inner = Rect {
            x: area.x + 1,
            width: area.width.saturating_sub(2),
            ..area
        };
        if self.transcript.is_empty() {
            let welcome = vec![
                Line::from(Span::styled("Thrall", theme::strong_accent())),
                Line::from(Span::styled(
                    "Ask anything, or give it a job in this folder. Type / for commands.",
                    theme::dim(),
                )),
            ];
            frame.render_widget(Paragraph::new(welcome), inner);
            return;
        }
        let total = self.transcript.prepare(inner.width);
        let height = usize::from(inner.height);
        if self.scroll_up > 0 && total > self.last_total {
            // Reading back: keep the view still while new text arrives below.
            self.scroll_up += total - self.last_total;
        }
        self.last_total = total;
        let max_scroll = total.saturating_sub(height);
        self.scroll_up = self.scroll_up.min(max_scroll);
        let top = max_scroll - self.scroll_up;
        let (lines, skip) = self.transcript.visible(top, height);
        frame.render_widget(
            Paragraph::new(lines)
                .wrap(Wrap { trim: false })
                .scroll((skip, 0)),
            inner,
        );
    }

    fn draw_tasks(&self, frame: &mut Frame<'_>, area: Rect) {
        let done = self
            .tasks
            .iter()
            .filter(|t| t.status == TaskStatus::Done)
            .count();
        let mut lines = vec![Line::from(Span::styled(
            format!(" Tasks {done}/{}", self.tasks.len()),
            theme::dim().add_modifier(Modifier::BOLD),
        ))];
        // Keep the task being worked on in view when the list is long.
        let current = self
            .tasks
            .iter()
            .position(|t| t.status == TaskStatus::InProgress)
            .unwrap_or(0);
        let start = current
            .saturating_sub(TASKS_MAX_SHOWN / 2)
            .min(self.tasks.len().saturating_sub(TASKS_MAX_SHOWN));
        for task in self.tasks.iter().skip(start).take(TASKS_MAX_SHOWN) {
            let (mark, style) = match task.status {
                TaskStatus::Done => ("☑", theme::dim().add_modifier(Modifier::CROSSED_OUT)),
                TaskStatus::InProgress => ("◐", theme::strong_accent()),
                TaskStatus::Pending => ("☐", theme::text()),
            };
            lines.push(Line::from(vec![
                Span::styled(format!("   {mark} "), style),
                Span::styled(task.text.clone(), style),
            ]));
        }
        frame.render_widget(Paragraph::new(lines), area);
    }

    fn draw_status(&self, frame: &mut Frame<'_>, area: Rect) {
        let mut spans = Vec::new();
        if let Some(since) = self.busy_since {
            let state = if self.permission.is_some() {
                "Waiting for you"
            } else {
                "Working"
            };
            spans.push(Span::styled(
                format!(
                    " {} {state} · {}s",
                    plain::spinner(self.ascii)[self.spinner % plain::spinner(self.ascii).len()],
                    since.elapsed().as_secs()
                ),
                theme::accent(),
            ));
            if self.permission.is_none() {
                spans.push(Span::styled(" · Esc to stop", theme::dim()));
            }
        } else {
            spans.push(Span::styled(" Ready", theme::dim()));
        }
        if self.scroll_up > 0 {
            spans.push(Span::styled(
                "  ↓ more below (Ctrl+End)",
                Style::default().fg(theme::WARN),
            ));
        }
        let mode_style = match self.mode {
            Mode::Ask => theme::dim(),
            Mode::AutoEdits | Mode::Plan => theme::accent(),
            Mode::FullAuto => Style::default().fg(theme::WARN),
        };
        let mode = format!("{} · Shift+Tab ", self.mode.label());
        let [left, right] = Layout::horizontal([
            Constraint::Min(10),
            Constraint::Length(u16::try_from(mode.chars().count()).unwrap_or(30)),
        ])
        .areas(area);
        frame.render_widget(Paragraph::new(Line::from(spans)), left);
        frame.render_widget(Paragraph::new(Span::styled(mode, mode_style)), right);
    }

    fn draw_suggestions(&self, frame: &mut Frame<'_>, body: Rect, input: Rect) {
        let items = self.suggestions();
        if items.is_empty() {
            return;
        }
        let height = u16::try_from(items.len()).unwrap_or(0) + 2;
        let area = Rect {
            x: input.x,
            y: input.y.saturating_sub(height).max(body.y),
            width: input.width.min(70),
            height: height.min(body.height),
        };
        let list: Vec<ListItem<'_>> = items
            .iter()
            .map(|(name, summary)| {
                ListItem::new(Line::from(vec![
                    Span::styled(format!("/{name:<10}"), theme::accent()),
                    Span::styled(format!(" {summary}"), theme::dim()),
                ]))
            })
            .collect();
        let mut state = ListState::default().with_selected(Some(self.suggestion));
        frame.render_widget(Clear, area);
        frame.render_stateful_widget(
            List::new(list)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_type(BorderType::Rounded)
                        .border_style(theme::dim()),
                )
                .highlight_style(Style::default().add_modifier(Modifier::REVERSED)),
            area,
            &mut state,
        );
    }
}

fn draw_permission(frame: &mut Frame<'_>, area: Rect, p: &Permission) {
    let colour = match p.risk {
        Risk::Low => theme::OK,
        Risk::Medium => theme::WARN,
        Risk::High => theme::ERROR,
    };
    let mut lines: Vec<Line<'_>> = p
        .summary
        .lines()
        .map(|l| {
            Line::from(Span::styled(
                l.to_owned(),
                Style::default().add_modifier(Modifier::BOLD),
            ))
        })
        .collect();
    // An edit shows exactly what will change before anything is written.
    if let Some(diff) = &p.diff {
        for line in diff.unified.lines().take(PERMISSION_DIFF_MAX_LINES) {
            let style = if line.starts_with('+') && !line.starts_with("+++") {
                Style::default().fg(theme::OK)
            } else if line.starts_with('-') && !line.starts_with("---") {
                Style::default().fg(theme::ERROR)
            } else if line.starts_with("@@") {
                theme::accent()
            } else {
                theme::dim()
            };
            lines.push(Line::from(Span::styled(line.to_owned(), style)));
        }
    }
    let bold = |c| Style::default().fg(c).add_modifier(Modifier::BOLD);
    // The answer keys sit on the border, so a long command can never push them out of sight.
    let mut buttons = vec![Span::raw(" ")];
    if p.picks.is_some() {
        buttons.push(Span::styled("[y] Go with Thrall's picks", bold(theme::OK)));
        buttons.push(Span::raw("   "));
        buttons.push(Span::styled("[t] Type my own answers", bold(theme::ACCENT)));
    } else if p.plan {
        buttons.push(Span::styled("[y] Approve", bold(theme::OK)));
        buttons.push(Span::raw("   "));
        buttons.push(Span::styled(
            "[a] Approve, ask before each change",
            bold(theme::ACCENT),
        ));
        buttons.push(Span::raw("   "));
        buttons.push(Span::styled("[n] Keep planning", bold(theme::ERROR)));
    } else {
        buttons.push(Span::styled("[y] Yes", bold(theme::OK)));
        if p.always {
            buttons.push(Span::raw("   "));
            buttons.push(Span::styled(
                "[a] Always (this session)",
                bold(theme::ACCENT),
            ));
        }
        buttons.push(Span::raw("   "));
        buttons.push(Span::styled("[n] No", bold(theme::ERROR)));
    }
    buttons.push(Span::raw(" "));
    let buttons = Line::from(buttons);
    let title = if p.plan {
        " Thrall's plan: "
    } else {
        " Thrall wants to: "
    };
    frame.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }).block(
            Block::default()
                .title(Span::styled(title, Style::default().fg(colour)))
                .title_bottom(buttons)
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(colour)),
        ),
        area,
    );
}

fn draw_picker(frame: &mut Frame<'_>, body: Rect, picker: &Picker) {
    let area = Rect {
        x: body.x + 2,
        y: body.y + 1,
        width: body.width.saturating_sub(4),
        height: body.height.saturating_sub(2),
    };
    let now = now_ms();
    let items: Vec<ListItem<'_>> = picker
        .sessions
        .iter()
        .map(|s| {
            let title = if s.title.is_empty() {
                "(untitled)"
            } else {
                &s.title
            };
            ListItem::new(Line::from(vec![
                Span::raw(format!("{title:<50}")),
                Span::styled(format!(" {}", ago(now - s.updated)), theme::dim()),
            ]))
        })
        .collect();
    let mut state = ListState::default().with_selected(Some(picker.selected));
    frame.render_widget(Clear, area);
    frame.render_stateful_widget(
        List::new(items)
            .block(
                Block::default()
                    .title(Span::styled(
                        " Resume a session · Enter open · Esc close ",
                        theme::accent(),
                    ))
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(theme::accent()),
            )
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED)),
        area,
        &mut state,
    );
}

/// "5 min ago", "3 h ago", "2 days ago".
fn ago(ms: i64) -> String {
    let mins = ms.max(0) / 60_000;
    match mins {
        0 => "just now".to_owned(),
        1..=59 => format!("{mins} min ago"),
        60..=1439 => format!("{} h ago", mins / 60),
        _ => format!("{} days ago", mins / 1440),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::KeyEventKind;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: ratatui::crossterm::event::KeyEventState::NONE,
        }
    }

    fn pause_for_card() {
        std::thread::sleep(CARD_SETTLE + std::time::Duration::from_millis(50));
    }

    #[test]
    fn each_helpers_steps_share_a_number() {
        let mut app = App::new("m".into(), false);
        for (step, helper) in [("s1", "h_a"), ("s2", "h_b"), ("s3", "h_a")] {
            app.on_event(Event::StepStarted {
                turn: "t".into(),
                step: step.into(),
                tool: "read_file".into(),
                title: "Reading".into(),
                helper: Some(helper.into()),
            });
        }
        let title_of = |app: &mut App, want: &str| {
            let mut found = String::new();
            app.transcript.update(
                |k| matches!(k, Kind::Step { id, .. } if id == want),
                |k| {
                    if let Kind::Step { title, .. } = k {
                        found = title.clone();
                    }
                },
            );
            found
        };
        assert_eq!(
            title_of(&mut app, "s1"),
            format!("{HELPER_STEP_MARK}1 · Reading")
        );
        assert_eq!(
            title_of(&mut app, "s2"),
            format!("{HELPER_STEP_MARK}2 · Reading")
        );
        assert_eq!(
            title_of(&mut app, "s3"),
            format!("{HELPER_STEP_MARK}1 · Reading")
        );
    }

    #[test]
    fn typing_cannot_answer_a_card() {
        let mut app = App::new("m".into(), false);
        app.on_event(Event::PlanProposed {
            plan: "p1".into(),
            markdown: "- step".into(),
        });
        pause_for_card();
        // "Start-Sleep" typed quickly: the "a" mustn't approve the plan.
        for c in "Start-Sleep".chars() {
            assert_eq!(app.on_key(key(KeyCode::Char(c))), None);
        }
        assert!(app.permission.is_some());
    }

    fn type_text(app: &mut App, text: &str) {
        for c in text.chars() {
            app.on_key(key(KeyCode::Char(c)));
        }
        // Let the paste window pass so Enter counts as Enter.
        std::thread::sleep(PASTE_ENTER_WINDOW * 2);
    }

    #[test]
    fn enter_sends_and_slash_commands_route() {
        let mut app = App::new("m".into(), false);
        type_text(&mut app, "hello");
        assert_eq!(
            app.on_key(key(KeyCode::Enter)),
            Some(Action::Send("hello".into()))
        );
        type_text(&mut app, "/cost");
        assert_eq!(
            app.on_key(key(KeyCode::Enter)),
            Some(Action::Command {
                name: "cost".into(),
                args: String::new()
            })
        );
        type_text(&mut app, "/new");
        assert_eq!(app.on_key(key(KeyCode::Enter)), Some(Action::NewSession));
    }

    #[test]
    fn messages_typed_while_busy_are_still_sent() {
        // The run loop hands them to the running turn.
        let mut app = App::new("m".into(), false);
        app.busy_since = Some(Instant::now());
        type_text(&mut app, "next");
        assert_eq!(
            app.on_key(key(KeyCode::Enter)),
            Some(Action::Send("next".into()))
        );
    }

    #[test]
    fn permission_keys_answer() {
        let mut app = App::new("m".into(), false);
        app.permission = Some(Permission {
            request: "r1".into(),
            summary: "Run `ls`".into(),
            risk: Risk::Medium,
            diff: None,
            always: false,
            plan: false,
            picks: None,
        });
        pause_for_card();
        assert_eq!(
            app.on_key(key(KeyCode::Char('y'))),
            Some(Action::Answer(
                "r1".into(),
                Reply::Permission(PermissionChoice::Yes)
            ))
        );
        // "always" isn't on offer here, so `a` does nothing.
        pause_for_card();
        assert_eq!(app.on_key(key(KeyCode::Char('a'))), None);
        pause_for_card();
        assert_eq!(
            app.on_key(key(KeyCode::Esc)),
            Some(Action::Answer(
                "r1".into(),
                Reply::Permission(PermissionChoice::No)
            ))
        );
    }

    #[test]
    fn plan_cards_approve_or_keep_planning() {
        let mut app = App::new("m".into(), false);
        app.on_event(Event::PlanProposed {
            plan: "p1".into(),
            markdown: "- step".into(),
        });
        pause_for_card();
        assert_eq!(
            app.on_key(key(KeyCode::Char('y'))),
            Some(Action::Answer(
                "p1".into(),
                Reply::Plan {
                    approve: true,
                    auto_edits: true
                }
            ))
        );
        pause_for_card();
        assert_eq!(
            app.on_key(key(KeyCode::Char('a'))),
            Some(Action::Answer(
                "p1".into(),
                Reply::Plan {
                    approve: true,
                    auto_edits: false
                }
            ))
        );
        pause_for_card();
        assert_eq!(
            app.on_key(key(KeyCode::Char('n'))),
            Some(Action::Answer(
                "p1".into(),
                Reply::Plan {
                    approve: false,
                    auto_edits: false
                }
            ))
        );
    }

    #[test]
    fn shift_tab_cycles_modes_and_esc_stops_a_turn() {
        let mut app = App::new("m".into(), false);
        let back_tab = KeyEvent {
            code: KeyCode::BackTab,
            modifiers: KeyModifiers::SHIFT,
            kind: KeyEventKind::Press,
            state: ratatui::crossterm::event::KeyEventState::NONE,
        };
        assert_eq!(app.on_key(back_tab), Some(Action::SetMode(Mode::AutoEdits)));
        assert_eq!(app.on_key(key(KeyCode::Esc)), None, "nothing to stop");
        app.busy_since = Some(Instant::now());
        assert_eq!(app.on_key(key(KeyCode::Esc)), Some(Action::Interrupt));
    }

    #[test]
    fn suggestions_complete_with_tab() {
        let mut app = App::new("m".into(), false);
        app.commands = vec![CommandItem {
            name: "cost".into(),
            summary: "Tokens and money spent".into(),
            args_hint: String::new(),
            skill: false,
        }];
        type_text(&mut app, "/cos");
        app.on_key(key(KeyCode::Tab));
        assert_eq!(app.input_text(), "/cost ");
    }

    #[test]
    fn events_build_the_transcript() {
        let mut app = App::new("m".into(), false);
        app.on_event(Event::MessageStarted {
            turn: "t".into(),
            message: "u1".into(),
            role: Role::User,
        });
        app.on_event(Event::MessageCompleted {
            message: "u1".into(),
            text: "hi".into(),
            files: Vec::new(),
        });
        app.on_event(Event::MessageStarted {
            turn: "t".into(),
            message: "m1".into(),
            role: Role::Assistant,
        });
        app.on_event(Event::MessageDelta {
            message: "m1".into(),
            text: "Hel".into(),
        });
        app.on_event(Event::MessageCompleted {
            message: "m1".into(),
            text: "Hello".into(),
            files: Vec::new(),
        });
        let total = app.transcript.prepare(80);
        let (lines, _) = app.transcript.visible(0, total);
        let shown: String = lines
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.to_string()))
            .collect();
        assert!(shown.contains("› hi"));
        assert!(shown.contains("● Hello"));
    }
}
