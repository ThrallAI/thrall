//! `AppState`: everything a turn needs, built once at startup and passed explicitly.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use thrall_core::Config;
use thrall_llm::LlmProvider;
use thrall_protocol::EventFrame;
use thrall_store::Store;
use thrall_tools::{Cancel, Interrupter, ReadLog, Tool};
use tokio::sync::{broadcast, mpsc};

use thrall_tools::tools::{MCP_CALL, MCP_SEARCH};

use crate::constants::NOTICE_CHANNEL_CAPACITY;
use crate::prompts::Prompts;

/// A check-in running in a session.
#[derive(Debug, Clone, Default)]
pub struct CheckIn {
    /// Its model ("" = the chat model).
    pub model: String,
    /// What the model is told besides the message (the checklist and the rules).
    pub note: String,
}

/// A background helper waiting to start.
pub struct BackgroundTask {
    /// Whose session it works for (it runs as them).
    pub user_id: String,
    /// Helper id.
    pub agent_id: String,
    /// Session it works for.
    pub session_id: String,
    /// Folder it works in.
    pub workspace: PathBuf,
    /// What it's doing.
    pub title: String,
    /// Its instructions.
    pub task: String,
    /// Its helper type, if one was asked for.
    pub kind: Option<thrall_tools::agents::AgentType>,
}

/// A turn in progress: how to stop it, and messages typed while it works.
struct Running {
    interrupter: Interrupter,
    inbox: Vec<String>,
}

/// The engine's state, as one person sees it. No globals: whatever needs this receives it as an
/// argument. Everything but `user_id` is the engine's and shared (a clone shares it); `for_user`
/// makes the same engine act for another person, so whatever runs with that copy (a turn, a
/// request) reads and writes only that person's rows.
#[derive(Clone)]
pub struct AppState {
    /// Settings from `config.toml`.
    pub config: Config,
    /// Storage.
    pub store: Arc<dyn Store>,
    /// The AI provider, as this copy's person uses it (their key, their limit; see `people`).
    pub llm: Arc<dyn LlmProvider>,
    /// The engine's own AI provider (the owner's key), which `llm` wraps.
    base_llm: Arc<dyn LlmProvider>,
    /// Everyone's terms (own key, model, monthly limit), kept in step with storage.
    terms: crate::people::Terms,
    /// People's own AI keys.
    pub own_keys: Arc<dyn crate::people::OwnKeys>,
    /// The person this copy acts for. The state an engine is built with acts for its owner.
    pub user_id: String,
    /// The engine's owner (who it was built for; or_user doesn't change it).
    pub owner_id: String,
    /// Tools the model may call.
    pub tools: Vec<Arc<dyn Tool>>,
    /// HTTP client shared by web tools.
    pub http: reqwest::Client,
    /// Permission and plan requests waiting for an answer.
    pub prompts: Arc<Prompts>,
    /// Thrall's data folder, if there is one. Things that are a person's own (THRALL.md, skills,
    /// uploads, Home's folder, the browser's profile) are in `my_dir`.
    pub data_dir: Option<PathBuf>,
    /// Thrall's working folder (`folders.new_in`, else `Documents\Thrall`), where Home's folder
    /// is. `None`: Home works in the data folder, as before 0.2.2.
    pub work_dir: Option<PathBuf>,
    /// The MCP servers this person may use (started on first use): all of them for the owner,
    /// only those shared with everyone for anyone else.
    pub mcp: Arc<thrall_mcp::McpHub>,
    /// Every configured MCP server (the owner's view, which members' views share connections
    /// with).
    mcp_all: Arc<thrall_mcp::McpHub>,
    /// Long-term memory. Searches by words only until an embedder is set (see `with_embedder`).
    pub memory: Arc<thrall_memory::Memory>,
    /// The browser (started on first use; a tab per session).
    pub browsers: Arc<thrall_tools::browser::Browsers>,
    /// Companion PCs' runners connected now, and the sessions working on them.
    pub companions: Arc<crate::companions::Companions>,
    /// Keys for services besides the AI provider (fal.ai).
    pub keys: Arc<dyn crate::keys::KeyStore>,
    /// The model in use; starts as the config's, `/model` changes it until restart.
    model: Arc<RwLock<String>>,
    /// Turns in progress, by session.
    running: Arc<Mutex<HashMap<String, Running>>>,
    /// Files read per session, for the edit guard.
    reads: Arc<Mutex<HashMap<String, ReadLog>>>,
    /// Real tokens per 100 estimated, per session, as the provider last counted them; keeps
    /// compaction's estimate honest (see `thrall_memory::compaction::token_scale`).
    token_scales: Arc<Mutex<HashMap<String, usize>>>,
    /// Events that happen outside a turn (background helpers, scheduled runs).
    pub notices: broadcast::Sender<EventFrame>,
    /// Background helpers waiting for the runner.
    background_tx: mpsc::UnboundedSender<BackgroundTask>,
    pub(crate) background_rx: Arc<Mutex<Option<mpsc::UnboundedReceiver<BackgroundTask>>>>,
    /// Whether the background and scheduler runners are going.
    pub(crate) runners_started: Arc<AtomicBool>,
    /// Set when this engine is being replaced (restart); its runners stop.
    pub(crate) stopping: Arc<AtomicBool>,
    /// Sessions running a scheduled job: nobody is there to answer questions.
    unattended: Arc<Mutex<HashSet<String>>>,
    /// Sessions running a check-in, with the model it uses ("" = the chat model). Check-ins
    /// only look: anything that would change something is refused.
    check_ins: Arc<Mutex<HashMap<String, CheckIn>>>,
    /// When the last check-in started, Unix ms (the engine's start, before the first).
    pub(crate) last_beat: Arc<AtomicI64>,
    /// Held while Home's current page is found or started, so only one fresh page starts.
    pub(crate) home_lock: Arc<tokio::sync::Mutex<()>>,
}

