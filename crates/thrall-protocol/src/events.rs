//! Engine → client session events (protocol §7–§8, §12): turns, streamed messages, tool steps,
//! prompts, tasks, notices and errors.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::constants::PROTOCOL_VERSION;
use crate::requests::SessionState;

/// One frame the engine sends about a session: the envelope fields plus the event itself.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct EventFrame {
    /// Protocol major version.
    pub v: u32,
    /// Session the event belongs to.
    pub session: String,
    /// Position in the session's event log. Absent on live-only events (deltas), which are never
    /// stored: after a reconnect the client gets the matching `message.completed` instead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seq: Option<u64>,
    /// When it happened, Unix milliseconds.
    pub at: i64,
    /// The event.
    #[serde(flatten)]
    pub event: Event,
}

impl EventFrame {
    /// A frame for the current protocol version.
    pub fn new(session: impl Into<String>, seq: Option<u64>, at: i64, event: Event) -> Self {
        Self {
            v: PROTOCOL_VERSION,
            session: session.into(),
            seq,
            at,
            event,
        }
    }
}

/// Everything the engine can tell clients about a session (so far).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
pub enum Event {
    /// A turn began.
    #[serde(rename = "turn.started")]
    TurnStarted {
        /// Turn id.
        turn: String,
        /// Nobody started it: a scheduled job or a check-in (clients don't announce it as the
        /// person's own work finishing).
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        scheduled: bool,
    },
    /// A turn ended.
    #[serde(rename = "turn.completed")]
    TurnCompleted {
        /// Turn id.
        turn: String,
        /// How it ended.
        status: TurnStatus,
        /// Tokens and cost.
        usage: Usage,
    },
    /// A turn was taken back, with every turn after it (the person edited or retried their last
    /// message, or rewound): clients remove what it and the later turns showed.
    #[serde(rename = "turn.retracted")]
    TurnRetracted {
        /// Turn id.
        turn: String,
    },
    /// A message began (Thrall's reply, or an echo of the user's message for other clients).
    #[serde(rename = "message.started")]
    MessageStarted {
        /// Turn id.
        turn: String,
        /// Message id.
        message: String,
        /// Who wrote it.
        role: Role,
    },
    /// More text for a message being written. Live-only.
    #[serde(rename = "message.delta")]
    MessageDelta {
        /// Message id.
        message: String,
        /// Text to append.
        text: String,
    },
    /// More thinking text, for models that expose it. Live-only.
    #[serde(rename = "reasoning.delta")]
    ReasoningDelta {
        /// Message id.
        message: String,
        /// Text to append.
        text: String,
    },
    /// A message is finished; `text` is the full final text.
    #[serde(rename = "message.completed")]
    MessageCompleted {
        /// Message id.
        message: String,
        /// Full text; clients replace what they streamed with this.
        text: String,
        /// Files attached to it (user messages).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        files: Vec<FileRef>,
    },
    /// A tool call began (a collapsible row in clients).
    #[serde(rename = "step.started")]
    StepStarted {
        /// Turn id.
        turn: String,
        /// Step id.
        step: String,
        /// Tool name.
        tool: String,
        /// Plain-language title: "Reading src/main.rs".
        title: String,
        /// The helper's own step when a helper took this step: clients list it under that one.
        /// Several helpers can run at once, so their steps interleave.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        helper: Option<String>,
    },
    /// Live output from a running step (shell). Live-only; capped.
    #[serde(rename = "step.output")]
    StepOutput {
        /// Step id.
        step: String,
        /// Text to append.
        text: String,
    },
    /// A tool call finished.
    #[serde(rename = "step.completed")]
    StepCompleted {
        /// Step id.
        step: String,
        /// How it went.
        status: StepStatus,
        /// One-line plain-language result.
        summary: String,
        /// The change, for file edits.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        diff: Option<Diff>,
        /// Files the step made (pictures), fetched with `GET /v1/files/{file}`.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        files: Vec<FileRef>,
        /// Files the step wrote on the computer (a saved document, a picture's copy, a download):
        /// the desktop app opens them with `step.file.open`, other clients fetch them with
        /// `step.file.read`, both by their place in this list.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        saved: Vec<SavedFile>,
    },
    /// The session's task list changed (the whole list each time).
    #[serde(rename = "tasks.updated")]
    TasksUpdated {
        /// The tasks, in order.
        items: Vec<TaskItem>,
    },
    /// Thrall wants permission before doing something.
    #[serde(rename = "permission.request")]
    PermissionRequest {
        /// Request id; answers quote it.
        request: String,
        /// Turn id.
        turn: String,
        /// The step waiting on the answer.
        step: String,
        /// Tool name.
        tool: String,
        /// Plain language: "Run `cargo test`".
        summary: String,
        /// How risky.
        risk: Risk,
        /// The change, for file edits (accept is `yes`, reject is `no`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        diff: Option<Diff>,
        /// The answers on offer.
        choices: Vec<PermissionChoice>,
    },
    /// In plan mode, Thrall presents a plan and waits for a decision.
    #[serde(rename = "plan.proposed")]
    PlanProposed {
        /// Plan id; decisions quote it.
        plan: String,
        /// The plan.
        markdown: String,
    },
    /// A plan was approved or turned down.
    #[serde(rename = "plan.resolved")]
    PlanResolved {
        /// Plan id.
        plan: String,
        /// Whether it was approved.
        approved: bool,
        /// Which client decided.
        answered_by: String,
    },
    /// Thrall needs information it can't safely guess (not permission). One card holds every
    /// question of the turn, each with Thrall's recommended answer picked. Nobody answering by
    /// `expires_at` means the recommended answers are used.
    #[serde(rename = "question.ask")]
    QuestionAsk {
        /// Question id; answers quote it.
        question: String,
        /// The questions, in order.
        items: Vec<QuestionItem>,
        /// Why Thrall is asking instead of deciding, in a sentence.
        #[serde(default, skip_serializing_if = "String::is_empty")]
        why: String,
        /// When Thrall goes ahead with its recommended answers, Unix ms.
        expires_at: i64,
    },
    /// A question card was answered, or ran out of time.
    #[serde(rename = "question.resolved")]
    QuestionResolved {
        /// Question id.
        question: String,
        /// One answer per question, in order.
        answers: Vec<String>,
        /// Which client answered; `timeout` when nobody did and the recommended answers were used.
        answered_by: String,
    },
    /// A permission request was answered.
    #[serde(rename = "permission.resolved")]
    PermissionResolved {
        /// Request id.
        request: String,
        /// The answer.
        choice: PermissionChoice,
        /// Which client answered.
        answered_by: String,
    },
    /// Session details changed.
    #[serde(rename = "session.updated")]
    SessionUpdated {
        /// New title.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        /// New permission mode (`ask` · `auto_edits` · `plan` · `full_auto`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mode: Option<String>,
        /// How long Full auto lasts (sent with `mode: full_auto`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mode_limit: Option<crate::requests::ModeLimit>,
        /// New folder the session works in.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        workspace: Option<String>,
    },
    /// A background helper (or scheduled job) started.
    #[serde(rename = "agent.started")]
    AgentStarted {
        /// Helper id.
        agent: String,
        /// What it's doing.
        title: String,
    },
    /// A background helper (or scheduled job) finished.
    #[serde(rename = "agent.completed")]
    AgentCompleted {
        /// Helper id.
        agent: String,
        /// What it was doing.
        title: String,
        /// `done` · `failed`.
        status: String,
        /// One line about the result.
        summary: String,
    },
    /// What the session is doing, and the model in use; sent whenever either changes. Live-only:
    /// `session.opened` carries the current state for a client that (re)attaches.
    #[serde(rename = "status")]
    Status {
        /// Idle, working, or waiting for an answer.
        state: SessionState,
        /// Model the session's turns use.
        model: String,
    },
    /// Thrall saved a lasting fact about the person to long-term memory. Clients show it with a
    /// way to forget it (`memory.forget`).
    #[serde(rename = "memory.saved")]
    MemorySaved {
        /// The fact's id.
        fact: String,
        /// Its topic.
        topic: String,
        /// The fact.
        content: String,
    },
    /// Something worth telling the user that isn't a question.
    #[serde(rename = "notice")]
    Notice {
        /// How prominent.
        level: NoticeLevel,
        /// Plain-language text.
        text: String,
    },
    /// The owner asked for the newest version (`update.install`): the desktop app installs it.
    /// Sent only to the owner's desktop app, with an empty `session`.
    #[serde(rename = "update.requested")]
    UpdateRequested {},
    /// Open a file a step wrote, or show it in its folder (`step.file.open`): the desktop app does
    /// it. Sent only to that person's desktop app on this computer, with an empty `session`.
    #[serde(rename = "file.open")]
    FileOpen {
        /// The file's full path.
        path: String,
        /// Show it in its folder instead of opening it.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        reveal: bool,
    },
    /// The PCs asking to join changed (one asked, or its request ended): ask `pc.requests` for
    /// the list. Sent only to the owner's clients on this computer, with an empty `session`.
    #[serde(rename = "pc.asked")]
    PcAsked {},
    /// Work for a companion PC's runner (sent only to its link, with an empty `session`): run
    /// `job` (`{"phase": "check" | "run" | "stop" | "read" | "write", ...}`) in `workspace` for
    /// `person`'s `in_session`, and answer with `runner.done {call}`.
    #[serde(rename = "runner.call")]
    RunnerCall {
        /// The call's id.
        call: String,
        /// Whose session it is.
        person: String,
        /// The session.
        in_session: String,
        /// Its folder, on the PC.
        workspace: String,
        /// What to do.
        job: serde_json::Value,
    },
    /// Something failed.
    #[serde(rename = "error")]
    Error {
        /// Stable code clients switch on.
        code: ErrorCode,
        /// Plain-language explanation and what to do.
        message: String,
        /// Whether trying again may work.
        retry: bool,
    },
}

