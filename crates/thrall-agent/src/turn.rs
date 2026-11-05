//! One turn: the user's message in; Thrall thinks, calls tools, and replies; everything recorded.
//!
//! The loop: call the model → if it asked for tools, run each as a step (gated by the permission
//! mode, asking where needed) and call again → until it answers without tools. Messages typed
//! meanwhile join at the next step; Esc stops everything; before finishing, Thrall's own checks
//! may ask it to verify its work or correct its reply.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Instant;

use futures::StreamExt;
use futures::future::BoxFuture;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::mpsc;

use base64::Engine as _;
use thrall_core::config::HookEvent;
use thrall_core::constants::{id_prefix, usage_role};
use thrall_core::ids::{new_id, now_ms};
use thrall_gates::constants::{decision, gate};
use thrall_gates::{Input, Mode};
use thrall_llm::{
    ChatMessage, ChatRequest, ChatRole, DocumentPart, ImagePart, LlmError, StreamEvent, ToolCall,
    ToolCallAssembler, ToolSpec,
};
use thrall_protocol::{
    ErrorCode, Event, EventFrame, NoticeLevel, Role, StepStatus, TurnStatus, Usage,
};
use thrall_store::{NewEvent, NewMessage, TurnEnd, UsageRecord};
use thrall_tools::Cancel;

use crate::checks::{self, Track};
use crate::constants::{
    CHECK_PREFIX, CHECKING_NOTICE, COMPACTED_NOTICE, CUT_OFF_NOTICE, CUT_OFF_NUDGE,
    CUT_OFF_THINKING, DELTA_FLUSH_INTERVAL, DESCRIBED_MARK, EMPTY_REPLY_NOTICE, EMPTY_REPLY_NUDGE,
    EPISODE_SOURCE_MAX_CHARS, FINISH_LENGTH, HAND_OFF, HOME_NOTE, LOW_CREDIT_NOTICE,
    LOW_CREDIT_USD, MAX_MODEL_CALLS_PER_TURN, PICTURE_FAILED_MARK, PICTURE_MAX_BYTES,
    PICTURES_PER_REQUEST, PLAN_QUESTION_MAX_CHARS, SCANNED_PDF_MAX_BYTES, SESSION_TITLE_MAX_CHARS,
    SKIPPED_INTERRUPTED, SYSTEM_PROMPT, TITLE_MAX_TOKENS, TITLE_MAX_WORDS, TITLE_PROMPT,
    TITLE_SOURCE_MAX_CHARS, TRANSCRIBE_MAX_TOKENS, TRANSCRIBE_PROMPT, TRANSCRIPT_MAX_CHARS,
    VISION_MAX_TOKENS, VISION_MODEL_HINT, VISION_PROMPT, VISION_TOOL, role, turn_status,
};
use crate::error::AgentError;
use crate::hooks;
use crate::session;
use crate::state::AppState;
use crate::steps::{audit, decide_plan, run_step};

/// How a turn went.
#[derive(Debug, Clone, PartialEq)]
pub struct TurnSummary {
    /// Turn id.
    pub turn_id: String,
    /// How it ended.
    pub status: TurnStatus,
    /// Tokens and cost, all model calls together.
    pub usage: Usage,
    /// Thrall's final reply (partial if the turn failed or was stopped).
    pub reply: String,
    /// The user approved a restart during the turn.
    pub restart: bool,
    /// Messages typed too late to join this turn; send them as the next one.
    pub leftover: Vec<String>,
}

/// What a context message's `content` column holds.
#[derive(Serialize, Deserialize)]
pub(crate) struct StoredContent {
    pub(crate) text: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) tool_calls: Vec<ToolCall>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) tool_call_id: Option<String>,
    /// Files the user attached to this message (paths), so a retry can attach them again.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) files: Vec<String>,
}

/// Sends events to the client and records the ones that belong in history.
pub(crate) struct Emitter<'a> {
    state: &'a AppState,
    pub(crate) session_id: &'a str,
    pub(crate) turn_id: &'a str,
    sink: &'a mpsc::Sender<EventFrame>,
    /// Context messages are ordered by time; this keeps their timestamps strictly increasing even
    /// when several land in the same millisecond.
    last_message_at: AtomicI64,
    /// False for background helpers: their steps are neither shown nor recorded.
    record: bool,
    /// The user turned a plan down in this turn: a reply after that is talk, not a new plan.
    plan_rejected: std::sync::atomic::AtomicBool,
    /// What this turn spent besides its own model calls (picture reading and making, helpers),
    /// added to the turn's total.
    side_usage: std::sync::Mutex<Usage>,
    /// Held while a permission card is out: clients show one card at a time, so helpers working
    /// side by side ask one after another.
    pub(crate) asking: tokio::sync::Mutex<()>,
}

impl<'a> Emitter<'a> {
    pub(crate) fn new(
        state: &'a AppState,
        session_id: &'a str,
        turn_id: &'a str,
        sink: &'a mpsc::Sender<EventFrame>,
        record: bool,
    ) -> Self {
        Self {
            state,
            session_id,
            turn_id,
            sink,
            last_message_at: AtomicI64::new(0),
            record,
            plan_rejected: std::sync::atomic::AtomicBool::new(false),
            side_usage: std::sync::Mutex::new(Usage::default()),
            asking: tokio::sync::Mutex::new(()),
        }
    }

    /// Record usage that isn't one of the turn's own model calls, and count it in the turn's total.
    pub(crate) async fn record_side_usage(
        &self,
        state: &AppState,
        usage: Usage,
    ) -> Result<(), AgentError> {
        self.record_side_usage_by(state, usage, CallBy::chat(state, self.session_id))
            .await
    }

    /// `record_side_usage`, saying who made the call.
    pub(crate) async fn record_side_usage_by(
        &self,
        state: &AppState,
        usage: Usage,
        by: CallBy,
    ) -> Result<(), AgentError> {
        if let Ok(mut side) = self.side_usage.lock() {
            add_usage(&mut side, usage);
        }
        record_usage(state, self.session_id, self.turn_id, usage, by).await
    }

    /// What the turn spent on the side so far.
    fn side_usage(&self) -> Usage {
        self.side_usage.lock().map(|u| *u).unwrap_or_default()
    }

    /// Remember that the user turned a plan down in this turn.
    pub(crate) fn note_plan_rejected(&self) {
        self.plan_rejected.store(true, Ordering::Relaxed);
    }

    /// Whether the user turned a plan down in this turn.
    pub(crate) fn plan_was_rejected(&self) -> bool {
        self.plan_rejected.load(Ordering::Relaxed)
    }

    /// Whether events and steps are recorded (false for background helpers).
    pub(crate) fn records(&self) -> bool {
        self.record
    }

    pub(crate) async fn emit(&self, event: Event) -> Result<(), AgentError> {
        if !self.record {
            return Ok(());
        }
        let at = now_ms();
        let seq = if event.is_live_only() {
            None
        } else {
            let seq = self
                .state
                .store
                .append_event(NewEvent {
                    user_id: self.state.user_id.clone(),
                    session_id: self.session_id.to_owned(),
                    turn_id: Some(self.turn_id.to_owned()),
                    kind: event.type_name().to_owned(),
                    payload: serde_json::to_string(&event)?,
                    at,
                })
                .await?;
            Some(seq)
        };
        let frame = EventFrame::new(self.session_id, seq, at, event);
        if self.sink.send(frame).await.is_err() {
            // The client went away; the turn still finishes and is recorded for replay.
            tracing::debug!(session = self.session_id, "event receiver closed");
        }
        Ok(())
    }

