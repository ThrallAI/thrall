//! Client → engine requests and the engine's direct replies (protocol §5–§7, §9–§10, §12).
//!
//! Every request carries an `id` chosen by the client; the engine answers each one exactly once
//! with a [`ReplyFrame`] quoting it in `re`: a typed result, an `ack`, or an `error`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::constants::PROTOCOL_VERSION;
use crate::events::{ErrorCode, PermissionChoice};

/// One frame a client sends.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RequestFrame {
    /// Protocol major version.
    pub v: u32,
    /// Request id, unique per connection; the reply quotes it as `re`.
    pub id: String,
    /// The request.
    #[serde(flatten)]
    pub request: Request,
}

impl RequestFrame {
    /// A frame for the current protocol version.
    pub fn new(id: impl Into<String>, request: Request) -> Self {
        Self {
            v: PROTOCOL_VERSION,
            id: id.into(),
            request,
        }
    }
}

/// Everything a client can ask the engine.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
pub enum Request {
    /// First frame on every connection. Answered by `welcome`.
    #[serde(rename = "hello")]
    Hello {
        /// Protocol majors the client speaks.
        protocols: Vec<u32>,
        /// Who is connecting.
        client: ClientInfo,
        /// Access token (local mode: read from the data folder).
        token: String,
    },
    /// Heartbeat. Answered by `pong`.
    #[serde(rename = "ping")]
    Ping {},
    /// Start a session. Answered by `session.opened`.
    #[serde(rename = "session.create")]
    SessionCreate {
        /// Folder the session works in.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        workspace: Option<String>,
        /// Title (Thrall names it after the first turn otherwise).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        /// Starting permission mode (`ask` · `auto_edits` · `plan` · `full_auto`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mode: Option<String>,
    },
    /// Attach to a session: `session.opened`, then every stored event after `after_seq` (all of
    /// them when absent), then live events.
    #[serde(rename = "session.open")]
    SessionOpen {
        /// Session id.
        session: String,
        /// Last `seq` the client already has.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        after_seq: Option<u64>,
    },
    /// Stop receiving a session's events; it keeps running. Answered by `ack`.
    #[serde(rename = "session.detach")]
    SessionDetach {
        /// Session id.
        session: String,
    },
    /// Sessions, newest first. Answered by `session.listed`.
    #[serde(rename = "session.list")]
    SessionList {
        /// How many.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        limit: Option<u32>,
        /// List archived sessions instead (most recently archived first).
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        archived: bool,
    },
    /// Archive a session (hidden from the list, kept in full) or bring it back. Answered by `ack`.
    #[serde(rename = "session.archive")]
    SessionArchive {
        /// Session id.
        session: String,
        /// True to archive, false to bring it back.
        archived: bool,
    },
    /// Rename a session. Answered by `ack`; attached clients get `session.updated`.
    #[serde(rename = "session.rename")]
    SessionRename {
        /// Session id.
        session: String,
        /// New title.
        title: String,
    },
    /// Change the permission mode. Answered by `ack`; attached clients get `session.updated`.
    #[serde(rename = "session.set_mode")]
    SessionSetMode {
        /// Session id.
        session: String,
        /// `ask` · `auto_edits` · `plan` · `full_auto`.
        mode: String,
        /// For `full_auto` only, how long it lasts: `task_end` (the default: back to Ask when the
        /// task finishes) · `time` (after `minutes`) · `manual` (until switched off). Full auto
        /// never lasts past a restart of Thrall.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        until: Option<String>,
        /// Minutes, for `until: time`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        minutes: Option<u32>,
    },
    /// Pin a session to the top of the list, or unpin it. Answered by `ack`.
    #[serde(rename = "session.pin")]
    SessionPin {
        /// Session id.
        session: String,
        /// True to pin.
        pinned: bool,
    },
    /// Put a session in one of the person's groups, or take it out (no `group`). Answered by
    /// `ack`.
    #[serde(rename = "session.move")]
    SessionMove {
        /// Session id.
        session: String,
        /// Group id; absent to take it out of its group.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        group: Option<String>,
    },
    /// Move a session to another working folder (one that exists). Answered by `ack`; attached
    /// clients get `session.updated`. Refused for Home and while Thrall is working in it.
    #[serde(rename = "session.set_folder")]
    SessionSetFolder {
        /// Session id.
        session: String,
        /// The folder's full path.
        folder: String,
    },
    /// The person's session groups, in order. Answered by `group.listed`.
    #[serde(rename = "group.list")]
    GroupList {},
    /// Add a group at the end. Answered by `group.listed`.
    #[serde(rename = "group.create")]
    GroupCreate {
        /// Its name.
        name: String,
    },
    /// Rename a group. Answered by `group.listed`.
    #[serde(rename = "group.rename")]
    GroupRename {
        /// Group id.
        group: String,
        /// New name.
        name: String,
    },
    /// Delete a group; its sessions stay, ungrouped. Answered by `group.listed`.
    #[serde(rename = "group.delete")]
    GroupDelete {
        /// Group id.
        group: String,
    },
    /// Put the groups in this order. Answered by `group.listed`.
    #[serde(rename = "group.reorder")]
    GroupReorder {
        /// Group ids, first to last.
        groups: Vec<String>,
    },
    /// Delete a session and its history. Clients ask the user first. Answered by `ack`.
    #[serde(rename = "session.delete")]
    SessionDelete {
        /// Session id.
        session: String,
    },
    /// Home, Thrall's built-in conversation: its current page (a fresh one starts when the last
    /// has had its day, per `home.fresh_start`) and earlier pages. Open the page with
    /// `session.open` like any session. Answered by `home.shown`.
    #[serde(rename = "home.get")]
    HomeGet {},
    /// Delete every Home page (refused while Thrall is working in Home). Clients ask the person
    /// first. Answered by `home.shown` with a fresh page.
    #[serde(rename = "home.clear")]
    HomeClear {},
    /// The check-in checklist and when the next check-in is due. Answered by `heartbeat.shown`.
    #[serde(rename = "heartbeat.get")]
    HeartbeatGet {},
    /// Replace the check-in checklist (empty removes it, which stops check-ins). Answered by
    /// `heartbeat.shown`.
    #[serde(rename = "heartbeat.set")]
    HeartbeatSet {
        /// The checklist, as markdown.
        checklist: String,
    },
    /// Check in now, whatever the hours. Answered by `ack` at once (or `error {invalid}` with no
    /// checklist); a message in Home follows if something needs the person, else a notice.
    #[serde(rename = "heartbeat.run")]
    HeartbeatRun {},
    /// Whether Thrall starts when the person signs in to Windows (the same entry as the desktop
    /// app's tray menu). Answered by `startup.shown`.
    #[serde(rename = "startup.get")]
    StartupGet {},
    /// Start Thrall at sign-in, or stop. Answered by `startup.shown` (or `error {invalid}` where
    /// it isn't available).
    #[serde(rename = "startup.set")]
    StartupSet {
        /// Start at sign-in.
        on: bool,
    },
    /// Install the newest version of Thrall: the desktop app on the engine's computer checks,
    /// downloads and installs it, and Thrall restarts. Answered by `ack` once the desktop app has
    /// been asked (or `error {invalid}` when it isn't running).
    #[serde(rename = "update.install")]
    UpdateInstall {},
    /// Where Thrall keeps its things on the engine's computer. Answered by `folders.shown`.
    #[serde(rename = "folders.get")]
    FoldersGet {},
    /// Open one of Thrall's folders in the file manager on the engine's computer (the working
    /// folder is made first if it isn't there yet). Answered by `ack`.
    #[serde(rename = "folder.open")]
    FolderOpen {
        /// Which folder.
        folder: ThrallFolder,
    },
    /// Your personality: how Thrall talks to you. Answered by personality.shown.
    #[serde(rename = "personality.get")]
    PersonalityGet {},
    /// Choose your personality (only ever your own). Answered by personality.shown.
    #[serde(rename = "personality.set")]
    PersonalitySet {
        /// A preset's id from personality.shown, or custom.
        preset: String,
        /// Your own words, used with custom.
        #[serde(default)]
        custom: String,
    },
    /// Send a message. Answered by ck; the turn's events follow. While a turn is running the
    /// text joins it at its next step instead.
    #[serde(rename = "turn.send")]
    TurnSend {
        /// Session id.
        session: String,
        /// What the user typed (`@path` mentions included).
        text: String,
        /// Ids of files uploaded with `POST /v1/files` to attach.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        files: Vec<String>,
    },
    /// What rewinding to before a turn would do: the message that started it and the files to put
    /// back. Answered by `checkpoint.shown`.
    #[serde(rename = "checkpoint.preview")]
    CheckpointPreview {
        /// Session id.
        session: String,
        /// The turn to go back to before.
        turn: String,
    },
    /// Go back to before a turn: its files and every later turn's go back to how they were
    /// (`files`), and/or the turn and every later one is taken back (`conversation`, one
    /// `turn.retracted` for `turn`). Refused while a turn runs. Answered by `turn.rewound`.
    #[serde(rename = "turn.rewind")]
    TurnRewind {
        /// Session id.
        session: String,
        /// The turn to go back to before.
        turn: String,
        /// Put files back.
        files: bool,
        /// Take the conversation back.
        conversation: bool,
        /// Files (as `checkpoint.shown` names them) to leave as they are.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        keep: Vec<String>,
    },
    /// Run the last message again: its turn is taken back (`turn.retracted`), the files it changed
    /// go back to how they were (except ones changed since by someone else, which a notice
    /// names) and the same text is sent as a new turn. Refused while a turn runs. Answered by
    /// `ack`.
    #[serde(rename = "turn.retry")]
    TurnRetry {
        /// Session id.
        session: String,
    },
    /// Replace the last message: its turn is taken back (files too, as for `turn.retry`) and
    /// `text` sent instead. Refused while a turn runs. Answered by `ack`.
    #[serde(rename = "turn.edit")]
    TurnEdit {
        /// Session id.
        session: String,
        /// The new message.
        text: String,
    },
    /// Stop the running turn (Esc). Answered by `ack`.
    #[serde(rename = "turn.interrupt")]
    TurnInterrupt {
        /// Session id.
        session: String,
    },
    /// Answer a `permission.request`. Answered by `ack`, or `error {already_answered}`.
    #[serde(rename = "permission.answer")]
    PermissionAnswer {
        /// Request id.
        request: String,
        /// The answer.
        choice: PermissionChoice,
    },
    /// Approve or turn down a `plan.proposed`. Answered by `ack`, or `error {already_answered}`.
    #[serde(rename = "plan.decide")]
    PlanDecide {
        /// Plan id.
        plan: String,
        /// Whether to go ahead.
        approve: bool,
        /// Approve and let file edits go ahead without asking.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        auto_edits: bool,
    },
    /// Answer a `question.ask`: one answer per question, in order. Answered by `ack`, or
    /// `error {already_answered}`.
    #[serde(rename = "question.answer")]
    QuestionAnswer {
        /// Question id.
        question: String,
        /// One answer per question.
        answers: Vec<String>,
    },
    /// Receive events from every session, not only the ones opened (the desktop app uses this for
    /// its tray icon and notifications). Live text (deltas, output) is left out. Answered by `ack`.
    #[serde(rename = "session.watch_all")]
    SessionWatchAll {},
    /// Stop the engine (the desktop app's Quit, when the person chooses to stop it too). Answered
    /// by `ack`; then every connection closes.
    #[serde(rename = "engine.stop")]
    EngineStop {},
    /// Turn an uploaded recording (`POST /v1/files`) into text: the app's microphone button.
    /// Answered by `audio.transcribed`.
    #[serde(rename = "audio.transcribe")]
    AudioTranscribe {
        /// The upload's id.
        file: String,
        /// Session the cost belongs to, if any.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session: Option<String>,
    },
    /// The slash commands, for autocomplete. Answered by `command.listed`.
    #[serde(rename = "command.list")]
    CommandList {},
    /// Run a slash command. Answered by `command.result`.
    #[serde(rename = "command.run")]
    CommandRun {
        /// Session it applies to, if any.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session: Option<String>,
        /// Name without the slash.
        name: String,
        /// Everything after the name.
        #[serde(default)]
        args: String,
    },
    /// The lessons in your experience log, newest first. Answered by `experience.listed`.
    #[serde(rename = "experience.list")]
    ExperienceList {},
    /// Delete a lesson from your experience log. Answered by `experience.listed`.
    #[serde(rename = "experience.forget")]
    ExperienceForget {
        /// The lesson's id.
        lesson: String,
    },
    /// The facts Thrall remembers about the person, newest first. Answered by `memory.listed`.
    #[serde(rename = "memory.list")]
    MemoryList {},
    /// Forget a fact: it's no longer recalled. Answered by `ack`, or `error {not_found}`.
    #[serde(rename = "memory.forget")]
    MemoryForget {
        /// The fact's id.
        fact: String,
    },
    /// Remove one of the person's own skills (its folder is deleted). Project skills can only be
    /// turned off. Answered by `settings.shown`.
    #[serde(rename = "skill.remove")]
    SkillRemove {
        /// The skill's name.
        name: String,
    },
    /// One of the person's own skills in full, to read or edit. Answered by `skill.shown`, or
    /// `error {invalid}` when there's no such skill of theirs.
    #[serde(rename = "skill.get")]
    SkillGet {
        /// The skill's name.
        name: String,
    },
    /// Change one of the person's own skills. A skill Thrall learned is rated again from what it
    /// now says (the person wrote it, so only what it does can raise the rating). Answered by
    /// `skill.shown`, or `error {invalid}` with the reason.
    #[serde(rename = "skill.edit")]
    SkillEdit {
        /// The skill's name (it keeps it).
        name: String,
        /// When to use it, one sentence.
        description: String,
        /// The instructions (Markdown).
        instructions: String,
    },
    /// Ask the model for a safer version of one of the person's own skills: the same job without
    /// deleting, sending, handling keys or running downloads on its own (it asks the person
    /// instead). Nothing is saved: `skill.edit` with the answer keeps it. Answered by
    /// `skill.safer`, or `error {invalid}` / `error {provider_unavailable}`.
    #[serde(rename = "skill.make_safe")]
    SkillMakeSafe {
        /// The skill's name.
        name: String,
    },
    /// The engine's settings, for the settings screen (read only for now). Key values never
    /// leave the engine: only whether each one is set. Answered by `settings.shown`.
    #[serde(rename = "settings.get")]
    SettingsGet {},
    /// Change one setting in `config.toml` (comments kept). Used at once when Thrall is idle,
    /// otherwise when its work finishes (`pending` in the answer). Answered by `settings.shown`,
    /// or `error {invalid}` with the reason (unknown setting, wrong type).
    #[serde(rename = "settings.set")]
    SettingsSet {
        /// Dotted key, e.g. `agent.verify` or `provider.model`.
        key: String,
        /// The new value.
        value: SettingValue,
    },
    /// Check that a supplier works with the key Thrall has for it (the key never leaves the
    /// engine), and list the models it offers. The owner only. Answered by `provider.tested`.
    #[serde(rename = "provider.test")]
    ProviderTest {
        /// `openrouter`, `chatgpt`, `local`, `fal`, or a supplier reached directly (`openai`,
        /// `anthropic`, `google`, `elevenlabs`).
        supplier: String,
    },
    /// Look on this computer for programs and model servers that can do Thrall's jobs (Ollama,
    /// LM Studio, whisper.cpp, Piper, a Stable Diffusion web UI). Nothing is installed or
    /// downloaded. The owner only. Answered by `local.found`.
    #[serde(rename = "local.find")]
    LocalFind {},
    /// Paired phones and the phone link's state. Not on the phone link itself. Answered by
    /// `phone.listed`.
    #[serde(rename = "phone.list")]
    PhoneList {},
    /// Start pairing a phone: a one-time code shown as a QR. Needs `remote.address`. Not on the
    /// phone link itself. Answered by `phone.pairing`.
    #[serde(rename = "phone.pair")]
    PhonePair {
        /// Who the phone is for (a person's id); absent: the owner.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        person: Option<String>,
    },
    /// Give a paired phone to another person: it reconnects as them. Not on the phone link
    /// itself. Answered by `phone.listed`.
    #[serde(rename = "phone.assign")]
    PhoneAssign {
        /// The phone's id.
        phone: String,
        /// The person's id.
        person: String,
    },
    /// Forget a phone: it can't connect again, and its open connection closes. Not on the phone
    /// link itself. Answered by `phone.listed`.
    #[serde(rename = "phone.remove")]
    PhoneRemove {
        /// The phone's id.
        phone: String,
    },
    /// The people who use this Thrall (anyone but the owner: only themselves). Answered by
    /// `people.listed`.
    #[serde(rename = "people.list")]
    PeopleList {
        /// Count each person's spending from this moment (Unix ms), e.g. the start of this month
        /// where the asker is; absent: all of it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        since: Option<i64>,
    },
    /// Add a person (a member). Owner only. Answered by `people.listed`.
    #[serde(rename = "people.add")]
    PeopleAdd {
        /// Their name.
        name: String,
    },
    /// Rename a person. Owner only. Answered by `people.listed`.
    #[serde(rename = "people.rename")]
    PeopleRename {
        /// Their id.
        person: String,
        /// Their new name.
        name: String,
    },
    /// Set what a person's turns run on and may spend. Owner only. Answered by `people.listed`.
    #[serde(rename = "people.terms")]
    PeopleTerms {
        /// Their id.
        person: String,
        /// Their turns use their own AI key (not the owner's). Not for the owner.
        #[serde(default)]
        own_key: bool,
        /// Their own model; absent: the engine's. Not for the owner (who changes the engine's).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model: Option<String>,
        /// The most their turns may cost in a calendar month, in US dollars; absent: no limit.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        monthly_limit: Option<f64>,
    },
    /// Keep (or, without `key`, forget) a person's own AI key. Yours, or (the owner) anyone's;
    /// never shown back. The owner's own key is set with `thrall key set`. Answered by `ack`.
    #[serde(rename = "people.key")]
    PeopleKey {
        /// Whose; absent: yours.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        person: Option<String>,
        /// The key; absent or empty: forget it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        key: Option<String>,
    },
    /// Remove a person: they can't connect any more and their phones are forgotten; their
    /// sessions and files are kept. The owner can't be removed. Owner only. Answered by
    /// `people.listed`.
    #[serde(rename = "people.remove")]
    PeopleRemove {
        /// Their id.
        person: String,
    },
    /// A one-time sign-in link for a person: the browser that opens it signs in as them. Owner
    /// only. Answered by `signin.code`.
    #[serde(rename = "signin.create")]
    SigninCreate {
        /// Their id.
        person: String,
    },
    /// Browsers and PCs signed in as a person: your own, or (the owner) anyone's. Answered by
    /// `device.listed`.
    #[serde(rename = "device.list")]
    DeviceList {
        /// Whose; absent: yours.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        person: Option<String>,
    },
    /// Sign a browser or PC out: its token stops working and its open connections close. Your
    /// own, or (the owner) anyone's. Answered by `device.listed`.
    #[serde(rename = "device.remove")]
    DeviceRemove {
        /// The device's id.
        device: String,
        /// Whose; absent: yours.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        person: Option<String>,
    },
    /// Send a file in pieces over the socket (how phones attach files). The first piece starts a
    /// new upload; later pieces name it and are added to the end. Each piece's `data` is base64,
    /// at most `FILE_CHUNK_BYTES` before encoding. Answered by `file.saved`.
    #[serde(rename = "file.put")]
    FilePut {
        /// The upload to add to (from an earlier `file.saved`); absent for the first piece.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        upload: Option<String>,
        /// The file's name (used with the first piece).
        #[serde(default)]
        name: String,
        /// This piece, base64.
        data: String,
    },
    /// Folders a new session could work in: ones sessions already use, and Thrall's own default.
    /// Answered by `folder.places`.
    #[serde(rename = "folder.places")]
    FolderPlaces {},
    /// List a folder inside a session's folder (read only). Answered by `folder.listed`.
    #[serde(rename = "folder.list")]
    FolderList {
        /// The session whose folder it is.
        session: String,
        /// Path inside it; empty for the folder itself.
        #[serde(default)]
        path: String,
    },
    /// Read a file inside a session's folder, a piece at a time (read only). Answered by
    /// `folder.part`.
    #[serde(rename = "folder.read")]
    FolderRead {
        /// The session whose folder it is.
        session: String,
        /// Path inside it.
        path: String,
        /// Where to start, in bytes.
        #[serde(default)]
        offset: u64,
    },
    /// Read a file attached to a message, a piece at a time. Answered by `file.part`.
    #[serde(rename = "file.get")]
    FileGet {
        /// The file's id.
        file: String,
        /// Where to start, in bytes.
        #[serde(default)]
        offset: u64,
    },
    /// Make a new, empty folder for sessions to work in. `parent` must be one `folder.places`
    /// offers; `name` is a plain folder name (no `\`, `/` or `..`). Never replaces a folder that's
    /// already there. Answered by `folder.created`.
    #[serde(rename = "folder.create")]
    FolderCreate {
        /// Where to make it (from `folder.places`).
        parent: String,
        /// Its name.
        name: String,
    },
    /// The newest phone app this engine holds, to offer as an update. Answered by `app.latest`.
    #[serde(rename = "app.latest")]
    AppLatest {},
    /// Read that phone app, a piece at a time. Answered by `file.part` (`file` is `app`).
    #[serde(rename = "app.get")]
    AppGet {
        /// Where to start, in bytes.
        #[serde(default)]
        offset: u64,
    },
}