impl AppState {
    /// State with the core tools.
    pub fn new(
        config: Config,
        store: Arc<dyn Store>,
        llm: Arc<dyn LlmProvider>,
        user_id: String,
    ) -> reqwest::Result<Self> {
        let (background_tx, background_rx) = mpsc::unbounded_channel();
        let (notices, _) = broadcast::channel(NOTICE_CHANNEL_CAPACITY);
        let mcp_servers = config.mcp.servers.clone();
        let mcp_all = Arc::new(thrall_mcp::McpHub::new(mcp_servers.clone()));
        let memory = Arc::new(thrall_memory::Memory::new(
            store.clone(),
            None,
            config.memory.embedding_model.clone(),
            config.provider.kind.as_str(),
        ));
        let terms = crate::people::Terms::default();
        let own_keys: Arc<dyn crate::people::OwnKeys> = Arc::new(crate::people::NoOwnKeys);
        Ok(Self {
            model: Arc::new(RwLock::new(config.provider.model.clone())),
            config,
            llm: Arc::new(crate::people::PersonLlm::new(
                &user_id,
                llm.clone(),
                store.clone(),
                terms.clone(),
                own_keys.clone(),
            )),
            store,
            base_llm: llm,
            terms,
            own_keys,
            owner_id: user_id.clone(),
            user_id,
            tools: {
                let mut tools = thrall_tools::tools::core();
                // MCP tools only when there's a server to use them on.
                if !mcp_servers.is_empty() {
                    tools.push(Arc::new(thrall_tools::tools::McpSearch));
                    tools.push(Arc::new(thrall_tools::tools::McpCall));
                }
                tools
            },
            mcp: mcp_all.clone(),
            mcp_all,
            memory,
            browsers: Arc::new(thrall_tools::browser::Browsers::default()),
            companions: Arc::new(crate::companions::Companions::default()),
            keys: Arc::new(crate::keys::SystemKeys),
            http: thrall_tools::http_client()?,
            prompts: Arc::new(Prompts::default()),
            data_dir: None,
            work_dir: None,
            running: Arc::new(Mutex::new(HashMap::new())),
            reads: Arc::new(Mutex::new(HashMap::new())),
            token_scales: Arc::new(Mutex::new(HashMap::new())),
            notices,
            background_tx,
            background_rx: Arc::new(Mutex::new(Some(background_rx))),
            runners_started: Arc::new(AtomicBool::new(false)),
            stopping: Arc::new(AtomicBool::new(false)),
            unattended: Arc::new(Mutex::new(HashSet::new())),
            check_ins: Arc::new(Mutex::new(HashMap::new())),
            last_beat: Arc::new(AtomicI64::new(thrall_core::ids::now_ms())),
            home_lock: Arc::new(tokio::sync::Mutex::new(())),
        })
    }

    /// The same engine, acting for `user_id`.
    pub fn for_user(&self, user_id: &str) -> Self {
        let mut me = self.clone();
        user_id.clone_into(&mut me.user_id);
        me.rewrap();
        if me.user_id == me.owner_id {
            me.mcp = me.mcp_all.clone();
        } else {
            me.mcp = Arc::new(me.mcp_all.shared_view());
            // Nothing shared: no MCP tools to offer.
            if me.mcp.is_empty() {
                me.tools
                    .retain(|t| t.name() != MCP_SEARCH && t.name() != MCP_CALL);
            }
        }
        me
    }