    fn next_message_at(&self) -> i64 {
        let now = now_ms();
        let prev = self.last_message_at.fetch_max(now, Ordering::SeqCst);
        if prev >= now {
            self.last_message_at.fetch_add(1, Ordering::SeqCst) + 1
        } else {
            now
        }
    }

    pub(crate) async fn save_message(
        &self,
        role: &str,
        content: StoredContent,
        id: Option<String>,
    ) -> Result<(), AgentError> {
        let at = self.next_message_at();
        self.state
            .store
            .add_message(NewMessage {
                id: id.unwrap_or_else(|| new_id(id_prefix::MESSAGE)),
                user_id: self.state.user_id.clone(),
                session_id: self.session_id.to_owned(),
                turn_id: Some(self.turn_id.to_owned()),
                role: role.to_owned(),
                content: serde_json::to_string(&content)?,
                tokens: 0,
                at,
            })
            .await?;
        Ok(())
    }

    /// A user message: into the context (with any `@path` files attached), and echoed as typed so
    /// every client shows it. Pictures are read by the picture model first (shown as a step) and
    /// its description is kept with the message, so the chat model works from text.
    async fn user_message(
        &self,
        text: &str,
        workspace: &Path,
        from_hooks: &str,
        files: &[PathBuf],
    ) -> Result<(), AgentError> {
        let id = new_id(id_prefix::MESSAGE);
        self.emit(Event::MessageStarted {
            turn: self.turn_id.to_owned(),
            message: id.clone(),
            role: Role::User,
        })
        .await?;
        self.emit(Event::MessageCompleted {
            message: id.clone(),
            text: text.to_owned(),
            files: files.iter().map(|p| crate::extras::file_ref(p)).collect(),
        })
        .await?;
        let mut for_model =
            crate::extras::expand_mentions(self.state, self.session_id, workspace, text);
        for_model.push_str(&crate::extras::skill_run(self.state, workspace, text));
        if let Some(note) = self.state.check_in_note(self.session_id) {
            for_model.push_str("\n\n");
            for_model.push_str(&note);
        }
        let (attached, scanned) =
            crate::extras::attach_files(self.state, self.session_id, files).await;
        for_model.push_str(&attached);
        for_model.push_str(&self.describe_pictures(text, files).await?);
        for pdf in &scanned {
            for_model.push_str(&self.transcribe_pdf(text, pdf).await?);
        }
        for clip in files
            .iter()
            .filter(|p| crate::audio::audio_format(p).is_some())
        {
            for_model.push_str(&crate::audio::transcribe_attachment(self, self.state, clip).await?);
        }
        if !from_hooks.is_empty() {
            for_model.push_str(&format!("\n\n[From the user's hooks]\n{from_hooks}"));
        }
        self.save_message(
            role::USER,
            StoredContent {
                text: for_model,
                tool_calls: Vec::new(),
                tool_call_id: None,
                files: files
                    .iter()
                    .map(|p| p.to_string_lossy().into_owned())
                    .collect(),
            },
            Some(id),
        )
        .await
    }

    /// The picture model (`[provider] vision_model`) describes the attached pictures, whatever the
    /// chat model can do; its description, ready to add to the message ("" when there are no
    /// pictures or no picture model is set: then pictures go to the chat model directly).
    async fn describe_pictures(&self, text: &str, files: &[PathBuf]) -> Result<String, AgentError> {
        let pictures: Vec<&PathBuf> = files
            .iter()
            .filter(|p| crate::extras::image_mime(p).is_some())
            .take(PICTURES_PER_REQUEST)
            .collect();
        if pictures.is_empty() || self.picture_readers().is_empty() {
            return Ok(String::new());
        }
        let names = pictures
            .iter()
            .map(|p| file_name(p))
            .collect::<Vec<_>>()
            .join(", ");
        let mut message = ChatMessage::new(
            ChatRole::User,
            format!("{VISION_PROMPT}\n\nThe user's message:\n\"\"\"\n{text}\n\"\"\""),
        );
        for path in &pictures {
            let small = std::fs::metadata(path).is_ok_and(|m| m.len() <= PICTURE_MAX_BYTES);
            if let (true, Ok(bytes), Some(mime)) =
                (small, std::fs::read(path), crate::extras::image_mime(path))
            {
                message.images.push(ImagePart {
                    mime: mime.to_owned(),
                    base64: base64::engine::general_purpose::STANDARD.encode(bytes),
                });
            }
        }
        Ok(
            match self
                .picture_reader_step(format!("Looking at {names}"), message, VISION_MAX_TOKENS)
                .await?
            {
                Ok((reader, described)) => format!(
                    "\n\n{DESCRIBED_MARK} by {reader}, Thrall's picture reader. Answer from this \
                     description; don't say you can't see the picture.]\n{described}"
                ),
                Err(errors) => format!(
                    "\n\n{PICTURE_FAILED_MARK}: {names}. {errors}. Tell the user plainly that \
                     reading the picture failed this time and that Retry will try again; don't \
                     suggest changing model.]"
                ),
            },
        )
    }

    /// A PDF that's mostly pictures of pages, transcribed by the picture model; the text to add
    /// to the message ("" when no picture model is set).
    async fn transcribe_pdf(&self, text: &str, path: &Path) -> Result<String, AgentError> {
        if self.picture_readers().is_empty() {
            return Ok(String::new());
        }
        let name = file_name(path);
        let bytes = match std::fs::metadata(path) {
            Ok(m) if m.len() <= SCANNED_PDF_MAX_BYTES => std::fs::read(path).ok(),
            _ => None,
        };
        let Some(bytes) = bytes else {
            return Ok(format!(
                "\n\n[{name} is mostly pictures of pages and too big for Thrall's picture reader \
                 to transcribe; tell the user only the headings above could be read.]"
            ));
        };
        let mut message = ChatMessage::new(
            ChatRole::User,
            format!("{TRANSCRIBE_PROMPT}\n\nThe user's message:\n\"\"\"\n{text}\n\"\"\""),
        );
        message.documents.push(DocumentPart {
            name: name.clone(),
            mime: "application/pdf".to_owned(),
            base64: base64::engine::general_purpose::STANDARD.encode(bytes),
        });
        Ok(
            match self
                .picture_reader_step(format!("Reading {name}"), message, TRANSCRIBE_MAX_TOKENS)
                .await?
            {
                Ok((reader, transcript)) => {
                    let transcript: String =
                        transcript.chars().take(TRANSCRIPT_MAX_CHARS).collect();
                    format!(
                        "\n\n[{name}, transcribed from its page images by {reader}, Thrall's \
                         picture reader. Use this instead of the extracted text above.]\n{transcript}"
                    )
                }
                Err(errors) => format!(
                    "\n\n[Thrall's picture reader couldn't transcribe {name}: {errors}. Tell the \
                     user plainly that reading the pages failed this time and that Retry will try \
                     again; don't suggest changing model.]"
                ),
            },
        )
    }