impl Request {
    /// The `type` string.
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::Hello { .. } => "hello",
            Self::Ping {} => "ping",
            Self::SessionCreate { .. } => "session.create",
            Self::SessionOpen { .. } => "session.open",
            Self::SessionDetach { .. } => "session.detach",
            Self::SessionList { .. } => "session.list",
            Self::SessionArchive { .. } => "session.archive",
            Self::SessionRename { .. } => "session.rename",
            Self::SessionSetMode { .. } => "session.set_mode",
            Self::SessionPin { .. } => "session.pin",
            Self::SessionMove { .. } => "session.move",
            Self::SessionSetFolder { .. } => "session.set_folder",
            Self::GroupList {} => "group.list",
            Self::GroupCreate { .. } => "group.create",
            Self::GroupRename { .. } => "group.rename",
            Self::GroupDelete { .. } => "group.delete",
            Self::GroupReorder { .. } => "group.reorder",
            Self::SessionDelete { .. } => "session.delete",
            Self::HomeGet {} => "home.get",
            Self::HomeClear {} => "home.clear",
            Self::HeartbeatGet {} => "heartbeat.get",
            Self::HeartbeatSet { .. } => "heartbeat.set",
            Self::HeartbeatRun {} => "heartbeat.run",
            Self::StartupGet {} => "startup.get",
            Self::StartupSet { .. } => "startup.set",
            Self::UpdateInstall {} => "update.install",
            Self::FoldersGet {} => "folders.get",
            Self::FolderOpen { .. } => "folder.open",
            Self::PersonalityGet {} => "personality.get",
            Self::PersonalitySet { .. } => "personality.set",
            Self::TurnSend { .. } => "turn.send",
            Self::CheckpointPreview { .. } => "checkpoint.preview",
            Self::TurnRewind { .. } => "turn.rewind",
            Self::TurnRetry { .. } => "turn.retry",
            Self::TurnEdit { .. } => "turn.edit",
            Self::TurnInterrupt { .. } => "turn.interrupt",
            Self::PermissionAnswer { .. } => "permission.answer",
            Self::PlanDecide { .. } => "plan.decide",
            Self::QuestionAnswer { .. } => "question.answer",
            Self::SessionWatchAll {} => "session.watch_all",
            Self::EngineStop {} => "engine.stop",
            Self::AudioTranscribe { .. } => "audio.transcribe",
            Self::CommandList {} => "command.list",
            Self::CommandRun { .. } => "command.run",
            Self::ExperienceList {} => "experience.list",
            Self::ExperienceForget { .. } => "experience.forget",
            Self::MemoryList {} => "memory.list",
            Self::MemoryForget { .. } => "memory.forget",
            Self::SkillRemove { .. } => "skill.remove",
            Self::SkillGet { .. } => "skill.get",
            Self::SkillEdit { .. } => "skill.edit",
            Self::SkillMakeSafe { .. } => "skill.make_safe",
            Self::SettingsGet {} => "settings.get",
            Self::SettingsSet { .. } => "settings.set",
            Self::ProviderTest { .. } => "provider.test",
            Self::LocalFind {} => "local.find",
            Self::PhoneList {} => "phone.list",
            Self::PhonePair { .. } => "phone.pair",
            Self::PhoneAssign { .. } => "phone.assign",
            Self::PhoneRemove { .. } => "phone.remove",
            Self::PeopleList { .. } => "people.list",
            Self::PeopleAdd { .. } => "people.add",
            Self::PeopleRename { .. } => "people.rename",
            Self::PeopleRemove { .. } => "people.remove",
            Self::PeopleTerms { .. } => "people.terms",
            Self::PeopleKey { .. } => "people.key",
            Self::SigninCreate { .. } => "signin.create",
            Self::DeviceList { .. } => "device.list",
            Self::DeviceRemove { .. } => "device.remove",
            Self::FilePut { .. } => "file.put",
            Self::FileGet { .. } => "file.get",
            Self::FolderPlaces {} => "folder.places",
            Self::FolderList { .. } => "folder.list",
            Self::FolderRead { .. } => "folder.read",
            Self::FolderCreate { .. } => "folder.create",
            Self::AppLatest {} => "app.latest",
            Self::AppGet { .. } => "app.get",
        }
    }
}

