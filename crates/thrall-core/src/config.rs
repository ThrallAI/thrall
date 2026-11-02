//! `config.toml`: the settings a user can edit by hand. Created with defaults on first start.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::constants::{
    DEFAULT_CONTEXT_LIMIT_TOKENS, DEFAULT_DESKTOP_SHORTCUT, DEFAULT_EMBEDDING_MODEL,
    DEFAULT_FAL_BASE_URL, DEFAULT_HEARTBEAT_HOURS, DEFAULT_HEARTBEAT_MINUTES,
    DEFAULT_HOME_KEEP_DAYS, DEFAULT_IMAGE_MODEL, DEFAULT_LOCAL_BASE_URL, DEFAULT_LOG_LEVEL,
    DEFAULT_MAX_TOKENS, DEFAULT_MODEL, DEFAULT_OPENROUTER_BASE_URL, DEFAULT_REMOTE_PORT,
    DEFAULT_SPEECH_MODEL, DEFAULT_SPEECH_VOICE, DEFAULT_TRANSCRIBE_MODEL,
    DEFAULT_VISION_BACKUP_MODEL, DEFAULT_VISION_MODEL, EDITABLE_SETTINGS, HEARTBEAT_MIN_MINUTES,
    SHORTCUT_MODIFIERS,
};
use crate::error::CoreError;

/// The whole config file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Which AI provider and model to use.
    pub provider: ProviderConfig,
    /// Logging.
    pub logging: LoggingConfig,
    /// How the agent checks its own work.
    pub agent: AgentConfig,
    /// Your own commands, run at points in a turn (`[[hooks]]`).
    pub hooks: Vec<HookConfig>,
    /// MCP servers Thrall can use (`[mcp.servers.<name>]`), started only when needed.
    pub mcp: McpConfig,
    /// Long-term memory and context size.
    pub memory: MemoryConfig,
    /// Named models to switch between (`[profiles.<name>]`, `/model @name`).
    pub profiles: std::collections::BTreeMap<String, ProfileConfig>,
    /// Skills.
    pub skills: SkillsConfig,
    /// How Thrall writes.
    pub writing: WritingConfig,
    /// Home, Thrall's own built-in conversation.
    pub home: HomeConfig,
    /// Check-ins: Thrall goes through your checklist now and then and speaks up in Home.
    pub heartbeat: HeartbeatConfig,
    /// Which pop-ups the desktop app shows, and when.
    pub notifications: NotificationsConfig,
    /// The desktop app itself.
    pub desktop: DesktopConfig,
    /// Phones reaching this engine from outside the computer.
    pub remote: RemoteConfig,
    /// Folders sessions work in.
    pub folders: FoldersConfig,
    /// Split mode (experimental): one model plans and checks, another does the work.
    pub split: SplitConfig,
}

/// `[split]`: split mode, experimental and off unless switched on. The Thinker looks around (it
/// can't change anything), writes a brief and hands the work to the Doer, then checks what comes
/// back and may send it back once. The Doer does the work with every tool and can ask the Thinker
/// when stuck. An empty model means the chat model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct SplitConfig {
    /// Turns run in split mode.
    pub enabled: bool,
    /// The model that plans, briefs and checks.
    pub thinker_model: String,
    /// The model that does the work.
    pub doer_model: String,
}

/// `[folders]`: where sessions work.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct FoldersConfig {
    /// Where a new folder goes unless another is picked (a full path); empty: Thrall's own folder
    /// (`Documents\Thrall`).
    pub new_in: String,
}

/// `[desktop]`: the desktop app.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DesktopConfig {
    /// Shows or hides Thrall's window from anywhere, like `Ctrl+Alt+T`; empty: no shortcut.
    pub shortcut: String,
}

impl Default for DesktopConfig {
    fn default() -> Self {
        Self {
            shortcut: DEFAULT_DESKTOP_SHORTCUT.to_owned(),
        }
    }
}

/// Whether `shortcut` is one or more modifiers then one key (`Ctrl+Alt+T`); empty is fine
/// (no shortcut). Whether the key itself exists is checked by the desktop app when it registers it.
pub fn shortcut_ok(shortcut: &str) -> bool {
    let shortcut = shortcut.trim();
    if shortcut.is_empty() {
        return true;
    }
    let parts: Vec<String> = shortcut
        .split('+')
        .map(|p| p.trim().to_lowercase())
        .collect();
    let Some((key, modifiers)) = parts.split_last() else {
        return false;
    };
    !modifiers.is_empty()
        && !key.is_empty()
        && !SHORTCUT_MODIFIERS.contains(&key.as_str())
        && modifiers
            .iter()
            .all(|m| SHORTCUT_MODIFIERS.contains(&m.as_str()))
}