    /// The picture model, then its backup (another company's, so one busy provider doesn't stop
    /// Thrall reading). Empty when no picture model is set.
    pub(crate) fn picture_readers(&self) -> Vec<String> {
        let provider = &self.state.config.provider;
        let vision = provider.vision_model.trim().to_owned();
        if vision.is_empty() {
            return Vec::new();
        }
        let backup = provider.vision_backup_model.trim().to_owned();
        let mut readers = vec![vision];
        if !backup.is_empty() && !readers.contains(&backup) {
            readers.push(backup);
        }
        readers
    }

    /// Run `message` past the picture readers in turn, shown as a step titled `title`: which
    /// reader answered and its text, or every reader's error.
    async fn picture_reader_step(
        &self,
        title: String,
        message: ChatMessage,
        max_tokens: u32,
    ) -> Result<Result<(String, String), String>, AgentError> {
        let readers = self.picture_readers();
        let step = new_id(id_prefix::STEP);
        self.emit(Event::StepStarted {
            turn: self.turn_id.to_owned(),
            step: step.clone(),
            tool: VISION_TOOL.to_owned(),
            title,
            helper: None,
        })
        .await?;
        let mut errors = Vec::new();
        let mut result = None;
        for reader in &readers {
            match self
                .read_pictures(reader, message.clone(), max_tokens)
                .await?
            {
                Ok(text) => {
                    result = Some((reader.clone(), text));
                    break;
                }
                Err(e) => errors.push(format!("{reader}: {e}")),
            }
        }
        let first = readers.first().cloned().unwrap_or_default();
        let (status, summary) = match &result {
            Some((reader, _)) if *reader == first => (StepStatus::Ok, format!("Read by {reader}")),
            Some((reader, _)) => (
                StepStatus::Ok,
                format!("Read by {reader} ({first} was unavailable)"),
            ),
            None => (
                StepStatus::Error,
                format!("Couldn't read it: {}", errors.join("; ")),
            ),
        };
        self.emit(Event::StepCompleted {
            step,
            status,
            summary,
            diff: None,
            files: Vec::new(),
        })
        .await?;
        Ok(result.ok_or_else(|| errors.join("; ")))
    }
    /// Ask one picture model to describe; its text, or why it failed. Usage is recorded either way.
    pub(crate) async fn read_pictures(
        &self,
        model: &str,
        message: ChatMessage,
        max_tokens: u32,
    ) -> Result<Result<String, String>, AgentError> {
        let request = ChatRequest {
            model: model.to_owned(),
            messages: vec![message],
            max_tokens,
            tools: Vec::new(),
            thinking: None,
        };
        let mut described = String::new();
        let mut usage = Usage::default();
        let mut failed = None;
        let mut stream = self.state.llm.stream(request);
        while let Some(item) = stream.next().await {
            match item {
                Ok(StreamEvent::Text(t)) => described.push_str(&t),
                Ok(StreamEvent::Usage(u)) => {
                    usage = Usage {
                        input_tokens: u.input_tokens,
                        output_tokens: u.output_tokens,
                        cost_usd: u.cost_usd,
                    }
                }
                Ok(_) => {}
                Err(e) => {
                    failed = Some(e.to_string());
                    break;
                }
            }
        }
        self.record_side_usage(self.state, usage).await?;
        let described = described.trim().to_owned();
        Ok(match failed {
            Some(e) => Err(e),
            None if described.is_empty() => Err("it returned no description".to_owned()),
            None => Ok(described),
        })
    }
}

/// Whether a reply without tool calls, in plan mode, reads as a plan: not empty, and not a short
/// question back to the user.
fn looks_like_plan(text: &str) -> bool {
    let text = text.trim();
    !text.is_empty() && !(text.ends_with('?') && text.chars().count() < PLAN_QUESTION_MAX_CHARS)
}

/// A path's file name, for messages.
fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// What one model call produced.
struct CallOutcome {
    text: String,
    tool_calls: Vec<ToolCall>,
    usage: Usage,
    /// Tokens sent that the provider had cached.
    cached_tokens: u64,
    error: Option<LlmError>,
    interrupted: bool,
    /// The model stopped at its length limit (finish reason "length").
    cut_off: bool,
}

/// Run one turn in `session_id` for the user's `text`, sending events to `sink` as they happen.
pub async fn run(
    state: &AppState,
    session_id: &str,
    text: &str,
    sink: &mpsc::Sender<EventFrame>,
) -> Result<TurnSummary, AgentError> {
    run_with_files(state, session_id, text, &[], sink).await
}

/// `run`, with files the user attached (uploaded to the engine).
pub async fn run_with_files(
    state: &AppState,
    session_id: &str,
    text: &str,
    files: &[PathBuf],
    sink: &mpsc::Sender<EventFrame>,
) -> Result<TurnSummary, AgentError> {
    // The input gate, before anything is recorded.
    let text = match thrall_gates::input_gate(text) {
        Input::Ok(t) => t,
        Input::Refused(why) => {
            audit(
                state,
                gate::INPUT,
                "message",
                decision::DENY,
                json!({ "reason": why }),
            )
            .await?;
            return Err(AgentError::InputRefused(why));
        }
    };
    let session = session::get(state, session_id).await?;
    let Some(cancel) = state.start_running(session_id) else {
        return Err(AgentError::Busy);
    };
    let slot = TurnSlot {
        state,
        session_id,
        released: std::sync::atomic::AtomicBool::new(false),
    };
    let result = run_turn(state, &session, &text, files, sink, cancel, &slot).await;
    // Released already when the turn finished normally; this covers turns that failed early.
    let late = slot.release();
    result.map(|mut summary| {
        summary.leftover.extend(late);
        summary
    })
}

/// A session's place as "running a turn". Released once: as soon as the turn is finished (so the
/// user can retry or send while the title and memory are written), or when it fails. A second
/// release does nothing, so it never frees a turn that started since.
struct TurnSlot<'a> {
    state: &'a AppState,
    session_id: &'a str,
    released: std::sync::atomic::AtomicBool,
}

impl TurnSlot<'_> {
    /// Free the session; returns messages typed too late to join the turn (empty the second time).
    fn release(&self) -> Vec<String> {
        if self.released.swap(true, Ordering::SeqCst) {
            return Vec::new();
        }
        self.state.stop_running(self.session_id)
    }
}