/// Who is connecting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ClientInfo {
    /// What sort of client.
    pub kind: ClientKind,
    /// Display name ("Thrall Terminal").
    pub name: String,
    /// Client version.
    pub version: String,
}

/// Sorts of client; `answered_by` names them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ClientKind {
    /// The terminal app.
    Tui,
    /// A browser.
    Web,
    /// The desktop app.
    Desktop,
    /// The Android app.
    Android,
    /// Telegram.
    Telegram,
    /// Scripts (`thrall run`).
    Headless,
}

impl ClientKind {
    /// The wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Tui => "tui",
            Self::Web => "web",
            Self::Desktop => "desktop",
            Self::Android => "android",
            Self::Telegram => "telegram",
            Self::Headless => "headless",
        }
    }
}

/// One frame the engine sends in answer to a request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ReplyFrame {
    /// Protocol major version.
    pub v: u32,
    /// The request id this answers.
    pub re: String,
    /// The answer.
    #[serde(flatten)]
    pub reply: Reply,
}

impl ReplyFrame {
    /// A frame for the current protocol version.
    pub fn new(re: impl Into<String>, reply: Reply) -> Self {
        Self {
            v: PROTOCOL_VERSION,
            re: re.into(),
            reply,
        }
    }

    /// An `error` reply.
    pub fn error(re: impl Into<String>, code: ErrorCode, message: impl Into<String>) -> Self {
        Self::new(
            re,
            Reply::Error {
                code,
                message: message.into(),
                retry: false,
            },
        )
    }
}