    /// Keep people's own keys in `own_keys`.
    pub fn with_own_keys(mut self, own_keys: Arc<dyn crate::people::OwnKeys>) -> Self {
        self.own_keys = own_keys;
        self.rewrap();
        self
    }

    /// Wrap the engine's provider for this copy's person.
    fn rewrap(&mut self) {
        self.llm = Arc::new(crate::people::PersonLlm::new(
            &self.user_id,
            self.base_llm.clone(),
            self.store.clone(),
            self.terms.clone(),
            self.own_keys.clone(),
        ));
    }

    /// Read everyone's terms from storage (at startup).
    pub async fn load_terms(&self) -> Result<(), thrall_store::StoreError> {
        let people = self.store.list_people().await?;
        let mut terms = self
            .terms
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        terms.clear();
        for p in people {
            terms.insert(
                p.id,
                thrall_store::PersonTerms {
                    own_key: p.own_key,
                    model: p.model,
                    monthly_limit_micros: p.monthly_limit_micros,
                },
            );
        }
        Ok(())
    }

    /// A person's terms as in force now.
    pub fn terms_of(&self, person: &str) -> thrall_store::PersonTerms {
        self.terms
            .read()
            .ok()
            .and_then(|t| t.get(person).cloned())
            .unwrap_or_default()
    }

    /// Put a person's new terms in force (after storing them).
    pub fn set_terms(&self, person: &str, terms: thrall_store::PersonTerms) {
        self.terms
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(person.to_owned(), terms);
    }

    /// The folder for this person's own things: the data folder for the owner (as before people
    /// existed), `people/<id>` in it for anyone else. None without a data folder, or for an id
    /// that isn't a plain name.
    pub fn my_dir(&self) -> Option<PathBuf> {
        let data = self.data_dir.as_ref()?;
        if self.user_id == self.owner_id {
            return Some(data.clone());
        }
        let plain = !self.user_id.is_empty()
            && self
                .user_id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
        plain.then(|| {
            data.join(thrall_core::constants::PEOPLE_DIR)
                .join(&self.user_id)
        })
    }

    /// Hear about events outside turns (background helpers, scheduled runs).
    pub fn subscribe(&self) -> broadcast::Receiver<EventFrame> {
        self.notices.subscribe()
    }

    /// Stop this engine's runners (it's being replaced by a restart).
    pub fn shut_down(&self) {
        self.stopping.store(true, Ordering::SeqCst);
    }

    /// Stop the programs Thrall left running in the background (`shell` with `keep_running`).
    pub async fn stop_background(&self) {
        thrall_tools::background::stop_all().await;
    }

    /// Whether background helpers can run here (the runners are going).
    pub fn can_run_background(&self) -> bool {
        self.runners_started.load(Ordering::SeqCst)
    }

    pub(crate) fn queue_background(&self, task: BackgroundTask) -> bool {
        self.background_tx.send(task).is_ok()
    }

    /// Whether nobody is there to answer questions in session_id (a scheduled run).
    /// Whether a check-in is running in `session_id`.
    pub fn is_check_in(&self, session_id: &str) -> bool {
        self.check_ins
            .lock()
            .map(|c| c.contains_key(session_id))
            .unwrap_or(false)
    }

    /// Mark (`Some`) or unmark a check-in running in `session_id`.
    pub(crate) fn set_check_in(&self, session_id: &str, check_in: Option<CheckIn>) {
        if let Ok(mut c) = self.check_ins.lock() {
            match check_in {
                Some(ci) => {
                    c.insert(session_id.to_owned(), ci);
                }
                None => {
                    c.remove(session_id);
                }
            }
        }
    }

    /// The model a turn in `session_id` uses: a check-in's own, or the one in use.
    pub fn model_for(&self, session_id: &str) -> String {
        let own = self
            .check_ins
            .lock()
            .ok()
            .and_then(|c| c.get(session_id).map(|ci| ci.model.clone()))
            .filter(|m| !m.trim().is_empty());
        own.unwrap_or_else(|| self.model())
    }

    /// What a check-in running in `session_id` tells the model, if one is.
    pub(crate) fn check_in_note(&self, session_id: &str) -> Option<String> {
        self.check_ins
            .lock()
            .ok()
            .and_then(|c| c.get(session_id).map(|ci| ci.note.clone()))
    }

    pub fn is_unattended(&self, session_id: &str) -> bool {
        self.unattended
            .lock()
            .map(|u| u.contains(session_id))
            .unwrap_or(false)
    }