async fn run_turn(
    state: &AppState,
    session: &thrall_store::Session,
    text: &str,
    files: &[PathBuf],
    sink: &mpsc::Sender<EventFrame>,
    cancel: Cancel,
    slot: &TurnSlot<'_>,
) -> Result<TurnSummary, AgentError> {
    // Their own key is there if they use one, and they're under their monthly limit; refused
    // before anything is recorded.
    let warning = crate::people::before_turn(state).await?;
    crate::people::load_rules(state).await?;
    // Home for someone whose PC is connected works on that PC.
    let moved = crate::companions::home_on_pc(state, session).await?;
    let session = moved.as_ref().unwrap_or(session);
    let session_id = session.id.as_str();
    let user_id = state.user_id.as_str();
    let workspace = crate::session::folder_of(session);
    // A session that works on a companion PC sends its PC tools there, and its project
    // instructions and skills come from there.
    state.companions.bind(session_id, session.pc.as_deref());
    if let Some(pc) = &session.pc {
        state
            .companions
            .load_project(pc, user_id, session_id, &workspace.to_string_lossy())
            .await;
    }
    let mut mode = Mode::parse(&session.mode).unwrap_or_default();
    let turn_id = new_id(id_prefix::TURN);
    let emitter = Emitter::new(state, session_id, &turn_id, sink, true);

    state
        .store
        .start_turn(user_id, session_id, &turn_id, now_ms())
        .await?;
    emitter
        .emit(Event::TurnStarted {
            turn: turn_id.clone(),
            scheduled: state.is_unattended(session_id),
        })
        .await?;
    if let Some(text) = warning {
        emitter
            .emit(Event::Notice {
                level: NoticeLevel::Warning,
                text,
            })
            .await?;
    }

    // A placeholder title straight away; the model writes a better one after the first turn.
    let needs_title = session.title.is_empty();
    if needs_title {
        state
            .store
            .set_session_title(user_id, session_id, &title_from(text))
            .await?;
    }
    let hook = hooks::fire(
        state,
        HookEvent::UserMessage,
        None,
        json!({ "text": text }),
        &workspace,
    )
    .await;
    emitter
        .user_message(text, &workspace, &hook.output, files)
        .await?;

    // Split mode (experimental): the Thinker runs the turn, looking only, and hands off the work.
    let mut split = crate::split::for_turn(state, session_id);
    let tools = match &split {
        Some(_) => crate::split::thinker_tools(tool_specs(state)),
        None => tool_specs(state),
    };
    // The tools' descriptions are sent with every call, so they count towards the context.
    let tool_tokens: usize = tools
        .iter()
        .map(|t| {
            thrall_memory::compaction::estimate_tokens(&t.name)
                + thrall_memory::compaction::estimate_tokens(&t.description)
                + thrall_memory::compaction::estimate_tokens(&t.parameters.to_string())
        })
        .sum();
    let mut total = Usage::default();
    let mut reply = String::new();
    let mut failure: Option<LlmError> = None;
    let mut interrupted = false;
    // An empty final reply is sent back once for a summary.
    let mut asked_for_reply = false;
    // Set once a reply is cut off before saying anything: the model then thinks less.
    let mut thinking = None;
    let mut track = Track::default();
    track.look_only = split.is_some();
    let mut calls = 0;

    loop {
        if cancel.is_cancelled() {
            interrupted = true;
            break;
        }
        // Messages typed while Thrall works join here, before its next step.
        for extra in state.take_inbox(session_id) {
            if let Input::Ok(extra) = thrall_gates::input_gate(&extra) {
                emitter.user_message(&extra, &workspace, "", &[]).await?;
            }
        }
        if calls >= MAX_MODEL_CALLS_PER_TURN {
            emitter
                .emit(Event::Notice {
                    level: NoticeLevel::Warning,
                    text: format!(
                        "Stopped after {MAX_MODEL_CALLS_PER_TURN} steps in one turn. Say \
                         \"carry on\" to let Thrall continue."
                    ),
                })
                .await?;
            break;
        }
        calls += 1;

        let mut messages = context(state, session_id, &workspace, mode).await?;
        // A long conversation gets its oldest part summarised before the next call.
        let system_tokens = messages.first().map_or(0, |m| {
            thrall_memory::compaction::estimate_tokens(&m.content)
        });
        let checked = crate::compaction::compact(
            state,
            session_id,
            &turn_id,
            system_tokens + tool_tokens,
            false,
        )
        .await?;
        // What this call's context was estimated at, to check against the provider's count
        // (unknown after compacting: the context just changed).
        let mut estimated = Some(checked.estimated_tokens);
        if checked.compacted.is_some() {
            estimated = None;
            emitter
                .emit(Event::Notice {
                    level: NoticeLevel::Info,
                    text: COMPACTED_NOTICE.to_owned(),
                })
                .await?;
            messages = context(state, session_id, &workspace, mode).await?;
        }
        let (model, role) = match &split {
            Some(s) => {
                crate::split::brief_thinker(&mut messages);
                (s.thinker.clone(), usage_role::THINKER)
            }
            None => (state.model_for(session_id), usage_role::CHAT),
        };
        let request = ChatRequest {
            model: model.clone(),
            messages,
            max_tokens: state.config.provider.max_tokens,
            tools: tools.clone(),
            thinking,
        };
        let outcome = stream_call(&emitter, state, &turn_id, request, &cancel).await?;
        if let Some(scale) = estimated.and_then(|e| {
            thrall_memory::compaction::token_scale(e, outcome.usage.input_tokens as usize)
        }) {
            state.set_token_scale(session_id, scale);
        }
        add_usage(&mut total, outcome.usage);
        let by = CallBy {
            model,
            role,
            cached_tokens: outcome.cached_tokens,
        };
        record_usage(state, session_id, &turn_id, outcome.usage, by).await?;
        reply = outcome.text.clone();

        if let Some(error) = outcome.error {
            // A failed reply stays out of the model's context so the next turn starts clean.
            failure = Some(error);
            break;
        }
        if outcome.interrupted {
            // Keep what was said before the interruption, so the next turn knows.
            if !outcome.text.is_empty() {
                emitter
                    .save_message(
                        role::ASSISTANT,
                        StoredContent {
                            text: outcome.text,
                            tool_calls: Vec::new(),
                            tool_call_id: None,
                            files: Vec::new(),
                        },
                        None,
                    )
                    .await?;
            }
            interrupted = true;
            break;
        }
        if !outcome.text.is_empty() || !outcome.tool_calls.is_empty() {
            emitter
                .save_message(
                    role::ASSISTANT,
                    StoredContent {
                        text: outcome.text.clone(),
                        tool_calls: outcome.tool_calls.clone(),
                        tool_call_id: None,
                        files: Vec::new(),
                    },
                    None,
                )
                .await?;
        }
        if outcome.tool_calls.is_empty() && outcome.text.trim().is_empty() {
            // Out of credit: OpenRouter shortens replies to what the credit can pay for instead of
            // refusing, so they come back empty. Asking again only spends more: say so and stop.
            if let Some(left) = state.llm.credit_left().await
                && left < LOW_CREDIT_USD
            {
                emitter
                    .emit(Event::Notice {
                        level: NoticeLevel::Warning,
                        text: LOW_CREDIT_NOTICE
                            .replace("{provider}", state.llm.name())
                            .replace("{left}", &format!("{:.2}", left.max(0.0))),
                    })
                    .await?;
                break;
            }
            // Finished without a word: ask once for a summary (or, when the reply ran out of room,
            // for smaller steps), then say so plainly.
            if !asked_for_reply {
                asked_for_reply = true;
                let nudge = if outcome.cut_off {
                    thinking = Some(CUT_OFF_THINKING);
                    CUT_OFF_NUDGE
                } else {
                    EMPTY_REPLY_NUDGE
                };
                emitter
                    .save_message(
                        role::USER,
                        StoredContent {
                            text: format!("{CHECK_PREFIX} {nudge}"),
                            tool_calls: Vec::new(),
                            tool_call_id: None,
                            files: Vec::new(),
                        },
                        None,
                    )
                    .await?;
                continue;
            }
            emitter
                .emit(Event::Notice {
                    level: NoticeLevel::Warning,
                    text: if outcome.cut_off {
                        CUT_OFF_NOTICE
                    } else {
                        EMPTY_REPLY_NOTICE
                    }
                    .to_owned(),
                })
                .await?;
            break;
        }
        if outcome.tool_calls.is_empty()
            && mode == Mode::Plan
            && looks_like_plan(&outcome.text)
            && !emitter.plan_was_rejected()
        {
            // Models often write the plan as a reply instead of calling exit_plan_mode: the reply
            // is the plan. Approve it and the work starts in this same turn. Not after the user
            // turned a plan down (the reply then asks what to change), and not for a question.
            if decide_plan(&emitter, state, outcome.text.clone(), &mut mode, &cancel).await? {
                emitter
                    .save_message(
                        role::USER,
                        StoredContent {
                            text: format!(
                                "{CHECK_PREFIX} The user approved your plan. Permission mode is \
                                 now: {}. Carry it out now, keeping the task list up to date.",
                                mode.label()
                            ),
                            tool_calls: Vec::new(),
                            tool_call_id: None,
                            files: Vec::new(),
                        },
                        None,
                    )
                    .await?;
                continue;
            }
            break;
        }
        if outcome.tool_calls.is_empty() {
            // A final answer: Thrall's own checks may send it back for one more look.
            match checks::after_reply(&state.config.agent, &mut track, &outcome.text) {
                Some(nudge) => {
                    emitter
                        .emit(Event::Notice {
                            level: NoticeLevel::Info,
                            text: CHECKING_NOTICE.to_owned(),
                        })
                        .await?;
                    emitter
                        .save_message(
                            role::USER,
                            StoredContent {
                                text: nudge,
                                tool_calls: Vec::new(),
                                tool_call_id: None,
                                files: Vec::new(),
                            },
                            None,
                        )
                        .await?;
                    continue;
                }
                None => break,
            }
        }
        let calls = &outcome.tool_calls;
        let mut next = 0;
        while next < calls.len() {
            let call = &calls[next];
            // Helpers asked for one after another in this reply work side by side (up to the
            // setting); each gets its own copy of the mode and its own checks, and their results
            // are saved in the order they were asked for.
            let run = calls[next..]
                .iter()
                .take_while(|c| crate::workers::is_foreground_helper(c))
                .count();
            if run > 1 && split.is_none() && !cancel.is_cancelled() {
                let group = &calls[next..next + run];
                let mut slots: Vec<(Mode, Track)> =
                    group.iter().map(|_| (mode, Track::default())).collect();
                // Boxed one by one (not from a closure), so the turn stays Send.
                let mut jobs: Vec<BoxFuture<'_, Result<String, AgentError>>> = Vec::new();
                for ((m, t), c) in slots.iter_mut().zip(group) {
                    jobs.push(Box::pin(run_step(
                        &emitter, state, &workspace, c, m, t, &cancel,
                    )));
                }
                let results: Vec<Result<String, AgentError>> = futures::stream::iter(jobs)
                    .buffered(state.config.agent.helpers_at_once())
                    .collect()
                    .await;
                for (c, content) in group.iter().zip(results) {
                    emitter
                        .save_message(
                            role::TOOL,
                            StoredContent {
                                text: content?,
                                tool_calls: Vec::new(),
                                tool_call_id: Some(c.id.clone()),
                                files: Vec::new(),
                            },
                            None,
                        )
                        .await?;
                }
                next += run;
                continue;
            }
            next += 1;
            if cancel.is_cancelled() {
                // Every call still needs a result, or the next request is refused.
                emitter
                    .save_message(
                        role::TOOL,
                        StoredContent {
                            text: SKIPPED_INTERRUPTED.to_owned(),
                            tool_calls: Vec::new(),
                            tool_call_id: Some(call.id.clone()),
                            files: Vec::new(),
                        },
                        None,
                    )
                    .await?;
                continue;
            }
            let content = match split.as_mut() {
                Some(s) if call.name == HAND_OFF => {
                    crate::split::hand_off(
                        &emitter, state, &workspace, s, call, mode, &mut track, &cancel,
                    )
                    .await?
                }
                _ => {
                    run_step(
                        &emitter, state, &workspace, call, &mut mode, &mut track, &cancel,
                    )
                    .await?
                }
            };
            emitter
                .save_message(
                    role::TOOL,
                    StoredContent {
                        text: content,
                        tool_calls: Vec::new(),
                        tool_call_id: Some(call.id.clone()),
                        files: Vec::new(),
                    },
                    None,
                )
                .await?;
        }
    }

    let status = if let Some(error) = &failure {
        emitter
            .emit(Event::Error {
                code: error_code(error),
                message: error.to_string(),
                retry: error.is_retryable(),
            })
            .await?;
        TurnStatus::Failed
    } else if interrupted {
        emitter
            .emit(Event::Notice {
                level: NoticeLevel::Info,
                text: "Stopped. Tell Thrall what to do next.".to_owned(),
            })
            .await?;
        TurnStatus::Interrupted
    } else {
        TurnStatus::Done
    };

    add_usage(&mut total, emitter.side_usage());
    state
        .store
        .finish_turn(TurnEnd {
            user_id: user_id.to_owned(),
            turn_id: turn_id.clone(),
            status: match status {
                TurnStatus::Done => turn_status::DONE,
                TurnStatus::Interrupted => turn_status::INTERRUPTED,
                TurnStatus::Failed => turn_status::FAILED,
            }
            .to_owned(),
            at: now_ms(),
            input_tokens: total.input_tokens,
            output_tokens: total.output_tokens,
            cost_usd: total.cost_usd,
        })
        .await?;
    emitter
        .emit(Event::TurnCompleted {
            turn: turn_id.clone(),
            status,
            usage: total,
        })
        .await?;
    // The turn is over: free the session now. What follows (hooks, title, memory) can't fail the
    // turn, and the user can retry or send meanwhile.
    let leftover = slot.release();

    hooks::fire(
        state,
        HookEvent::TurnEnd,
        None,
        json!({ "status": format!("{status:?}").to_lowercase(), "reply": reply }),
        &workspace,
    )
    .await;

    if needs_title
        && status == TurnStatus::Done
        && let Err(e) = name_session(&emitter, state, session_id, &turn_id, text).await
    {
        tracing::warn!(%e, "couldn't name the session");
    }

    // Remember the exchange, so later sessions can recall it (memory_search).
    if status == TurnStatus::Done
        && state.config.memory.remember_conversations
        && !state.is_check_in(session_id)
        && !reply.trim().is_empty()
    {
        let title = session::get(state, session_id)
            .await
            .map(|s| s.title)
            .unwrap_or_default();
        let summary = if title.is_empty() {
            title_from(text)
        } else {
            title
        };
        let content: String = format!("User: {text}\nThrall: {reply}")
            .chars()
            .take(EPISODE_SOURCE_MAX_CHARS)
            .collect();
        if let Err(e) = state
            .memory
            .remember_conversation(&state.user_id, Some(session_id), &summary, &content)
            .await
        {
            tracing::warn!(%e, "couldn't remember this conversation");
        }
    }

    Ok(TurnSummary {
        turn_id,
        status,
        usage: total,
        reply,
        restart: track.restart,
        leftover,
    })
}