/// `[remote]`: paired phones reach the engine over an encrypted link. Off until switched on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RemoteConfig {
    /// The phone listener runs (only paired phones get in).
    pub enabled: bool,
    /// The public address phones use, e.g. `https://pc.example.ts.net` (Tailscale Funnel).
    pub address: String,
    /// Port of the phone listener on 127.0.0.1 (what the tunnel forwards to).
    pub port: u16,
}

impl Default for RemoteConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            address: String::new(),
            port: DEFAULT_REMOTE_PORT,
        }
    }
}

/// `[notifications]`: the desktop app's pop-ups.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct NotificationsConfig {
    /// Thrall is waiting for an answer or a plan decision.
    pub needs_you: bool,
    /// A turn the person started finished.
    pub done: bool,
    /// A turn the person started failed.
    pub failed: bool,
    /// A helper or scheduled job finished.
    pub background: bool,
    /// A check-in found something worth saying.
    pub check_ins: bool,
    /// `HH:MM-HH:MM` when only "needs you" pop-ups come through; empty: no quiet hours.
    pub quiet_hours: String,
    /// Only while Thrall's window isn't the one in front.
    pub only_when_away: bool,
}

impl Default for NotificationsConfig {
    fn default() -> Self {
        Self {
            needs_you: true,
            done: true,
            failed: true,
            background: true,
            check_ins: true,
            quiet_hours: String::new(),
            only_when_away: true,
        }
    }
}

/// `[heartbeat]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct HeartbeatConfig {
    /// Check-ins happen (each one is a model call, so it costs a little).
    pub enabled: bool,
    /// Minutes between check-ins.
    pub every_minutes: u32,
    /// When check-ins may happen, `HH:MM-HH:MM` on this computer's clock (it may run past
    /// midnight, e.g. `22:00-06:00`); empty: any time.
    pub active_hours: String,
    /// Model for check-ins (an OpenRouter id); empty: the chat model.
    pub model: String,
}

impl Default for HeartbeatConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            every_minutes: DEFAULT_HEARTBEAT_MINUTES,
            active_hours: DEFAULT_HEARTBEAT_HOURS.to_owned(),
            model: String::new(),
        }
    }
}

/// `HH:MM-HH:MM` as minutes after midnight (start, end); `None` if it isn't one.
pub fn parse_hours(text: &str) -> Option<(u32, u32)> {
    let minutes = |t: &str| {
        let (h, m) = t.trim().split_once(':')?;
        let (h, m): (u32, u32) = (h.parse().ok()?, m.parse().ok()?);
        (h < 24 && m < 60).then_some(h * 60 + m)
    };
    let (from, to) = text.split_once('-')?;
    let (from, to) = (minutes(from)?, minutes(to)?);
    (from != to).then_some((from, to))
}

/// Whether `minute` (after midnight) falls in `HH:MM-HH:MM` hours; empty hours are any time.
pub fn in_hours(hours: &str, minute: u32) -> bool {
    if hours.trim().is_empty() {
        return true;
    }
    match parse_hours(hours) {
        Some((from, to)) if from < to => minute >= from && minute < to,
        Some((from, to)) => minute >= from || minute < to,
        None => true,
    }
}

/// `[home]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct HomeConfig {
    /// When Home starts a fresh page (earlier pages stay readable).
    pub fresh_start: FreshStart,
    /// Days earlier pages are kept before they're deleted; 0 keeps them for good.
    pub keep_days: u32,
}

impl Default for HomeConfig {
    fn default() -> Self {
        Self {
            fresh_start: FreshStart::default(),
            keep_days: DEFAULT_HOME_KEEP_DAYS,
        }
    }
}

/// When Home starts a fresh page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum FreshStart {
    /// Each day.
    #[default]
    Daily,
    /// Each week (Monday).
    Weekly,
    /// Never: one page.
    Never,
}

/// `[writing]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct WritingConfig {
    /// The humaniser: Thrall avoids the habits that make text read as machine-written.
    pub humanise: Humanise,
    /// `/humanise` also works on what AI detectors measure (sentence-length variation, the most
    /// expected word). No promise that a detector passes the result.
    pub detector_aware: bool,
}

/// How much the humaniser does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Humanise {
    /// Thrall writes as the model does.
    #[default]
    Off,
    /// Thrall's replies follow the writing rules.
    Light,
    /// Replies and the documents Thrall writes; each document is style-checked after it's
    /// written, so Thrall can fix what the check finds.
    Full,
}

/// `[skills]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct SkillsConfig {
    /// Skills turned off: not listed for Thrall and not run.
    pub disabled: Vec<String>,
}

