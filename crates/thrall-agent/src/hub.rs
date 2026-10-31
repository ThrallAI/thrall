//! The hub: one engine serving every client. It runs turns for whoever asks, keeps one event bus
//! that every client listens to, tracks what each session is doing (for `status` events), and
//! swaps in a fresh engine when Thrall restarts. Clients never hold agent state; they send
//! requests here and draw the events that come back.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock, Weak};

use futures::future::BoxFuture;
use thrall_core::constants::{CONFIG_FILE, UPLOADS_DIR, id_prefix};
use thrall_core::ids::now_ms;
use thrall_gates::Mode;
use thrall_protocol::constants::{
    MODE_LIMIT_MAX_MINUTES, MODE_UNTIL_MANUAL, MODE_UNTIL_TASK_END, MODE_UNTIL_TIME,
};
use thrall_protocol::{
    Event, EventFrame, ModeLimit, NoticeLevel, SessionInfo, SessionState, TurnStatus,
};
use thrall_store::Session;
use tokio::sync::{broadcast, mpsc};

use crate::constants::{ATTACHED_LINE, role::USER as ROLE_USER};
use crate::constants::{
    BUS_CAPACITY, CHATGPT_SUGGESTED_MODEL, EVENT_CHANNEL_CAPACITY, FACTS_LIST_LIMIT,
    FULL_AUTO_ENDED_MOVED, FULL_AUTO_ENDED_RESTART, FULL_AUTO_ENDED_TASK, FULL_AUTO_ENDED_TIME,
    FULL_AUTO_OWNER_ONLY, FULL_AUTO_SWEEP_SESSIONS, MS_PER_MINUTE, RESTARTED_NOTICE,
    REWIND_QUOTE_CHARS, SIGN_IN_TIMEOUT_SECS,
};
use crate::error::AgentError;
use crate::prompts::Reply;
use crate::state::AppState;
use crate::turn::StoredContent;
use crate::{commands, runners, session, turn};

/// Where an uploaded file is: `<data folder>/uploads/<id>/<name>`. `None` for an id that isn't
/// a plain file id (so a request can't point outside the uploads folder) or a file that's gone.
pub fn uploaded_file(state: &AppState, id: &str) -> Option<PathBuf> {
    let plain = id.starts_with(id_prefix::FILE)
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if !plain {
        return None;
    }
    let folder = state.my_dir()?.join(UPLOADS_DIR).join(id);
    std::fs::read_dir(folder)
        .ok()?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .find(|p| p.is_file())
}

/// Builds a fresh engine from the config file; used when Thrall restarts itself.
pub type Rebuild = Box<dyn Fn() -> BoxFuture<'static, Result<AppState, String>> + Send + Sync>;

/// What happened to a message sent with [`Hub::send`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sent {
    /// It started a new turn.
    Started,
    /// A turn was already running; the message joins it at its next step.
    Joined,
}

/// One engine, shared by every client.
pub struct Hub {
    state: RwLock<Arc<AppState>>,
    rebuild: Option<Rebuild>,
    bus: broadcast::Sender<EventFrame>,
    /// Sessions with a permission, plan or question card open.
    waiting: Mutex<HashSet<String>>,
    /// A setting was saved while Thrall was working; the engine reloads once it's idle.
    settings_pending: AtomicBool,
    /// A ChatGPT sign-in waiting for the browser; a new one replaces it.
    sign_in: Mutex<Option<tokio::task::AbortHandle>>,
    /// A web MCP server's sign-in waiting for the browser; a new one replaces it.
    mcp_sign_in: Mutex<Option<tokio::task::AbortHandle>>,
    /// Chats in Full auto, by session id, and when each goes back to Ask. Kept here, not on
    /// disk, so Full auto never lasts past a restart of Thrall.
    full_auto: Mutex<HashMap<String, FullAuto>>,
    me: Weak<Hub>,
}

/// One chat in Full auto.
struct FullAuto {
    /// Whose chat it is (one chat per person is in Full auto).
    user: String,
    /// How long it lasts.
    limit: ModeLimit,
    /// The timer that ends timed Full auto.
    timer: Option<tokio::task::AbortHandle>,
}

/// How long Full auto lasts, from what a client asked for. `None` for something that makes no
/// sense (an unknown `until`, or `time` without sensible minutes).
pub fn mode_limit_from(until: Option<&str>, minutes: Option<u32>) -> Option<ModeLimit> {
    match until.unwrap_or(MODE_UNTIL_TASK_END) {
        MODE_UNTIL_TASK_END | MODE_UNTIL_MANUAL => Some(ModeLimit {
            until: until.unwrap_or(MODE_UNTIL_TASK_END).to_owned(),
            ends_at: None,
        }),
        MODE_UNTIL_TIME => {
            let minutes = minutes.filter(|m| (1..=MODE_LIMIT_MAX_MINUTES).contains(m))?;
            Some(ModeLimit {
                until: MODE_UNTIL_TIME.to_owned(),
                ends_at: Some(now_ms() + i64::from(minutes) * MS_PER_MINUTE),
            })
        }
        _ => None,
    }
}

impl Hub {
    /// Start the hub on `state`, with its background runners and scheduler.
    pub fn new(state: AppState, rebuild: Option<Rebuild>) -> Arc<Self> {
        let (bus, _) = broadcast::channel(BUS_CAPACITY);
        let state = Arc::new(state);
        let hub = Arc::new_cyclic(|me| Self {
            state: RwLock::new(state.clone()),
            rebuild,
            bus,
            waiting: Mutex::new(HashSet::new()),
            settings_pending: AtomicBool::new(false),
            sign_in: Mutex::new(None),
            mcp_sign_in: Mutex::new(None),
            full_auto: Mutex::new(HashMap::new()),
            me: me.clone(),
        });
        hub.attach(state);
        hub
    }

    /// The engine acting for `user`.
    fn me(&self, user: &str) -> AppState {
        self.state().for_user(user)
    }