/// One streamed model call. Text goes out as it arrives; tool calls are gathered whole. The output
/// gate removes secrets from the final text.
async fn stream_call(
    emitter: &Emitter<'_>,
    state: &AppState,
    turn_id: &str,
    request: ChatRequest,
    cancel: &Cancel,
) -> Result<CallOutcome, AgentError> {
    let mut message_id: Option<String> = None;
    let mut text = String::new();
    let mut pending_text = String::new();
    let mut pending_reasoning = String::new();
    let mut calls = ToolCallAssembler::default();
    let mut usage = Usage::default();
    let mut cached_tokens = 0;
    let mut error = None;
    let mut interrupted = false;
    let mut cut_off = false;
    let mut last_flush = Instant::now();
    let mut cancel = cancel.clone();

    let mut stream = state.llm.stream(request);
    loop {
        let item = tokio::select! {
            item = stream.next() => item,
            () = cancel.cancelled() => {
                interrupted = true;
                break;
            }
        };
        let Some(item) = item else { break };
        match item {
            Ok(StreamEvent::Text(t)) => {
                start_message(emitter, turn_id, &mut message_id).await?;
                text.push_str(&t);
                pending_text.push_str(&t);
            }
            Ok(StreamEvent::Reasoning(t)) => {
                start_message(emitter, turn_id, &mut message_id).await?;
                pending_reasoning.push_str(&t);
            }
            Ok(StreamEvent::ToolCall(delta)) => calls.push(delta),
            Ok(StreamEvent::Usage(u)) => {
                usage = Usage {
                    input_tokens: u.input_tokens,
                    output_tokens: u.output_tokens,
                    cost_usd: u.cost_usd,
                };
                cached_tokens = u.cached_tokens;
            }
            Ok(StreamEvent::Finished(reason)) => {
                cut_off = reason.as_deref() == Some(FINISH_LENGTH);
            }
            Err(e) => {
                error = Some(e);
                break;
            }
        }
        if last_flush.elapsed() >= DELTA_FLUSH_INTERVAL {
            if let Some(id) = &message_id {
                flush(emitter, id, &mut pending_text, &mut pending_reasoning).await?;
            }
            last_flush = Instant::now();
        }
    }

    // The output gate: keys and tokens never stay in what Thrall says.
    let (clean, found) = thrall_gates::redact(&text);
    if found > 0 {
        audit(
            state,
            gate::OUTPUT,
            "reply",
            decision::CHANGED,
            json!({ "secrets_removed": found }),
        )
        .await?;
        text = clean;
        pending_text.clear();
    }
    // Links to ChatGPT's file folder lead nowhere here: name the file instead. The completed
    // message replaces what was streamed.
    let (named, changed) = crate::links::fix(&text);
    if changed {
        text = named;
        pending_text.clear();
    }
    if let Some(id) = &message_id {
        flush(emitter, id, &mut pending_text, &mut pending_reasoning).await?;
        emitter
            .emit(Event::MessageCompleted {
                message: id.clone(),
                text: text.clone(),
                files: Vec::new(),
            })
            .await?;
    }
    Ok(CallOutcome {
        text,
        tool_calls: if error.is_some() || interrupted {
            Vec::new()
        } else {
            calls.finish()
        },
        usage,
        cached_tokens,
        error,
        interrupted,
        cut_off,
    })
}