/// The engine's direct answers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
pub enum Reply {
    /// Answer to `hello`.
    #[serde(rename = "welcome")]
    Welcome {
        /// The protocol major both sides will speak.
        protocol: u32,
        /// About the engine.
        engine: EngineInfo,
        /// Who the engine acts for.
        user: UserInfo,
        /// Features this engine has, so clients don't guess.
        capabilities: Vec<String>,
        /// Limits the client must respect.
        limits: Limits,
    },
    /// Answer to `ping`.
    #[serde(rename = "pong")]
    Pong {},
    /// Done; nothing to return.
    #[serde(rename = "ack")]
    Ack {},
    /// The request failed.
    #[serde(rename = "error")]
    Error {
        /// Stable code clients switch on.
        code: ErrorCode,
        /// Plain-language explanation and what to do.
        message: String,
        /// Whether trying again may work.
        retry: bool,
    },
    /// A session is open on this connection.
    #[serde(rename = "session.opened")]
    SessionOpened(SessionInfo),
    /// Answer to `session.list`.
    #[serde(rename = "session.listed")]
    SessionListed {
        /// Newest first.
        items: Vec<SessionInfo>,
    },
    /// The person's session groups, in order (answers every `group.*` request).
    #[serde(rename = "group.listed")]
    GroupListed {
        /// The groups.
        items: Vec<GroupInfo>,
    },
    /// Answer to `home.get` and `home.clear`.
    #[serde(rename = "home.shown")]
    HomeShown {
        /// The page to use now.
        page: SessionInfo,
        /// Earlier pages, newest first.
        earlier: Vec<SessionInfo>,
    },
    /// Answer to `heartbeat.get` and `heartbeat.set`.
    #[serde(rename = "heartbeat.shown")]
    HeartbeatShown {
        /// The checklist ("" when there isn't one).
        checklist: String,
        /// When the next check-in is due, Unix ms (absent while check-ins are off).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        next_run: Option<i64>,
    },
    /// Answer to personality.get and personality.set.
    #[serde(rename = "personality.shown")]
    PersonalityShown {
        /// The chosen preset's id, or custom.
        preset: String,
        /// Your own words (kept even while a preset is chosen).
        custom: String,
        /// The presets on offer, standard first.
        presets: Vec<PersonalityPreset>,
    },
    /// Answer to startup.get and startup.set.
    #[serde(rename = "startup.shown")]
    StartupShown {
        /// This system can start Thrall at sign-in (the desktop app is installed beside the engine).
        available: bool,
        /// Thrall starts at sign-in.
        on: bool,
        /// What the setting is called on the engine's system ("Start with Windows", "Open at
        /// login", "Start when I log in"); empty from an older engine.
        #[serde(default)]
        label: String,
    },
    /// Answer to `folders.get`.
    #[serde(rename = "folders.shown")]
    FoldersShown {
        /// The data folder: settings, the database, logs (`%APPDATA%\Thrall` on Windows).
        data: String,
        /// Thrall's working folder, where Home and new folders go (`Documents\Thrall`), if this
        /// computer has one.
        work: Option<String>,
    },
    /// Answer to `command.list`.
    #[serde(rename = "command.listed")]
    CommandListed {
        /// The commands.
        items: Vec<CommandItem>,
    },
    /// Answer to `command.run`.
    #[serde(rename = "command.result")]
    CommandResult {
        /// What to show.
        markdown: String,
    },
    /// What a recording says (answers `audio.transcribe`).
    #[serde(rename = "audio.transcribed")]
    AudioTranscribed {
        /// The words.
        text: String,
    },
    /// Answer to `checkpoint.preview`.
    #[serde(rename = "checkpoint.shown")]
    CheckpointShown {
        /// The turn.
        turn: String,
        /// What the user typed to start it (their messages in it, joined).
        text: String,
        /// The files a rewind would put back, in the order they were first changed.
        files: Vec<RewindFile>,
    },
    /// Answer to `turn.rewind`.
    #[serde(rename = "turn.rewound")]
    TurnRewound {
        /// What the user typed to start the turn, to put back in the message box.
        text: String,
        /// Files put back (or removed, if the turn made them).
        restored: Vec<String>,
        /// Files that couldn't be put back, each with the reason.
        failed: Vec<String>,
    },
    /// Answer to `experience.list` and `experience.forget`.
    #[serde(rename = "experience.listed")]
    ExperienceListed {
        /// Newest first.
        items: Vec<LessonInfo>,
    },
    /// Answer to `skill.get` and `skill.edit`.
    #[serde(rename = "skill.shown")]
    SkillShown {
        /// The skill as saved.
        skill: SkillDetail,
    },
    /// Answer to `skill.make_safe`: a safer version, not saved.
    #[serde(rename = "skill.safer")]
    SkillSafer {
        /// The skill's name.
        name: String,
        /// When to use it.
        description: String,
        /// The rewritten instructions.
        instructions: String,
        /// What this version would be rated: `low`, `medium` or `high`.
        rating: String,
        /// Why, when it isn't low.
        why: Vec<String>,
    },
    /// Answer to `memory.list`.
    #[serde(rename = "memory.listed")]
    MemoryListed {
        /// Newest first.
        facts: Vec<FactInfo>,
    },
    /// Answer to `settings.get`.
    #[serde(rename = "settings.shown")]
    SettingsShown {
        /// `config.toml` as JSON, with values that could be secrets (MCP server environment
        /// variables that aren't keychain references) replaced by `"••••"`.
        config: serde_json::Value,
        /// Where the file is, if the engine has a data folder.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        config_file: Option<String>,
        /// The model turns use now (`/model` can differ from the file until restart).
        model: String,
        /// Keys Thrall knows about and whether each is stored.
        keys: Vec<KeyStatus>,
        /// Chat models the person's ChatGPT plan offers (`chatgpt/<model>`), when signed in.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        plan_models: Vec<String>,
        /// Skills Thrall can use (in the session folder `settings.get` was asked about, or only
        /// the person's own), including ones turned off.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        skills: Vec<SkillInfo>,
        /// Saved settings not in use yet: they apply when Thrall finishes what it's doing.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        pending: bool,
        /// MCP servers on the web that Thrall is signed in to (`/mcp login <name>`).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        mcp_signed_in: Vec<String>,
        /// Helper types Thrall can hand work to: its own and the person's (`agents/<name>.md`).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        helper_types: Vec<HelperTypeInfo>,
    },
    /// Answer to `local.find`.
    #[serde(rename = "local.found")]
    LocalFound {
        /// What was found, and what wasn't, job by job.
        items: Vec<LocalFound>,
        /// The folder local models go in (whisper.cpp models in `whisper/`, Piper voices in
        /// `piper/`), when Thrall has a data folder.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        models_dir: Option<String>,
    },
    /// Answer to `provider.test`.
    #[serde(rename = "provider.tested")]
    ProviderTested {
        /// The supplier tested.
        supplier: String,
        /// It answered and took the key.
        ok: bool,
        /// What happened, in plain words (credit left, or what's wrong and how to fix it).
        message: String,
        /// Models it offers, as Thrall names them (`direct/openai/gpt-5`, `local/llama3.1`), when
        /// it lists them.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        models: Vec<String>,
    },
    /// Answer to `phone.list` and `phone.remove`.
    #[serde(rename = "phone.listed")]
    PhoneListed {
        /// Paired phones, newest first.
        items: Vec<PhoneInfo>,
        /// The phone link.
        remote: RemoteInfo,
    },
    /// Answer to `phone.pair`: what the phone scans.
    #[serde(rename = "phone.pairing")]
    PhonePairing {
        /// The pairing link the QR holds (`thrall://pair?…`); it contains a secret, so it's shown
        /// only on this computer.
        link: String,
        /// The QR code's rows, top to bottom: `1` dark, `0` light (no quiet zone).
        qr: Vec<String>,
        /// When the code stops working, Unix ms.
        expires: i64,
    },
    /// Answer to the `people.*` requests.
    #[serde(rename = "people.listed")]
    PeopleListed {
        /// Everyone, the owner first, then oldest first.
        items: Vec<PersonInfo>,
    },
    /// Answer to `signin.create`. The link is the web app's address with `#signin=<code>`.
    #[serde(rename = "signin.code")]
    SigninCode {
        /// The one-time code (it signs someone in: show it only to them).
        code: String,
        /// Who it signs in as.
        name: String,
        /// When it stops working, Unix ms.
        expires: i64,
    },
    /// Answer to `device.list` and `device.remove`.
    #[serde(rename = "device.listed")]
    DeviceListed {
        /// Whose devices.
        person: String,
        /// Signed-in browsers and PCs, newest first.
        items: Vec<DeviceInfo>,
    },
    /// Answer to `file.put`: the upload so far.
    #[serde(rename = "file.saved")]
    FileSaved {
        /// Its id, for `turn.send {files}` and further pieces.
        file: String,
        /// The name it was saved under.
        name: String,
        /// Bytes so far.
        size: u64,
    },
    /// Answer to `folder.places`.
    #[serde(rename = "folder.places")]
    FolderPlaces {
        /// Folders sessions use, most recently active first.
        recent: Vec<String>,
        /// Thrall's own folder for sessions started without one picked (`Documents\Thrall`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        default: Option<String>,
        /// Where a new folder goes unless another is picked (`folders.new_in`, else `default`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        new_in: Option<String>,
    },
    /// Answer to `folder.create`.
    #[serde(rename = "folder.created")]
    FolderCreated {
        /// The new folder's full path.
        path: String,
    },
    /// Answer to `app.latest`.
    #[serde(rename = "app.latest")]
    AppLatest {
        /// The phone app this engine holds; absent when it holds none.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        release: Option<AppRelease>,
    },
    /// Answer to `folder.list`.
    #[serde(rename = "folder.listed")]
    FolderListed {
        /// The session.
        session: String,
        /// The folder, relative to the session's folder ("" for the folder itself).
        path: String,
        /// Folders first, then files, each newest first.
        items: Vec<FolderEntry>,
        /// Some entries were left out (a very full folder).
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        more: bool,
    },
    /// Answer to `folder.read`: one piece of a file in a session's folder.
    #[serde(rename = "folder.part")]
    FolderPart {
        /// The session.
        session: String,
        /// The file, relative to the session's folder.
        path: String,
        /// Where this piece starts.
        offset: u64,
        /// The whole file's size; the last piece ends there.
        size: u64,
        /// This piece, base64.
        data: String,
    },
    /// Answer to `file.get`: one piece of a file.
    #[serde(rename = "file.part")]
    FilePart {
        /// The file's id.
        file: String,
        /// Its name.
        name: String,
        /// Where this piece starts.
        offset: u64,
        /// The whole file's size; the last piece ends there.
        size: u64,
        /// This piece, base64.
        data: String,
    },
}