impl Event {
    /// The `type` string, as stored in the event log.
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::TurnStarted { .. } => "turn.started",
            Self::TurnCompleted { .. } => "turn.completed",
            Self::TurnRetracted { .. } => "turn.retracted",
            Self::MessageStarted { .. } => "message.started",
            Self::MessageDelta { .. } => "message.delta",
            Self::ReasoningDelta { .. } => "reasoning.delta",
            Self::MessageCompleted { .. } => "message.completed",
            Self::StepStarted { .. } => "step.started",
            Self::StepOutput { .. } => "step.output",
            Self::StepCompleted { .. } => "step.completed",
            Self::TasksUpdated { .. } => "tasks.updated",
            Self::PermissionRequest { .. } => "permission.request",
            Self::PermissionResolved { .. } => "permission.resolved",
            Self::PlanProposed { .. } => "plan.proposed",
            Self::PlanResolved { .. } => "plan.resolved",
            Self::QuestionAsk { .. } => "question.ask",
            Self::QuestionResolved { .. } => "question.resolved",
            Self::SessionUpdated { .. } => "session.updated",
            Self::AgentStarted { .. } => "agent.started",
            Self::AgentCompleted { .. } => "agent.completed",
            Self::Status { .. } => "status",
            Self::MemorySaved { .. } => "memory.saved",
            Self::Notice { .. } => "notice",
            Self::UpdateRequested {} => "update.requested",
            Self::FileOpen { .. } => "file.open",
            Self::PcAsked {} => "pc.asked",
            Self::RunnerCall { .. } => "runner.call",
            Self::Error { .. } => "error",
        }
    }

    /// Live-only events are sent but never stored and carry no `seq`.
    pub fn is_live_only(&self) -> bool {
        matches!(
            self,
            Self::MessageDelta { .. }
                | Self::ReasoningDelta { .. }
                | Self::StepOutput { .. }
                | Self::Status { .. }
        )
    }
}