    /// The engine as it is now (a restart replaces it).
    pub fn state(&self) -> Arc<AppState> {
        match self.state.read() {
            Ok(s) => s.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    /// Every event in every session, live. A receiver that falls behind gets `Lagged` and should
    /// catch up from the event log.
    pub fn subscribe(&self) -> broadcast::Receiver<EventFrame> {
        self.bus.subscribe()
    }

    /// Start `state`'s runners and pass its out-of-turn events (background helpers, scheduled
    /// runs) onto the bus.
    fn attach(&self, state: Arc<AppState>) {
        runners::start(state.clone());
        let mut notices = state.subscribe();
        let me = self.me.clone();
        tokio::spawn(async move {
            loop {
                match notices.recv().await {
                    Ok(frame) => match me.upgrade() {
                        Some(hub) => hub.publish(frame),
                        None => break,
                    },
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!(missed = n, "event bus forwarder fell behind");
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });
    }

    /// Put a frame on the bus, and a `status` frame after it when the session's state changes.
    fn publish(&self, frame: EventFrame) {
        let session = frame.session.clone();
        let changed = match &frame.event {
            Event::TurnStarted { .. } => Some(SessionState::Working),
            Event::PermissionRequest { .. } | Event::PlanProposed { .. } => {
                self.set_waiting(&session, true);
                Some(SessionState::WaitingForYou)
            }
            Event::PermissionResolved { .. } | Event::PlanResolved { .. } => {
                self.set_waiting(&session, false);
                Some(SessionState::Working)
            }
            // A turn still finishes up after its reply (it names the session, saves an episode):
            // the session is idle only once that's done. Turns run here report idle when they
            // return (`run_turns`); scheduled runs report it with their `agent.completed`.
            Event::TurnCompleted { .. } => {
                self.set_waiting(&session, false);
                (!self.state().is_running(&session)).then_some(SessionState::Idle)
            }
            Event::AgentCompleted { .. } => {
                (!self.state().is_running(&session)).then_some(SessionState::Idle)
            }
            _ => None,
        };
        // Nobody listening is fine.
        let _ = self.bus.send(frame);
        if let Some(state) = changed {
            self.publish_status(&session, state);
        }
    }

    fn publish_status(&self, session: &str, state: SessionState) {
        let _ = self.bus.send(EventFrame::new(
            session,
            None,
            now_ms(),
            Event::Status {
                state,
                model: self.state().model(),
            },
        ));
    }

    fn set_waiting(&self, session: &str, on: bool) {
        if let Ok(mut w) = self.waiting.lock() {
            if on {
                w.insert(session.to_owned());
            } else {
                w.remove(session);
            }
        }
    }

    /// What a session is doing now.
    pub fn state_of(&self, session_id: &str) -> SessionState {
        let waiting = self
            .waiting
            .lock()
            .map(|w| w.contains(session_id))
            .unwrap_or(false);
        if waiting {
            SessionState::WaitingForYou
        } else if self.state().is_running(session_id) {
            SessionState::Working
        } else {
            SessionState::Idle
        }
    }

    /// A session as clients see it.
    pub async fn info(&self, s: &Session) -> SessionInfo {
        let state = self.state().for_user(&s.user_id);
        let cost_usd = match state.store.usage_totals(&state.user_id, Some(&s.id)).await {
            Ok(totals) => totals.cost_usd,
            Err(e) => {
                tracing::warn!(%e, session = s.id, "couldn't read a session's cost");
                0.0
            }
        };
        SessionInfo {
            session: s.id.clone(),
            title: s.title.clone(),
            workspace: s.workspace.clone(),
            mode: s.mode.clone(),
            state: self.state_of(&s.id),
            last_seq: s.last_seq,
            updated: s.updated_at,
            cost_usd,
            group: s.group_id.clone(),
            pinned: s.pinned_at.is_some(),
            home: s.home_day.is_some(),
            mode_limit: self.mode_limit(&s.id),
        }
    }

    /// Home's current page (a fresh one when the last has had its day) and its earlier pages,
    /// newest first.
    pub async fn home(&self, user: &str) -> Result<(Session, Vec<Session>), AgentError> {
        let state = self.me(user);
        let page = crate::home::current(&state).await?;
        let earlier = crate::home::earlier(&state, &page.id).await?;
        Ok((page, earlier))
    }

    /// Delete every Home page; the next [`Hub::home`] starts afresh. Refused while Thrall is
    /// working in Home.
    pub async fn clear_home(&self, user: &str) -> Result<(), AgentError> {
        let state = self.me(user);
        crate::home::clear(&state).await
    }

    /// The check-in checklist and when the next check-in is due.
    pub fn heartbeat(&self) -> (String, Option<i64>) {
        let state = self.state();
        (
            crate::heartbeat::checklist(&state),
            crate::heartbeat::next_due(&state),
        )
    }

    /// Replace the check-in checklist.
    pub fn set_checklist(&self, text: &str) -> Result<(), AgentError> {
        crate::heartbeat::save_checklist(&self.state(), text)
    }

    /// Check in now, in the background (the result shows in Home).
    pub fn check_in_now(&self) -> Result<(), AgentError> {
        let state = self.state();
        if crate::heartbeat::checklist(&state).is_empty() {
            return Err(AgentError::InputRefused(
                "Write a checklist first: it's what Thrall goes through.".to_owned(),
            ));
        }
        tokio::spawn(async move {
            if let Err(e) = crate::heartbeat::beat(&state, true).await {
                tracing::warn!(%e, "check-in failed");
            }
        });
        Ok(())
    }

    /// Refuse changes Home doesn't allow (it's built in).
    async fn not_home(&self, user: &str, session_id: &str) -> Result<(), AgentError> {
        let state = self.me(user);
        if session::get(&state, session_id).await?.home_day.is_some() {
            return Err(AgentError::HomeIsBuiltIn);
        }
        Ok(())
    }

    /// Start a session. `workspace` should be the client's folder: without one the session works
    /// in the engine's own folder.
    pub async fn create(
        &self,
        user: &str,
        workspace: Option<String>,
        title: Option<String>,
        mode: Option<Mode>,
    ) -> Result<Session, AgentError> {
        let state = self.me(user);
        if mode == Some(Mode::FullAuto) && user != state.owner_id {
            return Err(AgentError::InputRefused(FULL_AUTO_OWNER_ONLY.to_owned()));
        }
        let s = session::create(&state, workspace).await?;
        if mode == Some(Mode::FullAuto) {
            self.set_mode(user, &s.id, Mode::FullAuto, None).await?;
        } else if let Some(mode) = mode.filter(|m| *m != Mode::Ask) {
            session::set_mode(&state, &s.id, mode).await?;
        }
        if let Some(title) = title.filter(|t| !t.trim().is_empty()) {
            state
                .store
                .set_session_title(&state.user_id, &s.id, title.trim())
                .await?;
        }
        session::get(&state, &s.id).await
    }

    /// Send a message in `session_id`: a new turn, or added to the one running. `files` are ids
    /// of uploaded files to attach; they can only start a turn, not join one.
    pub async fn send(
        &self,
        user: &str,
        session_id: &str,
        text: &str,
        files: &[String],
    ) -> Result<Sent, AgentError> {
        let state = self.me(user);
        session::get(&state, session_id).await?;
        let files = files
            .iter()
            .map(|id| uploaded_file(&state, id).ok_or_else(|| AgentError::FileNotFound(id.clone())))
            .collect::<Result<Vec<_>, _>>()?;
        self.send_paths(user, session_id, text, files)
    }

    /// `send`, with attachments already turned into paths.
    fn send_paths(
        &self,
        user: &str,
        session_id: &str,
        text: &str,
        files: Vec<PathBuf>,
    ) -> Result<Sent, AgentError> {
        let state = self.me(user);
        if files.is_empty() && state.add_to_running_turn(session_id, text) {
            return Ok(Sent::Joined);
        }
        if !files.is_empty() && state.is_running(session_id) {
            return Err(AgentError::Busy);
        }
        let Some(hub) = self.me.upgrade() else {
            return Err(AgentError::Busy);
        };
        let (user, session_id, text) = (user.to_owned(), session_id.to_owned(), text.to_owned());
        tokio::spawn(async move { hub.run_turns(user, session_id, text, files).await });
        Ok(Sent::Started)
    }

    /// Run a turn, then any messages typed too late to join it, as further turns; then say the
    /// session is idle.
    async fn run_turns(
        self: Arc<Self>,
        user: String,
        session_id: String,
        text: String,
        files: Vec<PathBuf>,
    ) {
        self.check_full_auto(&user, &session_id).await;
        self.clone()
            .run_turns_until_done(&user, &session_id, text, files)
            .await;
        if !self.state().is_running(&session_id) {
            self.publish_status(&session_id, SessionState::Idle);
            self.task_ended_full_auto(&user, &session_id).await;
        }
        self.apply_pending_settings().await;
    }

    /// Change one setting in `config.toml` (see `thrall_core::config::save_setting`). It's used
    /// at once when nothing is running; otherwise once the running work finishes. Returns whether
    /// it's in use now.
    pub async fn save_setting(
        &self,
        key: &str,
        value: &serde_json::Value,
    ) -> Result<bool, AgentError> {
        let state = self.state();
        let Some(dir) = &state.data_dir else {
            return Err(AgentError::InputRefused(
                "This engine has no config file to change.".to_owned(),
            ));
        };
        thrall_core::config::save_setting(&dir.join(CONFIG_FILE), key, value)
            .map_err(|e| AgentError::InputRefused(e.to_string()))?;
        self.settings_pending.store(true, Ordering::SeqCst);
        self.apply_pending_settings().await;
        Ok(!self.settings_pending.load(Ordering::SeqCst))
    }

    /// `/login chatgpt`: open ChatGPT's sign-in page and wait (in the background) for the browser
    /// to come back. The result is announced in `session_id`, and the engine reloads with it.
    async fn login(&self, user: &str, session_id: &str, args: &str) -> Result<String, AgentError> {
        if !args.trim().eq_ignore_ascii_case("chatgpt") {
            return Ok(
                "Sign in to what? `/login chatgpt` uses your ChatGPT plan (Plus or Pro) \
                       for chat models named `chatgpt/<model>`."
                    .to_owned(),
            );
        }
        let refused = |e: String| AgentError::InputRefused(e);
        let attempt = thrall_llm::chatgpt::begin_sign_in().map_err(|e| refused(e.to_string()))?;
        let earlier = self.sign_in.lock().ok().and_then(|mut s| s.take());
        if let Some(earlier) = earlier {
            earlier.abort();
            // Let the earlier attempt's listeners close.
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
        let listeners = crate::signin::listen().await.map_err(refused)?;
        let Some(hub) = self.me.upgrade() else {
            return Err(AgentError::Busy);
        };
        let (person, session) = (user.to_owned(), session_id.to_owned());
        let (state_token, verifier) = (attempt.state.clone(), attempt.verifier.clone());
        let task = tokio::spawn(async move {
            let outcome = async {
                let code = crate::signin::wait_for_code(listeners, &state_token).await?;
                let tokens =
                    thrall_llm::chatgpt::finish_sign_in(&hub.state().http, &code, &verifier)
                        .await
                        .map_err(|e| e.to_string())?;
                crate::signin::save_chatgpt(&tokens.refresh_token, tokens.account_id.as_deref())
            }
            .await;
            let (level, text) = match outcome {
                Ok(()) => {
                    hub.settings_pending.store(true, Ordering::SeqCst);
                    hub.apply_pending_settings().await;
                    tracing::info!("signed in to ChatGPT");
                    (
                        NoticeLevel::Info,
                        format!(
                            "Signed in to ChatGPT. Chat on your plan with a model named \
                             `chatgpt/<model>`, e.g. `/model {CHATGPT_SUGGESTED_MODEL}`, or set \
                             it in Settings → Models."
                        ),
                    )
                }
                Err(e) => {
                    tracing::warn!(%e, "ChatGPT sign-in failed");
                    (NoticeLevel::Warning, format!("ChatGPT sign-in failed: {e}"))
                }
            };
            if !session.is_empty() {
                hub.notice(&person, &session, level, &text).await;
            }
        });
        if let Ok(mut slot) = self.sign_in.lock() {
            *slot = Some(task.abort_handle());
        }
        let opened = crate::signin::open_browser(&attempt.url);
        Ok(format!(
            "{} Sign in there; Thrall finishes by itself when the page says you're done \
             (within {} minutes).\n\n[ChatGPT sign-in page]({})",
            if opened {
                "I've opened ChatGPT's sign-in page in your browser."
            } else {
                "Open ChatGPT's sign-in page to continue (link below)."
            },
            SIGN_IN_TIMEOUT_SECS / 60,
            attempt.url
        ))
    }

    /// `/logout chatgpt`: forget the ChatGPT sign-in.
    async fn logout(&self, args: &str) -> Result<String, AgentError> {
        if !args.trim().eq_ignore_ascii_case("chatgpt") {
            return Ok(
                "Sign out of what? `/logout chatgpt` forgets the ChatGPT sign-in.".to_owned(),
            );
        }
        let had = crate::signin::forget_chatgpt().map_err(AgentError::InputRefused)?;
        if !had {
            return Ok("Thrall wasn't signed in to ChatGPT.".to_owned());
        }
        self.settings_pending.store(true, Ordering::SeqCst);
        self.apply_pending_settings().await;
        Ok(
            "Signed out of ChatGPT. Models named `chatgpt/…` won't work until you sign in again."
                .to_owned(),
        )
    }

    /// `/mcp login <server>` and `/mcp logout <server>`; `None` for anything else (the plain
    /// listing).
    async fn mcp_sign(
        &self,
        user: &str,
        session_id: &str,
        args: &str,
    ) -> Option<Result<String, AgentError>> {
        let mut words = args.split_whitespace();
        let action = words.next()?.to_lowercase();
        if action != "login" && action != "logout" {
            return None;
        }
        let Some(server) = words.next() else {
            return Some(Ok(format!(
                "{} which server? `/mcp {action} <name>`; `/mcp` lists them.",
                if action == "login" {
                    "Sign in to"
                } else {
                    "Sign out of"
                }
            )));
        };
        let state = self.me(user);
        // Signing in or out changes the service for everyone who shares it.
        if user != state.owner_id {
            return Some(Ok(
                "Only the person who runs this Thrall can sign it in to services or out of them."
                    .to_owned(),
            ));
        }
        let Some(config) = state.mcp.server(server).cloned() else {
            return Some(Ok(format!(
                "There's no MCP server called {server}. `/mcp` lists them."
            )));
        };
        if action == "logout" {
            let had = match thrall_mcp::remote::sign_out(server) {
                Ok(had) => had,
                Err(e) => return Some(Err(AgentError::InputRefused(e))),
            };
            state.mcp.reconnect(server).await;
            return Some(Ok(if had {
                format!("Signed out of {server}.")
            } else {
                format!("Thrall wasn't signed in to {server}.")
            }));
        }
        Some(self.mcp_login(user, session_id, server, &config).await)
    }

    /// Sign in to a web MCP server: open its sign-in page and wait (in the background) for the
    /// browser to come back. The result is announced in `session_id`.
    async fn mcp_login(
        &self,
        user: &str,
        session_id: &str,
        server: &str,
        config: &thrall_core::config::McpServerConfig,
    ) -> Result<String, AgentError> {
        let refused = |e: String| AgentError::InputRefused(e);
        let earlier = self.mcp_sign_in.lock().ok().and_then(|mut s| s.take());
        if let Some(earlier) = earlier {
            earlier.abort();
            // Let the earlier attempt's listener close.
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
        let redirect = crate::signin::mcp_redirect_uri();
        let attempt = thrall_mcp::remote::begin_sign_in(server, config, &redirect)
            .await
            .map_err(refused)?;
        let listeners = crate::signin::listen_for_mcp().await.map_err(refused)?;
        let Some(hub) = self.me.upgrade() else {
            return Err(AgentError::Busy);
        };
        let (person, session, name, url) = (
            user.to_owned(),
            session_id.to_owned(),
            server.to_owned(),
            attempt.url.clone(),
        );
        let task = tokio::spawn(async move {
            let outcome = async {
                let code = crate::signin::wait_for(
                    listeners,
                    thrall_mcp::constants::SIGN_IN_PATH,
                    &attempt.state,
                    &name,
                )
                .await?;
                attempt.finish(&code).await
            }
            .await;
            let (level, text) = match outcome {
                Ok(()) => {
                    hub.state().mcp.reconnect(&name).await;
                    tracing::info!(server = %name, "signed in to an MCP server");
                    (
                        NoticeLevel::Info,
                        format!("Signed in to {name}. Thrall can use its tools now."),
                    )
                }
                Err(e) => {
                    tracing::warn!(server = %name, %e, "MCP sign-in failed");
                    (
                        NoticeLevel::Warning,
                        format!("Signing in to {name} didn't work: {e}"),
                    )
                }
            };
            if !session.is_empty() {
                hub.notice(&person, &session, level, &text).await;
            }
        });
        if let Ok(mut slot) = self.mcp_sign_in.lock() {
            *slot = Some(task.abort_handle());
        }
        let opened = crate::signin::open_browser(&url);
        Ok(format!(
            "{} Sign in there; Thrall finishes by itself when the page says you're done (within \
             {} minutes).\n\n[{server} sign-in page]({url})",
            if opened {
                format!("I've opened {server}'s sign-in page in your browser.")
            } else {
                format!("Open {server}'s sign-in page to continue (link below).")
            },
            SIGN_IN_TIMEOUT_SECS / 60,
        ))
    }

    /// Whether saved settings are waiting for Thrall to be idle.
    pub fn settings_pending(&self) -> bool {
        self.settings_pending.load(Ordering::SeqCst)
    }

    /// Reload the engine with saved settings if any are waiting and nothing is running.
    async fn apply_pending_settings(&self) {
        if !self.settings_pending() || self.state().any_running() {
            return;
        }
        match self.reload().await {
            Ok(()) => self.settings_pending.store(false, Ordering::SeqCst),
            Err(e) => tracing::warn!(%e, "couldn't apply the saved settings"),
        }
    }

    /// Build a fresh engine from the config file and swap it in, keeping every session.
    async fn reload(&self) -> Result<(), String> {
        let Some(rebuild) = &self.rebuild else {
            return Err("reloading isn't available here".to_owned());
        };
        let fresh = Arc::new(rebuild().await?);
        let old = match self.state.write() {
            Ok(mut s) => std::mem::replace(&mut *s, fresh.clone()),
            Err(poisoned) => std::mem::replace(&mut *poisoned.into_inner(), fresh.clone()),
        };
        old.shut_down();
        self.attach(fresh);
        Ok(())
    }

    async fn run_turns_until_done(
        self: Arc<Self>,
        user: &str,
        session_id: &str,
        mut text: String,
        mut files: Vec<PathBuf>,
    ) {
        loop {
            let state = self.me(user);
            let (tx, mut rx) = mpsc::channel::<EventFrame>(EVENT_CHANNEL_CAPACITY);
            let hub = self.clone();
            let forward = tokio::spawn(async move {
                while let Some(frame) = rx.recv().await {
                    hub.publish(frame);
                }
            });
            let result = turn::run_with_files(&state, session_id, &text, &files, &tx).await;
            // Attachments belong to the first message only.
            files.clear();
            drop(tx);
            let _ = forward.await;
            match result {
                Ok(summary) => {
                    // Restarting stops Thrall for everyone: only the owner's turns may.
                    if summary.restart && user == state.owner_id {
                        self.restart(user, session_id).await;
                    } else if summary.restart {
                        self.notice(
                            user,
                            session_id,
                            NoticeLevel::Warning,
                            "Only the person who runs this Thrall can restart it.",
                        )
                        .await;
                    }
                    if summary.leftover.is_empty() || summary.status == TurnStatus::Failed {
                        return;
                    }
                    text = summary.leftover.join("\n\n");
                }
                // Another turn started first: join it instead.
                Err(AgentError::Busy) if state.add_to_running_turn(session_id, &text) => return,
                Err(e) => {
                    self.publish(EventFrame::new(
                        session_id,
                        None,
                        now_ms(),
                        Event::Error {
                            code: e.code(),
                            message: e.to_string(),
                            retry: false,
                        },
                    ));
                    return;
                }
            }
        }
    }

    /// Rebuild the engine from the config file, keeping every session.
    async fn restart(&self, user: &str, session_id: &str) {
        if self.rebuild.is_none() {
            self.notice(
                user,
                session_id,
                NoticeLevel::Warning,
                "Restarting isn't available here.",
            )
            .await;
            return;
        }
        match self.reload().await {
            Ok(()) => {
                self.settings_pending.store(false, Ordering::SeqCst);
                self.notice(user, session_id, NoticeLevel::Info, RESTARTED_NOTICE)
                    .await;
            }
            Err(e) => {
                self.notice(
                    user,
                    session_id,
                    NoticeLevel::Warning,
                    &format!("Restart failed, still running as before: {e}"),
                )
                .await;
            }
        }
    }

    async fn notice(&self, user: &str, session_id: &str, level: NoticeLevel, text: &str) {
        runners::announce(
            &self.me(user),
            session_id,
            Event::Notice {
                level,
                text: text.to_owned(),
            },
        )
        .await;
    }

    /// The facts Thrall remembers, newest first.
    pub async fn facts(&self, user: &str) -> Result<Vec<thrall_protocol::FactInfo>, AgentError> {
        let state = self.me(user);
        Ok(state
            .store
            .search_knowledge(&state.user_id, "", FACTS_LIST_LIMIT)
            .await?
            .into_iter()
            .map(|k| thrall_protocol::FactInfo {
                fact: k.id,
                topic: k.topic,
                content: k.content,
                at: k.updated_at,
            })
            .collect())
    }

    /// Forget a fact. False when there was no such fact.
    pub async fn forget(&self, user: &str, fact: &str) -> Result<bool, AgentError> {
        let state = self.me(user);
        Ok(state
            .store
            .forget_knowledge(&state.user_id, fact, now_ms())
            .await?)
    }

    /// The person's own skills that are turned on, as (name, description) for `/` autocomplete.
    pub fn skill_commands(&self, user: &str) -> Vec<(String, String)> {
        let state = self.me(user);
        let Some(data) = state.my_dir() else {
            return Vec::new();
        };
        thrall_tools::skills::personal(&data)
            .into_iter()
            .filter(|s| !thrall_tools::skills::is_disabled(&s.name, &state.config.skills.disabled))
            .map(|s| (s.name, s.description))
            .collect()
    }

    /// What rewinding to before `turn` would do to files (see `checkpoints::plan`).
    async fn rewind_plan(
        &self,
        user: &str,
        session_id: &str,
        turn: &str,
    ) -> Result<Vec<crate::checkpoints::Planned>, AgentError> {
        let state = self.me(user);
        let workspace = state
            .store
            .get_session(&state.user_id, session_id)
            .await?
            .ok_or_else(|| AgentError::SessionNotFound(session_id.to_owned()))?
            .workspace
            .map(PathBuf::from);
        crate::checkpoints::plan(&state, session_id, turn, workspace.as_deref()).await
    }

    /// What rewinding to before `turn` would do: the message that started it, and the files.
    pub async fn rewind_preview(
        &self,
        user: &str,
        session_id: &str,
        turn: &str,
    ) -> Result<(String, Vec<thrall_protocol::RewindFile>), AgentError> {
        let state = self.me(user);
        let typed = state
            .store
            .turn_messages(&state.user_id, session_id, turn)
            .await?;
        if state
            .store
            .turns_from(&state.user_id, session_id, turn)
            .await?
            .is_empty()
        {
            return Err(AgentError::TurnNotFound);
        }
        let files = self.rewind_plan(user, session_id, turn).await?;
        Ok((
            typed.join("\n\n"),
            files.into_iter().map(|p| p.file).collect(),
        ))
    }

    /// Go back to before `turn`: put files back (`files`, leaving those in `keep`) and/or take
    /// back it and every later turn (`conversation`). Returns the message that started it, what
    /// was put back, and what couldn't be.
    pub async fn rewind(
        &self,
        user: &str,
        session_id: &str,
        turn: &str,
        files: bool,
        conversation: bool,
        keep: &[String],
    ) -> Result<(String, Vec<String>, Vec<String>), AgentError> {
        let state = self.me(user);
        if state.is_running(session_id) {
            return Err(AgentError::Busy);
        }
        let turns = state
            .store
            .turns_from(&state.user_id, session_id, turn)
            .await?;
        if turns.is_empty() {
            return Err(AgentError::TurnNotFound);
        }
        let typed = state
            .store
            .turn_messages(&state.user_id, session_id, turn)
            .await?;
        let mut kept = Vec::new();
        let (restored, failed) = if files {
            let planned = self.rewind_plan(user, session_id, turn).await?;
            kept = planned
                .iter()
                .map(|p| p.file.path.clone())
                .filter(|p| keep.contains(p))
                .collect();
            crate::checkpoints::restore(planned, keep).await
        } else {
            (Vec::new(), Vec::new())
        };
        if conversation {
            let at = now_ms();
            for t in &turns {
                state
                    .store
                    .retract_turn(&state.user_id, session_id, t, at)
                    .await?;
            }
            runners::announce(
                &state,
                session_id,
                Event::TurnRetracted {
                    turn: turn.to_owned(),
                },
            )
            .await;
        }
        // Said by the engine, after the turns are gone, so every client shows it (and keeps it).
        let first: String = typed
            .first()
            .map(|t| {
                t.lines()
                    .next()
                    .unwrap_or_default()
                    .chars()
                    .take(REWIND_QUOTE_CHARS)
                    .collect()
            })
            .unwrap_or_default();
        let first = first.trim_end_matches(['.', '!', '?', ' ']);
        let mut said = vec![if conversation {
            format!("Rewound to before “{first}”.")
        } else {
            format!("Put files back to before “{first}”.")
        }];
        if files {
            said.push(if restored.is_empty() {
                "No files needed putting back.".to_owned()
            } else {
                format!("Put back {}.", restored.join(", "))
            });
        }
        if !kept.is_empty() {
            said.push(format!("Left as they are: {}.", kept.join(", ")));
        }
        if !failed.is_empty() {
            said.push(format!("Couldn't put back {}.", failed.join("; ")));
        }
        runners::announce(
            &state,
            session_id,
            Event::Notice {
                level: if failed.is_empty() {
                    NoticeLevel::Info
                } else {
                    NoticeLevel::Warning
                },
                text: said.join(" "),
            },
        )
        .await;
        Ok((typed.join("\n\n"), restored, failed))
    }

    /// Take back the last turn and send its message again, or `text` instead. The files it
    /// changed go back to how they were, except ones changed since by someone else.
    pub async fn retry(
        &self,
        user: &str,
        session_id: &str,
        text: Option<String>,
    ) -> Result<(), AgentError> {
        let state = self.me(user);
        if state.is_running(session_id) {
            return Err(AgentError::Busy);
        }
        let (turn, mut typed) = state
            .store
            .last_turn(&state.user_id, session_id)
            .await?
            .ok_or(AgentError::NothingToRetry)?;
        // The turn's file changes go back too, except files someone changed since.
        let planned = self.rewind_plan(user, session_id, &turn).await?;
        let kept: Vec<String> = planned
            .iter()
            .filter(|p| p.file.changed)
            .map(|p| p.file.path.clone())
            .collect();
        let (_, failed) = crate::checkpoints::restore(planned, &kept).await;
        let mut left: Vec<String> = kept
            .iter()
            .map(|p| format!("{p} (changed since)"))
            .collect();
        left.extend(failed);
        if !left.is_empty() {
            runners::announce(
                &state,
                session_id,
                Event::Notice {
                    level: NoticeLevel::Warning,
                    text: format!("These files were left as they are: {}.", left.join(", ")),
                },
            )
            .await;
        }
        // The files attached to the turn's messages go again (those still there).
        let mut files: Vec<PathBuf> = Vec::new();
        for message in state
            .store
            .context_messages(&state.user_id, session_id)
            .await?
            .into_iter()
            .filter(|m| m.turn_id.as_deref() == Some(turn.as_str()) && m.role == ROLE_USER)
        {
            if let Ok(content) = serde_json::from_str::<StoredContent>(&message.content) {
                files.extend(
                    content
                        .files
                        .into_iter()
                        .map(PathBuf::from)
                        .filter(|p| p.is_file()),
                );
            }
        }
        // Older messages showed their attachments as a last line; the files go as files instead.
        for message in &mut typed {
            if let Some(at) = message.rfind(ATTACHED_LINE) {
                message.truncate(at);
            }
        }
        // An edit replaces the last message (the one Edit shows); every message of the turn goes
        // again, including ones typed while it ran.
        if let Some(edited) = text.filter(|t| !t.trim().is_empty()) {
            typed.pop();
            typed.push(edited);
        }
        let text = typed.join("\n\n");
        state
            .store
            .retract_turn(&state.user_id, session_id, &turn, now_ms())
            .await?;
        runners::announce(&state, session_id, Event::TurnRetracted { turn }).await;
        self.send_paths(user, session_id, &text, files)?;
        Ok(())
    }

    /// What an uploaded recording says (the microphone button). The cost goes to `session`.
    pub async fn transcribe_upload(
        &self,
        user: &str,
        file: &str,
        session: Option<&str>,
    ) -> Result<String, AgentError> {
        let state = self.me(user);
        let path =
            uploaded_file(&state, file).ok_or_else(|| AgentError::FileNotFound(file.to_owned()))?;
        match crate::audio::listen(&state, &path, None).await {
            Ok((text, usage)) => {
                crate::audio::record_loose_usage(&state, session, usage).await?;
                Ok(text)
            }
            Err(e) => Err(AgentError::Transcription(e)),
        }
    }

    /// Stop the turn running in `session_id` (Esc). False if none is running, or the session
    /// isn't `user`'s.
    pub async fn interrupt(&self, user: &str, session_id: &str) -> bool {
        let state = self.me(user);
        session::get(&state, session_id).await.is_ok() && state.interrupt(session_id)
    }

    /// Answer a permission or plan request as `user`. False if someone answered first, it's
    /// gone, or it isn't theirs to answer.
    pub fn answer(&self, user: &str, request_id: &str, reply: Reply, answered_by: &str) -> bool {
        self.state()
            .prompts
            .answer(request_id, user, reply, answered_by)
    }

    /// Change a session's permission mode and tell every client. Full auto is the owner's only;
    /// `limit` says how long it lasts (until the task ends when absent), and switching it on in
    /// one chat switches the person's other Full auto chat back to Ask.
    pub async fn set_mode(
        &self,
        user: &str,
        session_id: &str,
        mode: Mode,
        limit: Option<ModeLimit>,
    ) -> Result<(), AgentError> {
        let state = self.me(user);
        session::get(&state, session_id).await?;
        if mode == Mode::FullAuto && user != state.owner_id {
            return Err(AgentError::InputRefused(FULL_AUTO_OWNER_ONLY.to_owned()));
        }
        if mode == Mode::FullAuto {
            let limit = limit.unwrap_or(ModeLimit {
                until: MODE_UNTIL_TASK_END.to_owned(),
                ends_at: None,
            });
            for other in self.start_full_auto(user, session_id, limit) {
                self.end_full_auto(user, &other, FULL_AUTO_ENDED_MOVED)
                    .await;
            }
        } else if let Some(old) = self.take_full_auto(session_id)
            && let Some(timer) = old.timer
        {
            timer.abort();
        }
        session::set_mode(&state, session_id, mode).await?;
        self.announce_mode(&state, session_id, mode).await;
        Ok(())
    }

    /// Tell every client a session's mode (and how long Full auto lasts).
    async fn announce_mode(&self, state: &AppState, session_id: &str, mode: Mode) {
        runners::announce(
            state,
            session_id,
            Event::SessionUpdated {
                title: None,
                mode: Some(mode.as_str().to_owned()),
                mode_limit: self.mode_limit(session_id),
                workspace: None,
            },
        )
        .await;
    }

    /// How long Full auto lasts in `session_id`; `None` when it isn't on.
    fn mode_limit(&self, session_id: &str) -> Option<ModeLimit> {
        self.full_auto
            .lock()
            .ok()?
            .get(session_id)
            .map(|f| f.limit.clone())
    }

    /// Record that `session_id` is in Full auto (starting its timer for timed Full auto). Returns
    /// the person's other chats that were in Full auto, now forgotten, to switch back to Ask.
    fn start_full_auto(&self, user: &str, session_id: &str, limit: ModeLimit) -> Vec<String> {
        let timer = limit.ends_at.and_then(|ends_at| {
            let hub = self.me.upgrade()?;
            let (user, session) = (user.to_owned(), session_id.to_owned());
            let wait = u64::try_from(ends_at - now_ms()).unwrap_or(0);
            Some(
                tokio::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_millis(wait)).await;
                    // Forget the entry first: ending it here must not abort this very task.
                    let still_this_one = hub
                        .full_auto
                        .lock()
                        .map(|mut m| {
                            let ours =
                                m.get(&session).and_then(|f| f.limit.ends_at) == Some(ends_at);
                            if ours {
                                m.remove(&session);
                            }
                            ours
                        })
                        .unwrap_or(false);
                    if still_this_one {
                        hub.back_to_ask(&user, &session, FULL_AUTO_ENDED_TIME).await;
                    }
                })
                .abort_handle(),
            )
        });
        let Ok(mut chats) = self.full_auto.lock() else {
            return Vec::new();
        };
        let others: Vec<String> = chats
            .iter()
            .filter(|(id, f)| f.user == user && id.as_str() != session_id)
            .map(|(id, _)| id.clone())
            .collect();
        if let Some(old) = chats.insert(
            session_id.to_owned(),
            FullAuto {
                user: user.to_owned(),
                limit,
                timer,
            },
        ) && let Some(timer) = old.timer
        {
            timer.abort();
        }
        others
    }

    /// Forget a chat's Full auto (without touching its mode), handing back what it was.
    fn take_full_auto(&self, session_id: &str) -> Option<FullAuto> {
        self.full_auto.lock().ok()?.remove(session_id)
    }

    /// End a chat's Full auto now, saying why.
    async fn end_full_auto(&self, user: &str, session_id: &str, why: &str) {
        if let Some(old) = self.take_full_auto(session_id)
            && let Some(timer) = old.timer
        {
            timer.abort();
        }
        self.back_to_ask(user, session_id, why).await;
    }

    /// Put a chat that's in Full auto back to Ask and say why. Leaves any other mode alone.
    async fn back_to_ask(&self, user: &str, session_id: &str, why: &str) {
        let state = self.me(user);
        let Ok(s) = session::get(&state, session_id).await else {
            return;
        };
        if session::mode_of(&s) != Mode::FullAuto {
            return;
        }
        if let Err(e) = session::set_mode(&state, session_id, Mode::Ask).await {
            tracing::warn!(%e, session = session_id, "couldn't end Full auto");
            return;
        }
        self.announce_mode(&state, session_id, Mode::Ask).await;
        self.notice(user, session_id, NoticeLevel::Info, why).await;
    }

    /// Before a turn: a chat in Full auto that this engine never switched on (left from before a
    /// restart, or not the owner's) goes back to Ask first.
    async fn check_full_auto(&self, user: &str, session_id: &str) {
        let known = self
            .full_auto
            .lock()
            .map(|m| m.contains_key(session_id))
            .unwrap_or(false);
        if !known || user != self.state().owner_id {
            self.end_full_auto(user, session_id, FULL_AUTO_ENDED_RESTART)
                .await;
        }
    }

    /// After a task: Full auto that lasts until the task ends, ends.
    async fn task_ended_full_auto(&self, user: &str, session_id: &str) {
        let ends = self
            .full_auto
            .lock()
            .map(|m| {
                m.get(session_id)
                    .is_some_and(|f| f.limit.until == MODE_UNTIL_TASK_END)
            })
            .unwrap_or(false);
        if ends {
            self.end_full_auto(user, session_id, FULL_AUTO_ENDED_TASK)
                .await;
        }
    }

    /// At startup: chats left in Full auto from before go back to Ask (Full auto never lasts past
    /// a restart).
    pub async fn end_full_auto_from_before(&self) {
        let state = self.state();
        let people = match state.store.list_people().await {
            Ok(people) => people,
            Err(e) => {
                tracing::warn!(%e, "couldn't check for chats left in Full auto");
                return;
            }
        };
        for person in people {
            let mine = self.me(&person.id);
            let sessions = match mine
                .store
                .list_sessions(&person.id, FULL_AUTO_SWEEP_SESSIONS)
                .await
            {
                Ok(sessions) => sessions,
                Err(e) => {
                    tracing::warn!(%e, "couldn't check for chats left in Full auto");
                    continue;
                }
            };
            for s in sessions
                .iter()
                .filter(|s| session::mode_of(s) == Mode::FullAuto)
            {
                if let Err(e) = session::set_mode(&mine, &s.id, Mode::Ask).await {
                    tracing::warn!(%e, session = s.id, "couldn't end Full auto");
                }
            }
        }
    }

    /// Rename a session and tell every client.
    pub async fn rename(
        &self,
        user: &str,
        session_id: &str,
        title: &str,
    ) -> Result<(), AgentError> {
        self.not_home(user, session_id).await?;
        let state = self.me(user);
        state
            .store
            .set_session_title(&state.user_id, session_id, title)
            .await?;
        runners::announce(
            &state,
            session_id,
            Event::SessionUpdated {
                title: Some(title.to_owned()),
                mode: None,
                mode_limit: None,
                workspace: None,
            },
        )
        .await;
        Ok(())
    }

    /// Archive a session or bring it back.
    pub async fn archive(
        &self,
        user: &str,
        session_id: &str,
        archived: bool,
    ) -> Result<(), AgentError> {
        self.not_home(user, session_id).await?;
        let state = self.me(user);
        let at = archived.then(now_ms);
        state
            .store
            .set_session_archived(&state.user_id, session_id, at)
            .await?;
        Ok(())
    }

    /// Pin a session to the top, or unpin it.
    pub async fn pin(&self, user: &str, session_id: &str, pinned: bool) -> Result<(), AgentError> {
        self.not_home(user, session_id).await?;
        let state = self.me(user);
        state
            .store
            .set_session_pinned(&state.user_id, session_id, pinned.then(now_ms))
            .await?;
        Ok(())
    }

    /// Put a session in a group, or take it out.
    pub async fn move_to_group(
        &self,
        user: &str,
        session_id: &str,
        group: Option<&str>,
    ) -> Result<(), AgentError> {
        self.not_home(user, session_id).await?;
        let state = self.me(user);
        state
            .store
            .set_session_group(&state.user_id, session_id, group)
            .await?;
        Ok(())
    }

    /// Move a session to another working folder and tell every client. The folder as stored, or
    /// why it can't move (not while Thrall works in it; the folder must exist).
    pub async fn set_folder(
        &self,
        user: &str,
        session_id: &str,
        folder: &str,
    ) -> Result<Result<String, String>, AgentError> {
        self.not_home(user, session_id).await?;
        let state = self.me(user);
        if state.is_running(session_id) {
            return Ok(Err(
                "Thrall is working in this session; move it when he's done.".to_owned(),
            ));
        }
        let moved = session::set_folder(&state, session_id, folder).await?;
        if let Ok(path) = &moved {
            runners::announce(
                &state,
                session_id,
                Event::SessionUpdated {
                    title: None,
                    mode: None,
                    mode_limit: None,
                    workspace: Some(path.clone()),
                },
            )
            .await;
        }
        Ok(moved)
    }

    /// The person's groups, in order.
    pub async fn groups(&self, user: &str) -> Result<Vec<thrall_protocol::GroupInfo>, AgentError> {
        let state = self.me(user);
        Ok(state
            .store
            .list_groups(&state.user_id)
            .await?
            .into_iter()
            .map(|g| thrall_protocol::GroupInfo {
                group: g.id,
                name: g.name,
            })
            .collect())
    }

    /// Add a group at the end.
    pub async fn create_group(&self, user: &str, name: &str) -> Result<(), AgentError> {
        let state = self.me(user);
        let id = thrall_core::ids::new_id(thrall_core::constants::id_prefix::GROUP);
        state
            .store
            .create_group(&state.user_id, &id, name, now_ms())
            .await?;
        Ok(())
    }

    /// Rename a group.
    pub async fn rename_group(
        &self,
        user: &str,
        group: &str,
        name: &str,
    ) -> Result<(), AgentError> {
        let state = self.me(user);
        state
            .store
            .rename_group(&state.user_id, group, name)
            .await?;
        Ok(())
    }

    /// Delete a group; its sessions stay, ungrouped.
    pub async fn delete_group(&self, user: &str, group: &str) -> Result<(), AgentError> {
        let state = self.me(user);
        state.store.delete_group(&state.user_id, group).await?;
        Ok(())
    }

    /// Put the groups in this order.
    pub async fn reorder_groups(&self, user: &str, groups: &[String]) -> Result<(), AgentError> {
        let state = self.me(user);
        state.store.reorder_groups(&state.user_id, groups).await?;
        Ok(())
    }

    /// Delete a session and its history. Refused while a turn runs in it.
    pub async fn delete(&self, user: &str, session_id: &str) -> Result<(), AgentError> {
        self.not_home(user, session_id).await?;
        let state = self.me(user);
        if state.is_running(session_id) {
            return Err(AgentError::Busy);
        }
        state
            .store
            .delete_session(&state.user_id, session_id)
            .await?;
        Ok(())
    }

    /// Run a slash command; the answer is markdown. Changes it makes to a session are announced
    /// to every client.
    pub async fn command(
        &self,
        user: &str,
        session_id: Option<&str>,
        name: &str,
        args: &str,
    ) -> Result<String, AgentError> {
        let state = self.me(user);
        let session_id = session_id.unwrap_or_default();
        match name {
            "login" => return self.login(user, session_id, args).await,
            "logout" => return self.logout(args).await,
            "mcp" => {
                if let Some(result) = self.mcp_sign(user, session_id, args).await {
                    return result;
                }
            }
            // Through set_mode, so Full auto is the owner's only and gets its time limit.
            "mode" if !session_id.is_empty() => {
                if let Some(mode) = Mode::parse(args) {
                    return match self.set_mode(user, session_id, mode, None).await {
                        Ok(()) => commands::mode_changed(mode),
                        Err(AgentError::InputRefused(why)) => Ok(why),
                        Err(e) => Err(e),
                    };
                }
            }
            _ => {}
        }
        let markdown = commands::run(&state, session_id, name, args).await?;
        let changed = !args.trim().is_empty() && !session_id.is_empty();
        match name {
            "title" | "folder" if changed => {
                let s = session::get(&state, session_id).await?;
                runners::announce(
                    &state,
                    session_id,
                    Event::SessionUpdated {
                        title: (name == "title").then(|| s.title.clone()),
                        mode: None,
                        mode_limit: None,
                        workspace: if name == "folder" {
                            s.workspace.clone()
                        } else {
                            None
                        },
                    },
                )
                .await;
            }
            "model" if changed => self.publish_status(session_id, self.state_of(session_id)),
            _ => {}
        }
        Ok(markdown)
    }
}