/// One entry in a folder listing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FolderEntry {
    /// Its name.
    pub name: String,
    /// It's a folder.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub folder: bool,
    /// Size in bytes (0 for folders).
    #[serde(default)]
    pub size: u64,
    /// Last changed, Unix ms.
    #[serde(default)]
    pub modified: i64,
}

/// A build of the phone app, offered as an update.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AppRelease {
    /// Android's version number: higher is newer.
    pub version_code: u32,
    /// The version people see ("0.2.0").
    pub version_name: String,
    /// The APK's size in bytes.
    pub size: u64,
    /// The APK's SHA-256, hex, to check the download.
    pub sha256: String,
    /// What's new, a line or a few.
    #[serde(default)]
    pub notes: String,
}

/// A paired phone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PhoneInfo {
    /// Its id.
    pub phone: String,
    /// The name it gave when pairing.
    pub name: String,
    /// When it was paired, Unix ms.
    pub added: i64,
    /// When it last connected, Unix ms (absent if never since pairing).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_seen: Option<i64>,
    /// Connected right now.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub connected: bool,
    /// Whose phone it is (a person's id).
    #[serde(default)]
    pub person: String,
}

/// A person who uses this Thrall.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PersonInfo {
    /// Their id.
    pub person: String,
    /// Their name.
    pub name: String,
    /// `owner` (runs this engine) or `member`.
    pub role: String,
    /// When they were added, Unix ms.
    pub added: i64,
    /// How many phones are paired to them.
    #[serde(default)]
    pub phones: u32,
    /// What their turns have cost (USD) since `people.list`'s `since`. Only what was recorded
    /// with a price counts.
    #[serde(default)]
    pub spent: f64,
    /// Their turns use their own AI key.
    #[serde(default)]
    pub own_key: bool,
    /// Their own AI key is set (it's never shown).
    #[serde(default)]
    pub key_set: bool,
    /// Their own model, if they have one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Their monthly spending limit in US dollars, if they have one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub monthly_limit: Option<f64>,
}