/// One question on a `question.ask` card.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct QuestionItem {
    /// The question, in plain words.
    pub text: String,
    /// Answers to pick from (the person can always type their own instead).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<String>,
    /// Thrall's recommended answer: picked to start with, and used if nobody answers.
    pub recommended: String,
}

/// A file attached to a message. Clients fetch it with `GET /v1/files/{file}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FileRef {
    /// Upload id.
    pub file: String,
    /// File name.
    pub name: String,
    /// `image` · `document` · `text` · `other`, so clients know how to show it.
    pub kind: String,
    /// Size in bytes.
    pub size: u64,
}

/// A file a step wrote on the computer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SavedFile {
    /// Where it is, as a full path.
    pub path: String,
    /// Its name.
    pub name: String,
    /// Size in bytes, when it was written.
    pub size: u64,
}

/// How a step ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    /// It worked.
    Ok,
    /// It failed; `summary` says why.
    Error,
    /// It ran to the end but reported a problem (a command's exit code wasn't 0): the output may
    /// still be what was asked, and Thrall's reply says whether it was.
    Warn,
    /// The user said no.
    Denied,
    /// Stopped before finishing.
    Cancelled,
}

/// A file change (protocol §8).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Diff {
    /// File path as shown to the user.
    pub path: String,
    /// What happened to the file.
    pub kind: DiffKind,
    /// Standard unified diff.
    pub unified: String,
    /// Lines added.
    pub added: u32,
    /// Lines removed.
    pub removed: u32,
}