/// Announce Thrall's message the first time it has something to show.
async fn start_message(
    emitter: &Emitter<'_>,
    turn_id: &str,
    message_id: &mut Option<String>,
) -> Result<(), AgentError> {
    if message_id.is_none() {
        let id = new_id(id_prefix::MESSAGE);
        emitter
            .emit(Event::MessageStarted {
                turn: turn_id.to_owned(),
                message: id.clone(),
                role: Role::Assistant,
            })
            .await?;
        *message_id = Some(id);
    }
    Ok(())
}

async fn flush(
    emitter: &Emitter<'_>,
    message_id: &str,
    text: &mut String,
    reasoning: &mut String,
) -> Result<(), AgentError> {
    if !reasoning.is_empty() {
        emitter
            .emit(Event::ReasoningDelta {
                message: message_id.to_owned(),
                text: std::mem::take(reasoning),
            })
            .await?;
    }
    if !text.is_empty() {
        emitter
            .emit(Event::MessageDelta {
                message: message_id.to_owned(),
                text: std::mem::take(text),
            })
            .await?;
    }
    Ok(())
}

fn tool_specs(state: &AppState) -> Vec<ToolSpec> {
    state
        .tools
        .iter()
        .map(|t| ToolSpec {
            name: t.name().to_owned(),
            description: t.description().to_owned(),
            parameters: t.parameters(),
        })
        .collect()
}

fn add_usage(total: &mut Usage, call: Usage) {
    total.input_tokens += call.input_tokens;
    total.output_tokens += call.output_tokens;
    total.cost_usd = match (total.cost_usd, call.cost_usd) {
        (None, None) => None,
        (a, b) => Some(a.unwrap_or(0.0) + b.unwrap_or(0.0)),
    };
}

/// Who made a model call, and how much of it the provider had cached: for its usage row.
#[derive(Debug, Clone)]
pub(crate) struct CallBy {
    /// The model that answered.
    pub(crate) model: String,
    /// `usage_role`.
    pub(crate) role: &'static str,
    /// Tokens sent that the provider had cached.
    pub(crate) cached_tokens: u64,
}

impl CallBy {
    /// An ordinary call on the session's chat model.
    pub(crate) fn chat(state: &AppState, session_id: &str) -> Self {
        Self {
            model: state.model_for(session_id),
            role: usage_role::CHAT,
            cached_tokens: 0,
        }
    }
}

pub(crate) async fn record_usage(
    state: &AppState,
    session_id: &str,
    turn_id: &str,
    usage: Usage,
    by: CallBy,
) -> Result<(), AgentError> {
    if usage == Usage::default() {
        return Ok(());
    }
    state
        .store
        .record_usage(UsageRecord {
            id: new_id(id_prefix::USAGE),
            user_id: state.user_id.clone(),
            session_id: Some(session_id.to_owned()),
            turn_id: Some(turn_id.to_owned()),
            provider: state.config.provider.kind.as_str().to_owned(),
            model: by.model,
            role: by.role.to_owned(),
            input_tokens: usage.input_tokens,
            cached_tokens: by.cached_tokens,
            output_tokens: usage.output_tokens,
            cost_usd: usage.cost_usd,
            at: now_ms(),
        })
        .await?;
    Ok(())
}