/// A browser or PC signed in as a person.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DeviceInfo {
    /// Its id.
    pub device: String,
    /// The name it signed in under.
    pub name: String,
    /// When it signed in, Unix ms.
    pub added: i64,
    /// When it last connected, Unix ms.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_seen: Option<i64>,
    /// The one asking.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub current: bool,
}

/// The phone link's state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RemoteInfo {
    /// Switched on in settings (`remote.enabled`).
    pub enabled: bool,
    /// The public address phones use (`remote.address`; "" when not set).
    pub address: String,
    /// The listener is running.
    pub listening: bool,
    /// Its port on 127.0.0.1 (what the tunnel forwards to).
    pub port: u16,
}

/// A fact Thrall remembers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FactInfo {
    /// Its id.
    pub fact: String,
    /// Its topic.
    pub topic: String,
    /// The fact.
    pub content: String,
    /// When it was saved, Unix ms.
    pub at: i64,
}

/// A personality preset, for the settings screen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PersonalityPreset {
    /// Its id (personality.set {preset}).
    pub id: String,
    /// Its name.
    pub name: String,
    /// One line about it.
    pub description: String,
}

/// A lesson in the experience log, for the settings screen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LessonInfo {
    /// Its id.
    pub lesson: String,
    /// The lesson.
    pub text: String,
    /// When it was noted, Unix ms.
    pub at: i64,
}