/// One `[profiles.<name>]`: a model to switch to by name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileConfig {
    /// Model id: an OpenRouter id, or `chatgpt/<model>` for the ChatGPT plan.
    pub model: String,
    /// What it's for, shown beside it.
    #[serde(default)]
    pub description: String,
}

/// `[memory]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MemoryConfig {
    /// Model that turns text into vectors for searching by meaning (OpenRouter id).
    pub embedding_model: String,
    /// Remember each finished conversation turn so later sessions can recall it.
    pub remember_conversations: bool,
    /// Tokens the model's context may use before older messages are summarised.
    pub context_limit_tokens: usize,
    /// Lasting facts about the person are saved without asking (they're told each time). Off:
    /// Thrall asks before saving one.
    pub save_facts: bool,
    /// Big tool results from several exchanges ago are sent to the model shortened (the stored
    /// result keeps everything; each trim is logged). Off by default. If Thrall seems to forget
    /// something it fetched or read earlier, check this first.
    pub trim_old_tool_results: bool,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            embedding_model: DEFAULT_EMBEDDING_MODEL.to_owned(),
            remember_conversations: true,
            context_limit_tokens: DEFAULT_CONTEXT_LIMIT_TOKENS,
            save_facts: true,
            trim_old_tool_results: false,
        }
    }
}

/// `[mcp]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct McpConfig {
    /// Servers by name.
    pub servers: std::collections::BTreeMap<String, McpServerConfig>,
}

/// One `[mcp.servers.<name>]`: either a program on this computer that speaks MCP on its input and
/// output (`command`), or a server on the web (`url`, MCP's streamable HTTP).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct McpServerConfig {
    /// The program, e.g. `npx` or `uvx` (a server on this computer).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub command: String,
    /// Its arguments.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    /// Extra environment variables (e.g. an API key the server needs).
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub env: std::collections::BTreeMap<String, String>,
    /// The address of a server on the web, e.g. `https://example.com/mcp`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub url: String,
    /// Extra HTTP headers for a web server (e.g. an API key). Values can be `"keychain:name"`.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub headers: std::collections::BTreeMap<String, String>,
    /// The app id a web server gave you, for the few that don't let apps register themselves.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub client_id: String,
    /// Everyone using this Thrall may use it (as the owner: their sign-ins and keys). Off, the
    /// default: only the owner.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub shared: bool,
}

impl McpServerConfig {
    /// Whether it's a server on the web (rather than a program on this computer).
    pub fn is_remote(&self) -> bool {
        !self.url.is_empty()
    }

    /// What's wrong with it, if anything: it needs exactly one of `command` and `url`, and the
    /// settings of the other kind don't apply.
    pub fn problem(&self) -> Option<&'static str> {
        match (self.command.is_empty(), self.url.is_empty()) {
            (true, true) => Some("needs `command` (a program) or `url` (a server on the web)"),
            (false, false) => Some("has both `command` and `url`: pick one"),
            (false, true) if !self.headers.is_empty() || !self.client_id.is_empty() => {
                Some("`headers` and `client_id` are only for a server with a `url`")
            }
            (true, false) if !self.args.is_empty() || !self.env.is_empty() => {
                Some("`args` and `env` are only for a server with a `command`")
            }
            (true, false)
                if !(self.url.starts_with("https://") || self.url.starts_with("http://")) =>
            {
                Some("`url` must start with https:// (or http://)")
            }
            _ => None,
        }
    }
}

/// One `[[hooks]]` entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HookConfig {
    /// When it runs.
    pub event: HookEvent,
    /// Only for this tool (pre_tool / post_tool); every tool when left out.
    #[serde(default)]
    pub tool: Option<String>,
    /// The command, run in your shell. Details arrive as JSON on its input.
    pub command: String,
    /// Seconds before it's stopped (default 30).
    #[serde(default)]
    pub timeout_secs: Option<u64>,
}

/// When a hook runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookEvent {
    /// Before a tool runs. Exit code 2 blocks it; the hook's output says why.
    PreTool,
    /// After a tool ran.
    PostTool,
    /// When you send a message. Anything it prints is added for Thrall to read.
    UserMessage,
    /// When a turn ends.
    TurnEnd,
}

impl HookEvent {
    /// The config name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PreTool => "pre_tool",
            Self::PostTool => "post_tool",
            Self::UserMessage => "user_message",
            Self::TurnEnd => "turn_end",
        }
    }
}

/// `[agent]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AgentConfig {
    /// After changing files, Thrall checks its work before saying it's done.
    pub verify: bool,
    /// Thrall's final reply is checked against what it actually did; false claims get corrected.
    pub claim_check: bool,
    /// When Thrall stops to ask instead of deciding and noting what it assumed.
    pub questions: Questions,
    /// How long a question card waits before Thrall goes ahead with its recommended answers.
    pub question_wait_minutes: u32,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            verify: true,
            claim_check: true,
            questions: Questions::default(),
            question_wait_minutes: crate::constants::DEFAULT_QUESTION_WAIT_MINUTES,
        }
    }
}