    pub(crate) fn set_unattended(&self, session_id: &str, on: bool) {
        if let Ok(mut u) = self.unattended.lock() {
            if on {
                u.insert(session_id.to_owned());
            } else {
                u.remove(session_id);
            }
        }
    }

    /// Turn on searching memory by meaning (vectors from `embedder`).
    pub fn with_embedder(mut self, embedder: Arc<dyn thrall_llm::Embedder>) -> Self {
        self.memory = Arc::new(thrall_memory::Memory::new(
            self.store.clone(),
            Some(embedder),
            self.config.memory.embedding_model.clone(),
            self.config.provider.kind.as_str(),
        ));
        self
    }

    /// The model turns use now: this copy's person's own, or the engine's.
    pub fn model(&self) -> String {
        if let Some(own) = self.terms_of(&self.user_id).model {
            return own;
        }
        match self.model.read() {
            Ok(m) => m.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    /// Switch model (until restart).
    pub fn set_model(&self, model: &str) {
        match self.model.write() {
            Ok(mut m) => *m = model.to_owned(),
            Err(poisoned) => *poisoned.into_inner() = model.to_owned(),
        }
    }

    /// A tool by name.
    pub fn tool(&self, name: &str) -> Option<&Arc<dyn Tool>> {
        self.tools.iter().find(|t| t.name() == name)
    }

    /// Stop the turn running in `session_id` (Esc). False if none is running.
    pub fn interrupt(&self, session_id: &str) -> bool {
        match self.running.lock() {
            Ok(running) => match running.get(session_id) {
                Some(r) => {
                    r.interrupter.interrupt();
                    true
                }
                None => false,
            },
            Err(_) => false,
        }
    }

    /// Hand a message to the turn running in `session_id`; it joins at the next step. False if no
    /// turn is running (send it as a new turn instead).
    pub fn add_to_running_turn(&self, session_id: &str, text: &str) -> bool {
        match self.running.lock() {
            Ok(mut running) => match running.get_mut(session_id) {
                Some(r) => {
                    r.inbox.push(text.to_owned());
                    true
                }
                None => false,
            },
            Err(_) => false,
        }
    }

    /// Whether a turn is running in `session_id`.
    pub fn is_running(&self, session_id: &str) -> bool {
        self.running
            .lock()
            .map(|r| r.contains_key(session_id))
            .unwrap_or(false)
    }

    /// Whether a turn is running in any session.
    pub fn any_running(&self) -> bool {
        self.running.lock().map(|r| !r.is_empty()).unwrap_or(false)
    }

    /// Register a turn starting in `session_id`; `None` if one is already running.
    pub(crate) fn start_running(&self, session_id: &str) -> Option<Cancel> {
        let mut running = self.running.lock().ok()?;
        if running.contains_key(session_id) {
            return None;
        }
        let (interrupter, cancel) = Cancel::pair();
        running.insert(
            session_id.to_owned(),
            Running {
                interrupter,
                inbox: Vec::new(),
            },
        );
        Some(cancel)
    }

    /// Messages typed since the last look.
    pub(crate) fn take_inbox(&self, session_id: &str) -> Vec<String> {
        self.running
            .lock()
            .ok()
            .and_then(|mut r| r.get_mut(session_id).map(|t| std::mem::take(&mut t.inbox)))
            .unwrap_or_default()
    }

    /// The turn in `session_id` ended; returns anything typed too late to join it.
    pub(crate) fn stop_running(&self, session_id: &str) -> Vec<String> {
        self.running
            .lock()
            .ok()
            .and_then(|mut r| r.remove(session_id))
            .map(|t| t.inbox)
            .unwrap_or_default()
    }

    /// Real tokens per 100 estimated in `session_id`'s context, as last measured; 100 (the
    /// estimate as it is) until the provider has counted one.
    pub(crate) fn token_scale(&self, session_id: &str) -> usize {
        self.token_scales
            .lock()
            .ok()
            .and_then(|s| s.get(session_id).copied())
            .unwrap_or(thrall_memory::constants::TOKEN_SCALE_DEFAULT_PERCENT)
    }

    /// Remember `session_id`'s latest measured token scale.
    pub(crate) fn set_token_scale(&self, session_id: &str, scale: usize) {
        if let Ok(mut s) = self.token_scales.lock() {
            s.insert(session_id.to_owned(), scale);
        }
    }

    /// The edit guard's record of files read in `session_id`.
    pub fn reads_for(&self, session_id: &str) -> ReadLog {
        match self.reads.lock() {
            Ok(mut reads) => reads.entry(session_id.to_owned()).or_default().clone(),
            Err(_) => ReadLog::default(),
        }
    }
}