/// A skill, for the settings screen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SkillInfo {
    /// Its name (`/name` runs it).
    pub name: String,
    /// When to use it.
    pub description: String,
    /// One of the person's own (`false`: the project's).
    pub personal: bool,
    /// It runs in a helper.
    pub helper: bool,
    /// It's turned on.
    pub enabled: bool,
    /// Safety rating of a skill Thrall learned: `low`, `medium` or `high` (none for ones
    /// written by hand).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rating: Option<String>,
}

/// One of the person's own skills in full.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SkillDetail {
    /// Its name.
    pub name: String,
    /// When to use it.
    pub description: String,
    /// The instructions (Markdown).
    pub instructions: String,
    /// It runs in a helper.
    pub helper: bool,
    /// It's turned on.
    pub enabled: bool,
    /// Safety rating of a skill Thrall learned (none for ones written by hand).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rating: Option<String>,
}

/// A helper type, for the settings screen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct HelperTypeInfo {
    /// Its name (`agent {type}`).
    pub name: String,
    /// What it's for.
    pub description: String,
    /// It only looks (works in plan mode).
    pub read_only: bool,
    /// Its own model, if it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// `built_in` or `personal`.
    pub source: String,
}

/// A file a rewind would put back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RewindFile {
    /// The file, relative to the session folder when inside it.
    pub path: String,
    /// What happens to it.
    pub action: RewindAction,
    /// Someone changed it after Thrall last did (or it's gone): putting it back loses that.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub changed: bool,
}