/// When Thrall asks instead of deciding. Full auto never asks, whatever this says.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Questions {
    /// Never: Thrall decides everything and lists what it assumed.
    Never,
    /// Only for what can't be undone, costs money, or could go two very different ways.
    #[default]
    WhenItMatters,
    /// Also when Thrall isn't sure what's wanted.
    WhenUnsure,
}

/// AI providers Thrall can talk to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ProviderKind {
    /// OpenRouter (one key, many models).
    #[default]
    OpenRouter,
}

impl ProviderKind {
    /// Every provider Thrall supports.
    pub const ALL: &'static [Self] = &[Self::OpenRouter];

    /// The name used in config, commands and the keychain.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OpenRouter => "openrouter",
        }
    }

    /// Environment variable that can supply this provider's key instead of the keychain
    /// (for scripts and CI).
    pub fn key_env_var(self) -> &'static str {
        match self {
            Self::OpenRouter => "OPENROUTER_API_KEY",
        }
    }

    /// Parse a provider name typed by the user.
    pub fn parse(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "openrouter" => Some(Self::OpenRouter),
            _ => None,
        }
    }
}

/// `[provider]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ProviderConfig {
    /// Which provider.
    pub kind: ProviderKind,
    /// Model id as the provider names it.
    pub model: String,
    /// API base URL (change only for a proxy or a compatible service).
    pub base_url: String,
    /// Most tokens the model may write in one reply.
    pub max_tokens: u32,
    /// Reads every attached picture; the chat model gets its description. Empty: pictures go to
    /// the chat model directly (if it can see them).
    pub vision_model: String,
    /// Tried when `vision_model` fails (busy, down). Empty: no backup.
    pub vision_backup_model: String,
    /// Makes and edits pictures (`make_image`); every picture asks first, with its price.
    pub image_model: String,
    /// Reads text aloud (`speak`).
    pub speech_model: String,
    /// The voice `speak` uses unless asked for another.
    pub speech_voice: String,
    /// Turns recordings into text (attached audio, `transcribe`, the microphone button).
    pub transcribe_model: String,
    /// fal.ai's job queue, for video (`make_video`; its key: `thrall key set fal`).
    pub fal_base_url: String,
    /// A model server on this computer or network (Ollama, LM Studio) for `local/<model>`
    /// models: free, and no key needed. Empty: local models are off.
    pub local_base_url: String,
}

impl Default for ProviderConfig {
    fn default() -> Self {
        Self {
            kind: ProviderKind::default(),
            model: DEFAULT_MODEL.to_owned(),
            base_url: DEFAULT_OPENROUTER_BASE_URL.to_owned(),
            max_tokens: DEFAULT_MAX_TOKENS,
            vision_model: DEFAULT_VISION_MODEL.to_owned(),
            vision_backup_model: DEFAULT_VISION_BACKUP_MODEL.to_owned(),
            image_model: DEFAULT_IMAGE_MODEL.to_owned(),
            speech_model: DEFAULT_SPEECH_MODEL.to_owned(),
            speech_voice: DEFAULT_SPEECH_VOICE.to_owned(),
            transcribe_model: DEFAULT_TRANSCRIBE_MODEL.to_owned(),
            fal_base_url: DEFAULT_FAL_BASE_URL.to_owned(),
            local_base_url: DEFAULT_LOCAL_BASE_URL.to_owned(),
        }
    }
}

/// `[logging]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LoggingConfig {
    /// `error`, `warn`, `info`, `debug` or `trace`.
    pub level: String,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: DEFAULT_LOG_LEVEL.to_owned(),
        }
    }
}

impl Config {
    /// Parse config text. Missing settings take their defaults; unknown ones are an error, so a
    /// typo never silently does nothing.
    pub fn parse(text: &str, path: &Path) -> Result<Self, CoreError> {
        let config: Self = toml::from_str(text).map_err(|e| CoreError::Config {
            path: path.to_path_buf(),
            message: e.message().to_owned(),
        })?;
        if let Some((name, problem)) = config
            .mcp
            .servers
            .iter()
            .find_map(|(name, s)| s.problem().map(|p| (name, p)))
        {
            return Err(CoreError::Config {
                path: path.to_path_buf(),
                message: format!("MCP server `{name}` {problem}"),
            });
        }
        Ok(config)
    }