/// The system prompt plus the session's stored context, oldest first.
async fn context(
    state: &AppState,
    session_id: &str,
    workspace: &Path,
    mode: Mode,
) -> Result<Vec<ChatMessage>, AgentError> {
    let stored = state
        .store
        .context_messages(&state.user_id, session_id)
        .await?;
    let mut messages = Vec::with_capacity(stored.len() + 1);
    let mut system = format!(
        "{SYSTEM_PROMPT}\n\nWorking folder: {}\nOperating system: {}\nToday: {}\nPermission \
         mode: {}",
        workspace.display(),
        std::env::consts::OS,
        today(),
        mode.label()
    );
    if let Some(style) = crate::personality::prompt(&crate::personality::load(state)) {
        system.push_str("\n\n");
        system.push_str(&style);
    }
    if let Some(lessons) = crate::experience::prompt(&crate::experience::list(state)) {
        system.push_str("\n\n");
        system.push_str(&lessons);
    }
    if mode == Mode::Plan {
        system.push_str(
            "\n\nPlan mode is on: look around with the read-only tools, then present a plan with \
             exit_plan_mode and wait for approval. Nothing can be changed until it's approved. \
             To look at what an app offers through mcp_call (listing its commands, reading its \
             state), call it: the person is asked first unless the server marks the tool \
             read-only. Make one such call at a time, and don't use it to change anything.",
        );
    }
    system.push_str("\n\n");
    system.push_str(crate::questions::note(state.config.agent.questions, mode));
    if session::get(state, session_id)
        .await
        .is_ok_and(|s| s.home_day.is_some())
    {
        system.push_str("\n\n");
        system.push_str(HOME_NOTE);
    }
    if let Some(rules) = crate::writing::rules(state.config.writing.humanise) {
        system.push_str("\n\n");
        system.push_str(&rules);
    }
    system.push_str(&crate::extras::standing_instructions(
        state, session_id, workspace,
    ));
    system.push_str(&crate::extras::helper_types(state, workspace));
    messages.push(ChatMessage::new(ChatRole::System, system));
    // Old big tool results go shortened when trimming is on (the stored ones keep everything).
    let trim: Vec<usize> = if state.config.memory.trim_old_tool_results {
        let items: Vec<thrall_memory::trim::Item> = stored
            .iter()
            .map(|m| thrall_memory::trim::Item {
                is_user: m.role == role::USER,
                tool_chars: (m.role == role::TOOL).then(|| {
                    serde_json::from_str::<StoredContent>(&m.content)
                        .map_or(0, |c| c.text.chars().count())
                }),
            })
            .collect();
        thrall_memory::trim::plan(&items)
    } else {
        Vec::new()
    };
    if !trim.is_empty() {
        let ids: Vec<&str> = trim.iter().map(|&i| stored[i].id.as_str()).collect();
        tracing::info!(
            session = session_id,
            count = trim.len(),
            messages = ids.join(","),
            "trimmed old tool results (memory.trim_old_tool_results is on)"
        );
    }
    // User messages with pictures attached, by position, newest last.
    let mut with_pictures: Vec<(usize, Vec<PathBuf>)> = Vec::new();
    for (index, message) in stored.into_iter().enumerate() {
        let role = match message.role.as_str() {
            role::USER => ChatRole::User,
            role::ASSISTANT => ChatRole::Assistant,
            role::TOOL => ChatRole::Tool,
            other => {
                tracing::warn!(
                    role = other,
                    id = message.id,
                    "skipping message with unknown role"
                );
                continue;
            }
        };
        let mut content: StoredContent = serde_json::from_str(&message.content)?;
        if trim.contains(&index) {
            content.text = thrall_memory::trim::trimmed(&content.text);
        }
        if role == ChatRole::User {
            let pictures: Vec<PathBuf> = content
                .files
                .iter()
                .map(PathBuf::from)
                .filter(|p| crate::extras::image_mime(p).is_some())
                .collect();
            if !pictures.is_empty() {
                with_pictures.push((messages.len(), pictures));
            }
        }
        messages.push(ChatMessage {
            role,
            content: content.text,
            tool_calls: content.tool_calls,
            tool_call_id: content.tool_call_id,
            images: Vec::new(),
            documents: Vec::new(),
            audio: Vec::new(),
        });
    }
    attach_pictures(state, &mut messages, &with_pictures).await;
    Ok(repair_tool_results(messages))
}

/// Send the pictures the user attached with their messages, newest first, up to a few per request.
/// A model that can't see gets a note instead, so it tells the user rather than guessing.
async fn attach_pictures(
    state: &AppState,
    messages: &mut [ChatMessage],
    with_pictures: &[(usize, Vec<PathBuf>)],
) {
    if with_pictures.is_empty() {
        return;
    }
    let sees = state.llm.accepts_images(&state.model()).await;
    let mut budget = PICTURES_PER_REQUEST;
    for (index, paths) in with_pictures.iter().rev() {
        let Some(message) = messages.get_mut(*index) else {
            continue;
        };
        if message.content.contains(DESCRIBED_MARK)
            || (message.content.contains(PICTURE_FAILED_MARK) && sees != Some(true))
        {
            continue;
        }
        if sees == Some(false) {
            message.content.push_str(&format!(
                "\n\n[The current model ({}) can't see pictures, so the picture wasn't sent. Tell \
                 the user, and suggest switching to one that can, e.g. /model {VISION_MODEL_HINT}.]",
                state.model()
            ));
            continue;
        }
        for path in paths {
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let bytes = match std::fs::metadata(path) {
                Ok(m) if m.len() <= PICTURE_MAX_BYTES && budget > 0 => std::fs::read(path).ok(),
                _ => None,
            };
            match (bytes, crate::extras::image_mime(path)) {
                (Some(bytes), Some(mime)) => {
                    budget -= 1;
                    message.images.push(ImagePart {
                        mime: mime.to_owned(),
                        base64: base64::engine::general_purpose::STANDARD.encode(bytes),
                    });
                }
                _ => message.content.push_str(&format!(
                    "\n[Picture {name} not sent this time (too big, missing, or older than the \
                     last {PICTURES_PER_REQUEST}).]"
                )),
            }
        }
    }
}

/// Every tool call needs a result right after it, or providers refuse the whole request. A turn
/// that died mid-tool can leave one missing; fill the gap instead of breaking the session.
fn repair_tool_results(messages: Vec<ChatMessage>) -> Vec<ChatMessage> {
    let mut out: Vec<ChatMessage> = Vec::with_capacity(messages.len());
    let mut open: Vec<String> = Vec::new();
    for message in messages {
        let is_result = message.role == ChatRole::Tool;
        if !is_result && !open.is_empty() {
            for id in open.drain(..) {
                out.push(ChatMessage::tool_result(
                    id,
                    "No result: the turn was interrupted.",
                ));
            }
        }
        if is_result {
            match message.tool_call_id.as_deref() {
                Some(id) if open.iter().any(|o| o == id) => open.retain(|o| o != id),
                // A result with nothing to answer would also be refused; drop it.
                _ => continue,
            }
        }
        if message.role == ChatRole::Assistant {
            open = message.tool_calls.iter().map(|c| c.id.clone()).collect();
        }
        out.push(message);
    }
    for id in open {
        out.push(ChatMessage::tool_result(
            id,
            "No result: the turn was interrupted.",
        ));
    }
    out
}