/// One of Thrall's folders.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ThrallFolder {
    /// The data folder: settings, the database, logs.
    Data,
    /// The working folder: Home and new folders.
    Work,
}

/// What a rewind does to a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RewindAction {
    /// Its earlier content goes back.
    Restore,
    /// Thrall made it, so it's removed.
    Delete,
    /// It was too big to keep a copy of, so it stays as it is.
    Unavailable,
}

/// A setting's value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum SettingValue {
    /// Yes or no.
    Bool(bool),
    /// A whole number.
    Int(i64),
    /// Text.
    Text(String),
}

impl SettingValue {
    /// As JSON.
    pub fn to_json(&self) -> serde_json::Value {
        match self {
            Self::Bool(b) => serde_json::Value::Bool(*b),
            Self::Int(i) => serde_json::Value::from(*i),
            Self::Text(s) => serde_json::Value::String(s.clone()),
        }
    }
}

/// Whether a key is stored (`thrall key set <name>`). Never the key itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct KeyStatus {
    /// Key name (`openrouter`, `fal`).
    pub name: String,
    /// What it's for.
    pub purpose: String,
    /// Whether it's stored.
    pub set: bool,
}

/// A program or model server found on this computer, for one of Thrall's jobs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LocalFound {
    /// The job: `chat`, `transcribe`, `speak` or `pictures`.
    pub job: String,
    /// What it is (`Ollama`, `whisper.cpp`).
    pub name: String,
    /// It can do the job now (installed, running, with a model).
    pub ready: bool,
    /// What it has, or what's missing and how to add it.
    pub note: String,
    /// Model ids to pick for the job, as Thrall takes them (`program/whisper/ggml-base.en`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub models: Vec<String>,
}

/// About the engine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EngineInfo {
    /// Engine version; a client built for another version may offer to restart the engine.
    pub version: String,
    /// The model turns use now (`status` events report changes).
    #[serde(default)]
    pub model: String,
    /// `home` (free, for a household). Other editions may come later.
    #[serde(default)]
    pub edition: String,
}

/// Who the engine acts for on this connection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct UserInfo {
    /// User id.
    pub id: String,
    /// Display name.
    pub name: String,
    /// `owner` (runs this engine) or `member`.
    #[serde(default)]
    pub role: String,
}

/// Limits a client must respect.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Limits {
    /// Largest frame, in bytes.
    pub frame_bytes: u64,
}

/// A session, as `session.opened` and `session.listed` describe it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SessionInfo {
    /// Session id.
    pub session: String,
    /// Title.
    pub title: String,
    /// Folder it works in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    /// Permission mode (`ask` · `auto_edits` · `plan` · `full_auto`).
    pub mode: String,
    /// What it's doing now.
    pub state: SessionState,
    /// Last event `seq` stored for it.
    pub last_seq: u64,
    /// Last activity, Unix milliseconds.
    pub updated: i64,
    /// Money spent in this session so far, in US dollars.
    #[serde(default)]
    pub cost_usd: f64,
    /// The person's group it's in (a group id), if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    /// Pinned to the top of the list.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub pinned: bool,
    /// A page of Home (built in: it can't be renamed, archived, pinned, moved or deleted).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub home: bool,
    /// How long Full auto lasts, when `mode` is `full_auto`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode_limit: Option<ModeLimit>,
}

/// How long Full auto lasts in a session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ModeLimit {
    /// `task_end` (back to Ask when the task finishes) · `time` (back to Ask at `ends_at`) ·
    /// `manual` (until switched off, or Thrall restarts).
    pub until: String,
    /// When it goes back to Ask, Unix milliseconds (`time` only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ends_at: Option<i64>,
}

/// One of the person's session groups.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GroupInfo {
    /// Group id.
    pub group: String,
    /// Name.
    pub name: String,
}

/// What a session is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    /// Nothing running.
    Idle,
    /// A turn is running.
    Working,
    /// A turn is waiting on a permission, plan or question card.
    WaitingForYou,
}

/// A slash command, for autocomplete.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CommandItem {
    /// Name without the slash.
    pub name: String,
    /// What it does.
    pub summary: String,
    /// Arguments hint ("[model id]").
    pub args_hint: String,
    /// It's a skill or a built-in Thrall carries out itself (`/humanise`): clients send
    /// `/name …` as a message, and Thrall does it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub skill: bool,
}