/// What a diff does to its file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DiffKind {
    /// New file.
    Create,
    /// Changed file.
    Modify,
    /// Removed file.
    Delete,
}

/// One entry in the task list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TaskItem {
    /// Task id.
    pub id: String,
    /// What it is.
    pub text: String,
    /// Where it's at.
    pub status: TaskStatus,
}

/// Task progress.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    /// Not started.
    Pending,
    /// Being worked on.
    InProgress,
    /// Finished.
    Done,
}

/// How risky an action is; clients colour permission cards by it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Risk {
    /// Harmless.
    Low,
    /// Worth a look.
    Medium,
    /// Could do damage.
    High,
}

/// An answer to a permission request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PermissionChoice {
    /// Go ahead, once.
    Yes,
    /// Don't.
    No,
    /// Go ahead, and don't ask again for this kind of action.
    Always,
}

/// How a turn ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TurnStatus {
    /// Finished normally.
    Done,
    /// The user stopped it.
    Interrupted,
    /// Something went wrong; an `error` event explains.
    Failed,
}

/// Who wrote a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// The person.
    User,
    /// Thrall.
    Assistant,
}

/// Notice prominence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum NoticeLevel {
    /// For information.
    Info,
    /// Worth attention.
    Warning,
}

/// Tokens and cost for a turn.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
pub struct Usage {
    /// Tokens sent to the model.
    pub input_tokens: u64,
    /// Tokens the model wrote.
    pub output_tokens: u64,
    /// Cost in US dollars, when the provider reports it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
}

/// Stable error codes (protocol §12).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// Bad or missing token.
    Unauthorized,
    /// Unknown request type or protocol version.
    Unsupported,
    /// Malformed request.
    Invalid,
    /// Session, turn, request or file doesn't exist.
    NotFound,
    /// Can't do that while a turn is running.
    Busy,
    /// Someone else answered that prompt first.
    AlreadyAnswered,
    /// No such slash command.
    UnknownCommand,
    /// Too many requests.
    RateLimited,
    /// The AI provider failed or timed out.
    ProviderUnavailable,
    /// No AI key set.
    ProviderKeyMissing,
    /// The AI provider didn't accept the key.
    ProviderKeyRejected,
    /// The AI provider refused the request (unknown model, too long, out of credit, …).
    ProviderRejected,
    /// Frame or file over the limit.
    TooLarge,
    /// Engine bug.
    Internal,
}