    /// Load the config file, writing one with defaults first if it doesn't exist.
    pub fn load_or_create(path: &Path) -> Result<Self, CoreError> {
        let io_err = |source| CoreError::Io {
            path: path.to_path_buf(),
            source,
        };
        match std::fs::read_to_string(path) {
            Ok(text) => Self::parse(&text, path),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let config = Self::default();
                let text = toml::to_string_pretty(&config).map_err(|e| CoreError::Config {
                    path: path.to_path_buf(),
                    message: e.to_string(),
                })?;
                std::fs::write(path, text).map_err(io_err)?;
                Ok(config)
            }
            Err(e) => Err(io_err(e)),
        }
    }
}

/// `text` (a config file) with `key` (dotted, one of [`EDITABLE_SETTINGS`]) set to `value`.
/// Comments and layout are kept. The result must still be a valid config, so a wrong type or
/// value is refused before anything is written.
pub fn with_setting(
    text: &str,
    key: &str,
    value: &serde_json::Value,
    path: &Path,
) -> Result<(String, Config), CoreError> {
    // The file is fine (nothing was written): say the change was refused, and why.
    changed(text, key, value, path).map_err(|e| match e {
        CoreError::Config { message, .. } => CoreError::SettingRefused(message),
        other => other,
    })
}

fn changed(
    text: &str,
    key: &str,
    value: &serde_json::Value,
    path: &Path,
) -> Result<(String, Config), CoreError> {
    let bad = |message: String| CoreError::Config {
        path: path.to_path_buf(),
        message,
    };
    let profile = profile_key(key);
    let skill = skill_key(key);
    let mcp = mcp_shared_key(key);
    if !EDITABLE_SETTINGS.contains(&key) && profile.is_none() && skill.is_none() && mcp.is_none() {
        return Err(bad(format!(
            "{key} can't be changed from the settings screen"
        )));
    }
    let item = match value {
        serde_json::Value::Bool(b) => toml_edit::value(*b),
        serde_json::Value::String(s) => toml_edit::value(s.trim()),
        serde_json::Value::Number(n) => match n.as_i64() {
            Some(i) => toml_edit::value(i),
            None => return Err(bad(format!("{key} needs a whole number"))),
        },
        _ => return Err(bad(format!("{key} needs a yes/no, a number or text"))),
    };
    let mut doc: toml_edit::DocumentMut = text
        .parse()
        .map_err(|e: toml_edit::TomlError| bad(e.message().to_owned()))?;
    if let Some(name) = mcp {
        let Some(on) = value.as_bool() else {
            return Err(bad(format!("{key} needs a yes/no")));
        };
        let server = doc
            .get_mut("mcp")
            .and_then(|m| m.get_mut("servers"))
            .and_then(|s| s.get_mut(name))
            .and_then(toml_edit::Item::as_table_like_mut)
            .ok_or_else(|| bad(format!("there's no MCP server called {name}")))?;
        if on {
            server.insert("shared", toml_edit::value(true));
        } else {
            server.remove("shared");
        }
        let text = doc.to_string();
        let config = Config::parse(&text, path)?;
        return Ok((text, config));
    }
    if let Some(name) = skill {
        let Some(on) = value.as_bool() else {
            return Err(bad(format!("{key} needs a yes/no")));
        };
        set_skill_enabled(&mut doc, name, on);
        let text = doc.to_string();
        let config = Config::parse(&text, path)?;
        return Ok((text, config));
    }
    if let Some((profile, field)) = profile {
        set_profile(&mut doc, profile, field, item, value);
        let text = doc.to_string();
        let config = Config::parse(&text, path)?;
        return Ok((text, config));
    }
    let (table, name) = key.split_once('.').unwrap_or(("", key));
    if !doc.contains_table(table) {
        doc.insert(table, toml_edit::table());
    }
    let Some(section) = doc.get_mut(table).and_then(toml_edit::Item::as_table_mut) else {
        return Err(bad(format!("[{table}] isn't a section")));
    };
    section.insert(name, item);
    let text = doc.to_string();
    let config = Config::parse(&text, path)?;
    let h = &config.heartbeat;
    if !h.active_hours.trim().is_empty() && parse_hours(&h.active_hours).is_none() {
        return Err(bad(
            "active hours look like 08:00-22:00 (or leave it empty for any time)".to_owned(),
        ));
    }
    let quiet = &config.notifications.quiet_hours;
    if !quiet.trim().is_empty() && parse_hours(quiet).is_none() {
        return Err(bad(
            "quiet hours look like 22:00-07:00 (or leave it empty for none)".to_owned(),
        ));
    }
    if !shortcut_ok(&config.desktop.shortcut) {
        return Err(bad(
            "a shortcut is keys joined by +, like Ctrl+Alt+T (or leave it empty for none)"
                .to_owned(),
        ));
    }
    let address = config.remote.address.trim();
    if !address.is_empty() && !address.starts_with("https://") && !address.starts_with("http://") {
        return Err(bad(
            "the phone address starts with https:// (or leave it empty)".to_owned(),
        ));
    }
    let new_in = config.folders.new_in.trim();
    if !new_in.is_empty() && !Path::new(new_in).is_absolute() {
        return Err(bad(
            "new folders go in a full path, like C:\\Users\\you\\Projects (or leave it empty)"
                .to_owned(),
        ));
    }
    if h.every_minutes < HEARTBEAT_MIN_MINUTES {
        return Err(bad(format!(
            "check-ins can be at most every {HEARTBEAT_MIN_MINUTES} minutes"
        )));
    }
    Ok((text, config))
}