/// Ask the model for a short title, and use it if one comes back.
async fn name_session(
    emitter: &Emitter<'_>,
    state: &AppState,
    session_id: &str,
    turn_id: &str,
    first_message: &str,
) -> Result<(), AgentError> {
    let source: String = first_message.chars().take(TITLE_SOURCE_MAX_CHARS).collect();
    let request = ChatRequest {
        model: state.model(),
        messages: vec![ChatMessage::new(
            ChatRole::User,
            format!("{TITLE_PROMPT}\n\n\"\"\"\n{source}\n\"\"\""),
        )],
        max_tokens: TITLE_MAX_TOKENS,
        tools: Vec::new(),
        thinking: None,
    };
    let mut text = String::new();
    let mut usage = Usage::default();
    let mut stream = state.llm.stream(request);
    while let Some(item) = stream.next().await {
        match item {
            Ok(StreamEvent::Text(t)) => text.push_str(&t),
            Ok(StreamEvent::Usage(u)) => {
                usage = Usage {
                    input_tokens: u.input_tokens,
                    output_tokens: u.output_tokens,
                    cost_usd: u.cost_usd,
                }
            }
            Ok(_) => {}
            Err(e) => {
                // The placeholder title stays; not worth failing anything over.
                tracing::debug!(%e, "title call failed");
                return Ok(());
            }
        }
    }
    let by = CallBy {
        model: state.model(),
        role: usage_role::CHAT,
        cached_tokens: 0,
    };
    record_usage(state, session_id, turn_id, usage, by).await?;
    let title = clean_title(&text);
    if title.is_empty() {
        return Ok(());
    }
    state
        .store
        .set_session_title(&state.user_id, session_id, &title)
        .await?;
    emitter
        .emit(Event::SessionUpdated {
            title: Some(title),
            mode: None,
            mode_limit: None,
            workspace: None,
        })
        .await
}

/// The model's title, tidied: first line, no quotes or markdown, no full stop, not too long.
fn clean_title(raw: &str) -> String {
    let line = raw
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or_default();
    let line = line
        .trim()
        .trim_start_matches(|c: char| c == '#' || c == '*' || c.is_whitespace())
        .trim_start_matches("Title:")
        .trim()
        .trim_matches(|c: char| matches!(c, '"' | '\'' | '*' | '`' | '“' | '”'))
        .trim_end_matches('.')
        .trim();
    // A long reply, or one in the first person, is the model answering instead of naming.
    let words = line.split_whitespace().count();
    let first = line.split_whitespace().next().unwrap_or_default();
    if words > TITLE_MAX_WORDS || matches!(first, "I" | "I'll" | "I'm" | "I've" | "Sure" | "Sure,")
    {
        return String::new();
    }
    shorten(line)
}

/// A placeholder title from the first message: its first line, shortened at a whole word.
fn title_from(text: &str) -> String {
    shorten(text.lines().next().unwrap_or_default().trim())
}

fn shorten(line: &str) -> String {
    if line.chars().count() <= SESSION_TITLE_MAX_CHARS {
        return line.to_owned();
    }
    let cut: String = line.chars().take(SESSION_TITLE_MAX_CHARS).collect();
    // Break at the last space, unless that would throw away most of the title (one huge word).
    let shortened = match cut.rfind(char::is_whitespace) {
        Some(i) if i >= cut.len() / 2 => &cut[..i],
        _ => cut.as_str(),
    };
    let mut title = shortened
        .trim_end_matches(|c: char| c.is_whitespace() || c.is_ascii_punctuation())
        .to_owned();
    title.push('…');
    title
}

/// Today's date (UTC) as YYYY-MM-DD, from the system clock.
pub(crate) fn today() -> String {
    let days = now_ms().div_euclid(86_400_000);
    // Civil-from-days (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

fn error_code(error: &LlmError) -> ErrorCode {
    match error {
        LlmError::Unauthorized { .. } => ErrorCode::ProviderKeyRejected,
        LlmError::RateLimited { .. } => ErrorCode::RateLimited,
        LlmError::Rejected { .. } | LlmError::OutOfCredit { .. } | LlmError::OverLimit { .. } => {
            ErrorCode::ProviderRejected
        }
        LlmError::KeyMissing { .. } => ErrorCode::ProviderKeyMissing,
        LlmError::ServerError { .. } | LlmError::Network { .. } | LlmError::BadResponse { .. } => {
            ErrorCode::ProviderUnavailable
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn questions_in_plan_mode_are_not_plans() {
        assert!(!looks_like_plan("What would you like changed in the plan?"));
        assert!(!looks_like_plan("   "));
        assert!(looks_like_plan("## Plan\n1. Add routes\n2. Add tests"));
        // A long plan that happens to end with a question is still a plan.
        let long = format!("{}Shall I go ahead?", "step. ".repeat(200));
        assert!(looks_like_plan(&long));
    }

    #[test]
    fn titles_are_first_line_and_short() {
        assert_eq!(title_from("fix the build\nplease"), "fix the build");
        let long = "x".repeat(SESSION_TITLE_MAX_CHARS + 5);
        assert_eq!(
            title_from(&long).chars().count(),
            SESSION_TITLE_MAX_CHARS + 1
        );
    }

    #[test]
    fn long_titles_end_at_a_whole_word() {
        let title = title_from(
            "In three short bullet points, explain what a Rust borrow checker does. Then end.",
        );
        assert_eq!(
            title,
            "In three short bullet points, explain what a Rust borrow…"
        );
    }

    #[test]
    fn model_titles_are_tidied() {
        assert_eq!(
            clean_title("\"Rust Borrow Checker Explained.\""),
            "Rust Borrow Checker Explained"
        );
        assert_eq!(
            clean_title("\n\n**Title:** Fixing the build\nmore"),
            "Fixing the build"
        );
        assert_eq!(clean_title("   "), "");
        assert_eq!(
            clean_title("I'll help you with that step by step. Let me start by listing"),
            ""
        );
    }

    #[test]
    fn today_is_a_plausible_date() {
        let d = today();
        assert_eq!(d.len(), 10);
        assert!(d.starts_with("20"));
    }

    #[test]
    fn missing_tool_results_are_filled_and_strays_dropped() {
        let mut asked = ChatMessage::new(ChatRole::Assistant, "");
        asked.tool_calls = vec![
            ToolCall {
                id: "a".into(),
                name: "list_dir".into(),
                arguments: "{}".into(),
            },
            ToolCall {
                id: "b".into(),
                name: "list_dir".into(),
                arguments: "{}".into(),
            },
        ];
        let fixed = repair_tool_results(vec![
            ChatMessage::tool_result("stray", "x"),
            asked,
            ChatMessage::tool_result("a", "ok"),
            ChatMessage::new(ChatRole::User, "next"),
        ]);
        let roles: Vec<ChatRole> = fixed.iter().map(|m| m.role).collect();
        assert_eq!(
            roles,
            vec![
                ChatRole::Assistant,
                ChatRole::Tool,
                ChatRole::Tool,
                ChatRole::User
            ]
        );
        assert_eq!(fixed[2].tool_call_id.as_deref(), Some("b"));
    }
}