/// `profiles.<name>.model` or `.description` → (name, field). Names are lower-case letters,
/// digits, `-` and `_`.
fn profile_key(key: &str) -> Option<(&str, &str)> {
    let rest = key.strip_prefix("profiles.")?;
    let (name, field) = rest.rsplit_once('.')?;
    let ok = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_');
    (ok && (field == "model" || field == "description")).then_some((name, field))
}

/// `mcp.servers.<name>.shared` → the server's name.
fn mcp_shared_key(key: &str) -> Option<&str> {
    let name = key.strip_prefix("mcp.servers.")?.strip_suffix(".shared")?;
    (!name.trim().is_empty()).then_some(name)
}

/// `skills.<name>.enabled` → the skill's name.
fn skill_key(key: &str) -> Option<&str> {
    let name = key.strip_prefix("skills.")?.strip_suffix(".enabled")?;
    (!name.trim().is_empty()).then_some(name)
}

/// Turn a skill on or off: its name leaves or joins `[skills] disabled`.
fn set_skill_enabled(doc: &mut toml_edit::DocumentMut, name: &str, on: bool) {
    if !doc.contains_table("skills") {
        doc.insert("skills", toml_edit::table());
    }
    let Some(skills) = doc
        .get_mut("skills")
        .and_then(toml_edit::Item::as_table_mut)
    else {
        return;
    };
    let mut names: Vec<String> = skills
        .get("disabled")
        .and_then(toml_edit::Item::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    names.retain(|n| !n.eq_ignore_ascii_case(name));
    if !on {
        names.push(name.to_owned());
    }
    let mut list = toml_edit::Array::new();
    for n in names {
        list.push(n);
    }
    skills.insert("disabled", toml_edit::value(list));
}

/// Set a profile's field; an empty model removes the profile.
fn set_profile(
    doc: &mut toml_edit::DocumentMut,
    name: &str,
    field: &str,
    item: toml_edit::Item,
    value: &serde_json::Value,
) {
    if !doc.contains_table("profiles") {
        let mut t = toml_edit::Table::new();
        t.set_implicit(true);
        doc.insert("profiles", toml_edit::Item::Table(t));
    }
    let Some(profiles) = doc
        .get_mut("profiles")
        .and_then(toml_edit::Item::as_table_mut)
    else {
        return;
    };
    let removing = field == "model" && value.as_str().is_some_and(|s| s.trim().is_empty());
    if removing {
        profiles.remove(name);
        return;
    }
    if !profiles.contains_table(name) {
        profiles.insert(name, toml_edit::table());
    }
    if let Some(p) = profiles
        .get_mut(name)
        .and_then(toml_edit::Item::as_table_mut)
    {
        p.insert(field, item);
    }
}

/// Change one setting in the config file at `path` (see [`with_setting`]) and return the new
/// config. The file is replaced in one step, so a crash never leaves half a file.
pub fn save_setting(
    path: &Path,
    key: &str,
    value: &serde_json::Value,
) -> Result<Config, CoreError> {
    let io_err = |source| CoreError::Io {
        path: path.to_path_buf(),
        source,
    };
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(io_err(e)),
    };
    let (text, config) = with_setting(&text, key, value, path)?;
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, text).map_err(io_err)?;
    std::fs::rename(&tmp, path).map_err(io_err)?;
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skills_are_turned_off_and_on_again() {
        let p = Path::new("config.toml");
        let (text, config) =
            with_setting("", "skills.tidy.enabled", &serde_json::json!(false), p).unwrap();
        assert_eq!(config.skills.disabled, vec!["tidy".to_owned()]);
        let (text, config) =
            with_setting(&text, "skills.Notes.enabled", &serde_json::json!(false), p).unwrap();
        assert_eq!(config.skills.disabled.len(), 2);
        let (_, config) =
            with_setting(&text, "skills.TIDY.enabled", &serde_json::json!(true), p).unwrap();
        assert_eq!(config.skills.disabled, vec!["Notes".to_owned()]);
        assert!(with_setting("", "skills.x.enabled", &serde_json::json!("no"), p).is_err());
        assert!(Config::default().memory.save_facts);
    }

    #[test]
    fn services_are_shared_and_unshared() {
        let p = Path::new("config.toml");
        let text = "[mcp.servers.youtube]\ncommand = \"python\"\n\n\
                    [mcp.servers.github]\nurl = \"https://example.com/mcp\"\n";
        let (text, config) = with_setting(
            text,
            "mcp.servers.youtube.shared",
            &serde_json::json!(true),
            p,
        )
        .unwrap();
        assert!(config.mcp.servers["youtube"].shared);
        assert!(!config.mcp.servers["github"].shared);
        assert!(text.contains("shared = true"));
        let (text, config) = with_setting(
            &text,
            "mcp.servers.youtube.shared",
            &serde_json::json!(false),
            p,
        )
        .unwrap();
        assert!(!config.mcp.servers["youtube"].shared);
        assert!(!text.contains("shared"));
        assert!(
            with_setting(
                &text,
                "mcp.servers.nope.shared",
                &serde_json::json!(true),
                p
            )
            .is_err()
        );
        assert!(
            with_setting(
                &text,
                "mcp.servers.youtube.shared",
                &serde_json::json!("yes"),
                p
            )
            .is_err()
        );
    }

    #[test]
    fn the_humaniser_is_off_until_chosen() {
        let p = Path::new("config.toml");
        assert_eq!(Config::default().writing.humanise, Humanise::Off);
        let (_, config) =
            with_setting("", "writing.humanise", &serde_json::json!("full"), p).unwrap();
        assert_eq!(config.writing.humanise, Humanise::Full);
        assert!(with_setting("", "writing.humanise", &serde_json::json!("loud"), p).is_err());
        assert!(!Config::default().writing.detector_aware);
        assert_eq!(Config::default().home.fresh_start, FreshStart::Daily);
        assert_eq!(Config::default().home.keep_days, DEFAULT_HOME_KEEP_DAYS);
        let (_, config) =
            with_setting("", "home.fresh_start", &serde_json::json!("weekly"), p).unwrap();
        assert_eq!(config.home.fresh_start, FreshStart::Weekly);
        assert!(!Config::default().heartbeat.enabled);
        let n = Config::default().notifications;
        assert!(n.needs_you && n.check_ins && n.only_when_away && n.quiet_hours.is_empty());
        assert!(
            with_setting(
                "",
                "notifications.quiet_hours",
                &serde_json::json!("night"),
                p
            )
            .is_err()
        );
        let hours = |v: &str| with_setting("", "heartbeat.active_hours", &serde_json::json!(v), p);
        assert!(hours("8am-10pm").is_err());
        assert!(hours("22:00-06:00").is_ok() && hours("").is_ok());
        assert!(with_setting("", "heartbeat.every_minutes", &serde_json::json!(1), p).is_err());
        assert!(with_setting("", "home.keep_days", &serde_json::json!(-1), p).is_err());
        let (_, config) =
            with_setting("", "writing.detector_aware", &serde_json::json!(true), p).unwrap();
        assert!(config.writing.detector_aware);
    }
    #[test]
    fn the_desktop_shortcut_stays_clear_of_games_and_can_be_changed_or_turned_off() {
        let p = Path::new("config.toml");
        assert_eq!(Config::default().desktop.shortcut, "Ctrl+Alt+T");
        let set = |v: &str| with_setting("", "desktop.shortcut", &serde_json::json!(v), p);
        assert_eq!(
            set("Alt+Shift+T").unwrap().1.desktop.shortcut,
            "Alt+Shift+T"
        );
        assert_eq!(set("").unwrap().1.desktop.shortcut, "");
        for bad in [
            "Space",
            "Ctrl+",
            "Ctrl+Shift",
            "Ctrl Alt Space",
            "Hyper+Space",
        ] {
            assert!(set(bad).is_err(), "{bad} should be refused");
        }
    }

    #[test]
    fn active_hours_can_run_past_midnight() {
        assert_eq!(parse_hours("08:00-22:00"), Some((480, 1320)));
        assert!(in_hours("08:00-22:00", 600) && !in_hours("08:00-22:00", 1330));
        assert!(in_hours("22:00-06:00", 1380) && in_hours("22:00-06:00", 60));
        assert!(!in_hours("22:00-06:00", 720));
        assert!(in_hours("", 0));
        assert_eq!(parse_hours("25:00-26:00"), None);
    }

    #[test]
    fn a_setting_is_changed_and_comments_stay() {
        let text = "# mine\n[provider]\nmodel = \"a/b\" # chat\n";
        let p = Path::new("config.toml");
        let (out, config) =
            with_setting(text, "provider.model", &serde_json::json!("c/d"), p).unwrap();
        assert!(out.contains("# mine") && out.contains("model = \"c/d\""));
        assert_eq!(config.provider.model, "c/d");
        let (out, config) =
            with_setting(&out, "agent.verify", &serde_json::json!(false), p).unwrap();
        assert!(out.contains("[agent]") && !config.agent.verify);
    }

    #[test]
    fn profiles_are_added_and_removed() {
        let p = Path::new("config.toml");
        let (out, config) = with_setting(
            "",
            "profiles.fast.model",
            &serde_json::json!("chatgpt/gpt-5.3-codex"),
            p,
        )
        .unwrap();
        assert_eq!(config.profiles["fast"].model, "chatgpt/gpt-5.3-codex");
        let (out, config) = with_setting(
            &out,
            "profiles.fast.description",
            &serde_json::json!("Quick"),
            p,
        )
        .unwrap();
        assert_eq!(config.profiles["fast"].description, "Quick");
        let (_, config) =
            with_setting(&out, "profiles.fast.model", &serde_json::json!(""), p).unwrap();
        assert!(config.profiles.is_empty());
        assert!(with_setting("", "profiles.Bad Name.model", &serde_json::json!("x"), p).is_err());
        assert!(
            with_setting("", "profiles.x.description", &serde_json::json!("d"), p).is_err(),
            "a profile needs a model"
        );
    }

    #[test]
    fn wrong_settings_are_refused() {
        let p = Path::new("config.toml");
        let one = |k: &str, v: serde_json::Value| with_setting("", k, &v, p);
        assert!(
            one("mcp.servers", serde_json::json!("x")).is_err(),
            "not editable"
        );
        assert!(
            one("agent.verify", serde_json::json!("yes")).is_err(),
            "wrong type"
        );
        assert!(one("provider.max_tokens", serde_json::json!(1.5)).is_err());
        assert!(one("provider.max_tokens", serde_json::json!(4096)).is_ok());
    }

    #[test]
    fn mcp_servers_are_a_program_or_a_web_address() {
        let local = parse("[mcp.servers.gh]\ncommand = \"npx\"\nargs = [\"x\"]\n").unwrap();
        assert!(!local.mcp.servers["gh"].is_remote());
        let web = parse(
            "[mcp.servers.docs]\nurl = \"https://example.com/mcp\"\n\
             headers = { X-Api-Key = \"keychain:docs\" }\n",
        )
        .unwrap();
        assert!(web.mcp.servers["docs"].is_remote());
        let refused = |text: &str, says: &str| {
            let e = parse(text).unwrap_err().to_string();
            assert!(e.contains(says), "{e}");
        };
        refused("[mcp.servers.a]\nargs = [\"x\"]\n", "needs `command`");
        refused(
            "[mcp.servers.a]\ncommand = \"x\"\nurl = \"https://e.com\"\n",
            "pick one",
        );
        refused("[mcp.servers.a]\nurl = \"e.com/mcp\"\n", "https://");
        refused(
            "[mcp.servers.a]\nurl = \"https://e.com\"\nenv = { A = \"b\" }\n",
            "only for a server with a `command`",
        );
        refused(
            "[mcp.servers.a]\ncommand = \"x\"\nclient_id = \"c\"\n",
            "only for a server with a `url`",
        );
    }

    fn parse(text: &str) -> Result<Config, CoreError> {
        Config::parse(text, Path::new("config.toml"))
    }

    #[test]
    fn empty_file_gives_defaults() {
        assert_eq!(parse("").unwrap(), Config::default());
    }

    #[test]
    fn partial_file_keeps_other_defaults() {
        let config = parse("[provider]\nmodel = \"openai/gpt-5\"\n").unwrap();
        assert_eq!(config.provider.model, "openai/gpt-5");
        assert_eq!(config.provider.max_tokens, DEFAULT_MAX_TOKENS);
        assert_eq!(config.logging, LoggingConfig::default());
    }

    #[test]
    fn typo_is_an_error_not_ignored() {
        let err = parse("[provider]\nmodle = \"x\"\n").unwrap_err();
        assert!(err.to_string().contains("config.toml"));
    }

    #[test]
    fn defaults_round_trip_through_toml() {
        let text = toml::to_string_pretty(&Config::default()).unwrap();
        assert_eq!(parse(&text).unwrap(), Config::default());
    }

    #[test]
    fn provider_names_parse() {
        assert_eq!(
            ProviderKind::parse(" OpenRouter "),
            Some(ProviderKind::OpenRouter)
        );
        assert_eq!(ProviderKind::parse("nope"), None);
    }
}
