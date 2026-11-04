//! One client's WebSocket connection (protocol §2–§13): `hello` first, then requests in and
//! events out. Each request gets exactly one reply. Events go to the sessions the client has
//! opened; a client that falls behind catches up from the event log, so nothing stored is lost.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use axum::extract::ws::{Message, WebSocket};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use futures::{SinkExt, StreamExt};
use thrall_agent::hub::uploaded_file;
use thrall_agent::{Hub, Mode, Reply as Answer, commands, session};
use thrall_core::constants::id_prefix;
use thrall_core::ids::{new_id, now_ms};
use thrall_protocol::constants::{
    DEAD_CONNECTION_SECS, FILE_CHUNK_BYTES, HELLO_TIMEOUT_SECS, MAX_FRAME_BYTES, PROTOCOL_VERSION,
};
use thrall_protocol::{
    ClientKind, CommandItem, EngineInfo, ErrorCode, Event, EventFrame, Limits, Reply, ReplyFrame,
    Request, RequestFrame, ServerFrame, UserInfo,
};
use tokio::sync::broadcast::error::RecvError;

use crate::constants::{
    CAPABILITIES, DEFAULT_SESSION_LIST, MAX_UPLOAD_BYTES, PHONE_APP_FILE_ID, REPLAY_PAGE,
    SILENCE_CHECK, SKILL_ARGS_HINT,
};
use crate::uploads;

/// What every connection shares.
pub struct Shared {
    /// The engine.
    pub hub: Arc<Hub>,
    /// The access token clients must present.
    pub token: String,
    /// Engine version, for `welcome`.
    pub version: String,
    /// Connections open now (an engine started on demand stops when nobody's left).
    pub clients: AtomicUsize,
    /// Signalled when a client asks the engine to stop (`engine.stop`).
    pub stop: tokio::sync::Notify,
    /// Paired phones and the phone link.
    pub remote: Arc<crate::remote::Remote>,
    /// Sign-in links and signed-in browsers.
    pub signins: crate::signin::SignIns,
    /// Bumped by `update.install`; the owner's desktop app hears it and installs the update.
    pub update_asked: tokio::sync::watch::Sender<u64>,
    /// The owner's desktop apps connected now (`update.install` needs one).
    pub desktops: AtomicUsize,
}

/// Counts a connection for as long as it lives.
struct Counted<'a>(&'a AtomicUsize);

impl<'a> Counted<'a> {
    fn new(n: &'a AtomicUsize) -> Self {
        n.fetch_add(1, Ordering::SeqCst);
        Self(n)
    }
}

impl Drop for Counted<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// What arrives on a connection, whatever carries it.
pub enum Incoming {
    /// A protocol frame.
    Text(String),
    /// A sign of life only (pings, pongs, frames that aren't text).
    Alive,
}

/// Who is on the other end, as far as the link itself can tell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Peer {
    /// A client on this computer: it shows the access token in `hello`.
    Local,
    /// A paired phone, already proved by its key on the encrypted link.
    Phone {
        /// The phone's id.
        id: String,
    },
}

/// Serve one local WebSocket connection until it closes.
pub async fn serve(socket: WebSocket, shared: Arc<Shared>) {
    let (sink, stream) = socket.split();
    let incoming = stream
        .map(|msg| match msg {
            Ok(Message::Text(text)) => Some(Incoming::Text(text.as_str().to_owned())),
            Ok(Message::Close(_)) | Err(_) => None,
            Ok(_) => Some(Incoming::Alive),
        })
        .take_while(|m| std::future::ready(m.is_some()))
        .filter_map(std::future::ready);
    let outgoing = sink
        .with(|text: String| std::future::ready(Ok::<_, axum::Error>(Message::Text(text.into()))));
    serve_link(incoming, outgoing, shared, Peer::Local).await;
}

/// Serve one connection, over any link that carries protocol frames as text, until it closes.
pub async fn serve_link<I, O>(incoming: I, outgoing: O, shared: Arc<Shared>, peer: Peer)
where
    I: futures::Stream<Item = Incoming>,
    O: futures::Sink<String>,
{
    let mut stream = std::pin::pin!(incoming);
    let mut sink = std::pin::pin!(outgoing);
    let Some((kind, person, device)) = greet(&mut sink, &mut stream, &shared, &peer).await else {
        let _ = sink.close().await;
        return;
    };
    tracing::info!(client = kind.as_str(), ?peer, "client connected");
    let _counted = Counted::new(&shared.clients);
    let mut conn = Connection {
        shared: shared.clone(),
        kind,
        peer,
        attached: HashMap::new(),
        watch_all: false,
        uploads: HashSet::new(),
        person,
        device,
        whose: HashMap::new(),
    };
    let mut bus = shared.hub.subscribe();
    let mut signed_out = shared.signins.signed_out.subscribe();
    let owners_desktop = kind == ClientKind::Desktop
        && conn.person.role == thrall_core::constants::person_role::OWNER;
    let _desktop = owners_desktop.then(|| Counted::new(&shared.desktops));
    let mut update_asked = shared.update_asked.subscribe();
    let mut heard = Instant::now();
    let mut check = tokio::time::interval(SILENCE_CHECK);
    loop {
        let out = tokio::select! {
            msg = stream.next() => match msg {
                Some(Incoming::Text(text)) => {
                    heard = Instant::now();
                    conn.on_text(&text).await
                }
                Some(Incoming::Alive) => {
                    heard = Instant::now();
                    Vec::new()
                }
                None => break,
            },
            frame = bus.recv() => match frame {
                Ok(frame) => conn.on_event(frame).await.into_iter().collect(),
                Err(RecvError::Lagged(missed)) => {
                    tracing::warn!(missed, client = kind.as_str(), "client fell behind; catching up");
                    conn.catch_up().await
                }
                Err(RecvError::Closed) => break,
            },
            Ok(()) = update_asked.changed(), if owners_desktop => {
                vec![ServerFrame::Event(EventFrame::new(
                    "",
                    None,
                    now_ms(),
                    Event::UpdateRequested {},
                ))]
            }
            _ = signed_out.changed() => {
                if !conn.still_signed_in().await {
                    tracing::info!(client = kind.as_str(), "browser signed out; closing");
                    break;
                }
                Vec::new()
            }
            _ = check.tick() => {
                if heard.elapsed().as_secs() > DEAD_CONNECTION_SECS {
                    tracing::info!(client = kind.as_str(), "connection went quiet; closing");
                    break;
                }
                Vec::new()
            }
        };
        if send_all(&mut sink, out).await.is_err() {
            break;
        }
    }
    let _ = sink.close().await;
    tracing::info!(client = kind.as_str(), "client disconnected");
}

async fn send_all<O>(sink: &mut O, frames: Vec<ServerFrame>) -> Result<(), O::Error>
where
    O: futures::Sink<String> + Unpin,
{
    for frame in frames {
        match serde_json::to_string(&frame) {
            Ok(text) => sink.send(text).await?,
            Err(e) => tracing::warn!(%e, "couldn't encode a frame"),
        }
    }
    Ok(())
}

fn reply(re: &str, reply: Reply) -> ServerFrame {
    ServerFrame::Reply(ReplyFrame::new(re, reply))
}

fn error(re: &str, code: ErrorCode, message: impl Into<String>) -> ServerFrame {
    ServerFrame::Reply(ReplyFrame::error(re, code, message))
}

/// `file.get`: one piece of an attached file, from `offset`.
async fn file_get(
    id: &str,
    state: &thrall_agent::AppState,
    file: &str,
    offset: u64,
) -> ServerFrame {
    let Some(path) = uploaded_file(state, file) else {
        return error(id, ErrorCode::NotFound, "There's no such file.");
    };
    let read = async {
        use tokio::io::{AsyncReadExt, AsyncSeekExt};
        let mut f = tokio::fs::File::open(&path).await?;
        let size = f.metadata().await?.len();
        let start = offset.min(size);
        f.seek(std::io::SeekFrom::Start(start)).await?;
        let want = (size - start).min(FILE_CHUNK_BYTES as u64) as usize;
        let mut buf = vec![0u8; want];
        f.read_exact(&mut buf).await?;
        Ok::<_, std::io::Error>((start, size, buf))
    }
    .await;
    match read {
        Ok((offset, size, buf)) => reply(
            id,
            Reply::FilePart {
                file: file.to_owned(),
                name: path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                offset,
                size,
                data: BASE64.encode(buf),
            },
        ),
        Err(e) => {
            tracing::warn!(%e, "couldn't read a file for file.get");
            error(id, ErrorCode::NotFound, "There's no such file.")
        }
    }
}

/// The person a connection acts for. Every way in resolves here, so other sign-ins (pairing
/// codes, passwords, a company's accounts) can be added in one place: today a client on this
/// computer showing the access token is the owner, and a phone is whoever it was paired to (the
/// owner for phones paired before people existed). `None` when that person doesn't exist.
async fn who(shared: &Shared, peer: &Peer) -> Option<thrall_store::Person> {
    let state = shared.hub.state();
    // The engine's own user is its owner.
    let user = match peer {
        Peer::Local => None,
        Peer::Phone { id } => shared.remote.person_of(id),
    }
    .unwrap_or_else(|| state.user_id.clone());
    match state.store.get_person(&user).await {
        // Someone the owner removed can't come back in.
        Ok(person) => person.filter(|p| p.removed_at.is_none()),
        Err(e) => {
            tracing::warn!(%e, "couldn't look up who is connecting");
            None
        }
    }
}

/// Wait for `hello`, check it, and answer `welcome`. `None` closes the connection. A phone has
/// already proved itself with its key, so only local clients need the access token.
async fn greet<I, O>(
    sink: &mut O,
    stream: &mut I,
    shared: &Shared,
    peer: &Peer,
) -> Option<(ClientKind, thrall_store::Person, Option<String>)>
where
    I: futures::Stream<Item = Incoming> + Unpin,
    O: futures::Sink<String> + Unpin,
{
    let deadline = std::time::Duration::from_secs(HELLO_TIMEOUT_SECS);
    let text = match tokio::time::timeout(deadline, stream.next()).await {
        Ok(Some(Incoming::Text(text))) => text,
        // Closed straight away: normal for a phone's pairing connection.
        Ok(None) => {
            tracing::debug!(?peer, "a connection closed before saying hello");
            return None;
        }
        _ => {
            tracing::info!(?peer, "a client connected but never said hello");
            return None;
        }
    };
    let frame: RequestFrame = match serde_json::from_str(&text) {
        Ok(f) => f,
        Err(e) => {
            tracing::warn!(?peer, %e, "turned a client away: its first message wasn't a valid hello");
            let msg = error("", ErrorCode::Invalid, "The first message must be hello.");
            let _ = send_all(sink, vec![msg]).await;
            return None;
        }
    };
    let Request::Hello {
        protocols,
        client,
        token: given,
    } = frame.request
    else {
        let msg = error(
            &frame.id,
            ErrorCode::Invalid,
            "The first message must be hello.",
        );
        let _ = send_all(sink, vec![msg]).await;
        return None;
    };
    // A local client is whoever its token says: the access token is the owner, a device token
    // (from a sign-in link) its person.
    let mut signed_in = None;
    if *peer == Peer::Local {
        match crate::signin::local_person(shared, &given).await {
            Ok(found) => signed_in = found,
            Err(e) => tracing::warn!(%e, "couldn't check a sign-in"),
        }
    }
    if *peer == Peer::Local && signed_in.is_none() {
        tracing::warn!(
            client = client.name,
            "turned a client away: wrong access token"
        );
        let msg = error(
            &frame.id,
            ErrorCode::Unauthorized,
            "Thrall didn't accept this connection's access token.",
        );
        let _ = send_all(sink, vec![msg]).await;
        return None;
    }
    let Some(protocol) = protocols
        .iter()
        .copied()
        .filter(|v| thrall_protocol::supports(*v))
        .max()
    else {
        let msg = error(
            &frame.id,
            ErrorCode::Unsupported,
            format!(
                "This Thrall engine speaks protocol {PROTOCOL_VERSION}. Update {} to connect.",
                client.name
            ),
        );
        let _ = send_all(sink, vec![msg]).await;
        return None;
    };
    let state = shared.hub.state();
    let found = match signed_in {
        Some(found) => Some(found),
        None => who(shared, peer).await.map(|p| (p, None)),
    };
    let Some((person, device)) = found else {
        tracing::warn!(?peer, "turned a client away: its person doesn't exist");
        let msg = error(
            &frame.id,
            ErrorCode::Unauthorized,
            "This device isn't linked to anyone on this Thrall.",
        );
        let _ = send_all(sink, vec![msg]).await;
        return None;
    };
    let welcome = reply(
        &frame.id,
        Reply::Welcome {
            protocol,
            engine: EngineInfo {
                version: shared.version.clone(),
                model: state.model(),
                edition: thrall_core::constants::EDITION.to_owned(),
            },
            user: UserInfo {
                id: person.id.clone(),
                name: person.name.clone(),
                role: person.role.clone(),
            },
            capabilities: CAPABILITIES.iter().map(|c| (*c).to_owned()).collect(),
            limits: Limits {
                frame_bytes: MAX_FRAME_BYTES as u64,
            },
        },
    );
    send_all(sink, vec![welcome]).await.ok()?;
    Some((client.kind, person, device))
}

struct Connection {
    shared: Arc<Shared>,
    kind: ClientKind,
    /// Who it is (a local client or a paired phone).
    peer: Peer,
    /// Open sessions and the last stored `seq` sent for each.
    attached: HashMap<String, u64>,
    /// Also send events from every other session (`session.watch_all`).
    watch_all: bool,
    /// Uploads this connection started with `file.put` (only these take more pieces).
    uploads: HashSet<String>,
    /// The signed-in browser it came from (`None`: the access token or a phone).
    device: Option<String>,
    /// The person it acts for.
    person: thrall_store::Person,
    /// Sessions seen on the bus, and whether they're this person's (only theirs are sent).
    whose: HashMap<String, bool>,
}

impl Connection {
    /// The engine acting for this connection's person.
    fn me(&self) -> thrall_agent::AppState {
        self.shared.hub.state().for_user(&self.person.id)
    }

    /// Whether a signed-in browser hasn't been signed out (true for other connections).
    async fn still_signed_in(&self) -> bool {
        let Some(device) = &self.device else {
            return true;
        };
        let store = &self.shared.hub.state().store;
        store.device_signed_in(device).await.unwrap_or(true)
    }

    /// Whose devices a request may see or sign out: your own, or anyone's for the owner.
    fn whose_devices(&self, person: Option<String>) -> Option<String> {
        match person {
            None => Some(self.person.id.clone()),
            Some(p) if p == self.person.id => Some(p),
            Some(p) if self.person.role == thrall_core::constants::person_role::OWNER => Some(p),
            Some(_) => None,
        }
    }

    /// `device.listed` for `person`.
    async fn devices(&self, person: &str) -> Result<Reply, thrall_agent::AgentError> {
        let found = self.shared.hub.state().store.list_devices(person).await?;
        let items = found
            .into_iter()
            .map(|d| thrall_protocol::DeviceInfo {
                current: self.device.as_deref() == Some(d.id.as_str()),
                device: d.id,
                name: d.name,
                added: d.created_at,
                last_seen: d.last_seen_at,
            })
            .collect();
        Ok(Reply::DeviceListed {
            person: person.to_owned(),
            items,
        })
    }

    /// `person` is someone who uses this Thrall (not removed).
    async fn is_person(&self, person: &str) -> Result<bool, thrall_agent::AgentError> {
        let found = self.shared.hub.state().store.get_person(person).await?;
        Ok(found.is_some_and(|p| p.removed_at.is_none()))
    }

    async fn on_text(&mut self, text: &str) -> Vec<ServerFrame> {
        if text.len() > MAX_FRAME_BYTES {
            return vec![error(
                "",
                ErrorCode::TooLarge,
                "That message is too big to send in one piece.",
            )];
        }
        let frame: RequestFrame = match serde_json::from_str(text) {
            Ok(f) => f,
            Err(e) => {
                // Find the id if there is one, so the client can match the error.
                let id = serde_json::from_str::<serde_json::Value>(text)
                    .ok()
                    .and_then(|v| v.get("id").and_then(|i| i.as_str()).map(str::to_owned))
                    .unwrap_or_default();
                let unknown = e.to_string().contains("unknown variant");
                return vec![if unknown {
                    error(
                        &id,
                        ErrorCode::Unsupported,
                        "This engine doesn't know that request.",
                    )
                } else {
                    error(
                        &id,
                        ErrorCode::Invalid,
                        format!("That request isn't valid: {e}"),
                    )
                }];
            }
        };
        self.handle(&frame.id, frame.request).await
    }

    async fn handle(&mut self, id: &str, request: Request) -> Vec<ServerFrame> {
        let hub = self.shared.hub.clone();
        let by = self.kind.as_str();
        let result: Result<Vec<ServerFrame>, thrall_agent::AgentError> = async {
            Ok(match request {
                Request::Hello { .. } => {
                    vec![error(id, ErrorCode::Invalid, "Already connected.")]
                }
                Request::Ping {} => vec![reply(id, Reply::Pong {})],
                _ if owner_only(&request)
                    && self.person.role != thrall_core::constants::person_role::OWNER =>
                {
                    vec![error(
                        id,
                        ErrorCode::Unauthorized,
                        "Only the person who runs this Thrall can do that.",
                    )]
                }
                Request::SessionCreate {
                    workspace,
                    title,
                    mode,
                } => {
                    let mode = match mode.as_deref().map(Mode::parse) {
                        Some(None) => {
                            return Ok(vec![error(id, ErrorCode::Invalid, "Unknown mode.")]);
                        }
                        Some(m) => m,
                        None => None,
                    };
                    if let Some(Err(why)) = workspace.as_deref().map(crate::folders::usable) {
                        return Ok(vec![error(id, ErrorCode::Invalid, why)]);
                    }
                    let s = hub.create(&self.person.id, workspace, title, mode).await?;
                    self.attached.insert(s.id.clone(), s.last_seq);
                    vec![reply(id, Reply::SessionOpened(hub.info(&s).await))]
                }
                Request::SessionOpen { session, after_seq } => {
                    let s = session::get(&self.me(), &session).await?;
                    let mut out = vec![reply(id, Reply::SessionOpened(hub.info(&s).await))];
                    let after = after_seq.unwrap_or(0);
                    self.attached.insert(session.clone(), after);
                    out.extend(self.replay(&session).await?);
                    out
                }
                Request::SessionDetach { session } => {
                    self.attached.remove(&session);
                    vec![reply(id, Reply::Ack {})]
                }
                Request::SessionList { limit, archived } => {
                    let state = self.me();
                    let limit = limit.unwrap_or(DEFAULT_SESSION_LIST);
                    let sessions = if archived {
                        state
                            .store
                            .list_archived_sessions(&state.user_id, limit)
                            .await?
                    } else {
                        state.store.list_sessions(&state.user_id, limit).await?
                    };
                    let mut items = Vec::with_capacity(sessions.len());
                    for s in &sessions {
                        items.push(hub.info(s).await);
                    }
                    vec![reply(id, Reply::SessionListed { items })]
                }
                Request::SessionRename { session, title } => {
                    let title = title.trim();
                    if title.is_empty() {
                        return Ok(vec![error(
                            id,
                            ErrorCode::Invalid,
                            "A title can't be empty.",
                        )]);
                    }
                    hub.rename(&self.person.id, &session, title).await?;
                    vec![reply(id, Reply::Ack {})]
                }
                Request::SessionSetMode {
                    session,
                    mode,
                    until,
                    minutes,
                } => match Mode::parse(&mode) {
                    Some(mode) => match thrall_agent::hub::mode_limit_from(until.as_deref(), minutes)
                    {
                        Some(limit) => {
                            hub.set_mode(&self.person.id, &session, mode, Some(limit))
                                .await?;
                            vec![reply(id, Reply::Ack {})]
                        }
                        None => vec![error(
                            id,
                            ErrorCode::Invalid,
                            "Full auto lasts until the task ends, for 1 to 1440 minutes, or until \
                             you switch it off.",
                        )],
                    },
                    None => vec![error(
                        id,
                        ErrorCode::Invalid,
                        "Unknown mode: use ask, auto_edits, plan or full_auto.",
                    )],
                },
                Request::SessionArchive { session, archived } => {
                    hub.archive(&self.person.id, &session, archived).await?;
                    vec![reply(id, Reply::Ack {})]
                }
                Request::SessionPin { session, pinned } => {
                    hub.pin(&self.person.id, &session, pinned).await?;
                    vec![reply(id, Reply::Ack {})]
                }
                Request::SessionMove { session, group } => {
                    hub.move_to_group(&self.person.id, &session, group.as_deref())
                        .await?;
                    vec![reply(id, Reply::Ack {})]
                }
                Request::SessionSetFolder { session, folder } => {
                    match hub.set_folder(&self.person.id, &session, &folder).await? {
                        Ok(_) => vec![reply(id, Reply::Ack {})],
                        Err(why) => vec![error(id, ErrorCode::Invalid, &why)],
                    }
                }
                Request::GroupList {} => vec![reply(
                    id,
                    Reply::GroupListed {
                        items: hub.groups(&self.person.id).await?,
                    },
                )],
                Request::GroupCreate { name } => {
                    let name = name.trim();
                    if name.is_empty() {
                        return Ok(vec![error(id, ErrorCode::Invalid, "A group needs a name.")]);
                    }
                    hub.create_group(&self.person.id, name).await?;
                    vec![reply(
                        id,
                        Reply::GroupListed {
                            items: hub.groups(&self.person.id).await?,
                        },
                    )]
                }
                Request::GroupRename { group, name } => {
                    let name = name.trim();
                    if name.is_empty() {
                        return Ok(vec![error(id, ErrorCode::Invalid, "A group needs a name.")]);
                    }
                    hub.rename_group(&self.person.id, &group, name).await?;
                    vec![reply(
                        id,
                        Reply::GroupListed {
                            items: hub.groups(&self.person.id).await?,
                        },
                    )]
                }
                Request::GroupDelete { group } => {
                    hub.delete_group(&self.person.id, &group).await?;
                    vec![reply(
                        id,
                        Reply::GroupListed {
                            items: hub.groups(&self.person.id).await?,
                        },
                    )]
                }
                Request::GroupReorder { groups } => {
                    hub.reorder_groups(&self.person.id, &groups).await?;
                    vec![reply(
                        id,
                        Reply::GroupListed {
                            items: hub.groups(&self.person.id).await?,
                        },
                    )]
                }
                Request::SessionDelete { session } => {
                    hub.delete(&self.person.id, &session).await?;
                    self.attached.remove(&session);
                    vec![reply(id, Reply::Ack {})]
                }
                Request::HeartbeatGet {} | Request::HeartbeatSet { .. } => {
                    if let Request::HeartbeatSet { checklist } = &request {
                        hub.set_checklist(checklist)?;
                    }
                    let (checklist, next_run) = hub.heartbeat();
                    vec![reply(
                        id,
                        Reply::HeartbeatShown {
                            checklist,
                            next_run,
                        },
                    )]
                }
                Request::HeartbeatRun {} => {
                    hub.check_in_now()?;
                    vec![reply(id, Reply::Ack {})]
                }
                Request::StartupGet {} | Request::StartupSet { .. } => {
                    vec![startup(id, &request)]
                }
                Request::UpdateInstall {} => {
                    if self.shared.desktops.load(Ordering::SeqCst) == 0 {
                        vec![error(
                            id,
                            ErrorCode::Invalid,
                            "Updates install through Thrall's desktop app, and it isn't running.",
                        )]
                    } else {
                        self.shared.update_asked.send_modify(|n| *n += 1);
                        vec![reply(id, Reply::Ack {})]
                    }
                }
                Request::FoldersGet {} => vec![reply(id, crate::folders::shown(&hub.state()))],
                Request::FolderOpen { folder } => {
                    match crate::folders::open(&hub.state(), folder) {
                        Ok(()) => vec![reply(id, Reply::Ack {})],
                        Err(why) => vec![error(id, ErrorCode::Invalid, why)],
                    }
                }
                Request::HomeGet {} | Request::HomeClear {} => {
                    if matches!(request, Request::HomeClear {}) {
                        hub.clear_home(&self.person.id).await?;
                    }
                    let (page, earlier) = hub.home(&self.person.id).await?;
                    let page = hub.info(&page).await;
                    let mut items = Vec::with_capacity(earlier.len());
                    for s in &earlier {
                        items.push(hub.info(s).await);
                    }
                    vec![reply(
                        id,
                        Reply::HomeShown {
                            page,
                            earlier: items,
                        },
                    )]
                }
                Request::TurnSend {
                    session,
                    text,
                    files,
                } => {
                    if text.trim().is_empty() {
                        return Ok(vec![error(id, ErrorCode::Invalid, "The message is empty.")]);
                    }
                    hub.send(&self.person.id, &session, &text, &files).await?;
                    vec![reply(id, Reply::Ack {})]
                }
                Request::CheckpointPreview { session, turn } => {
                    let (text, files) =
                        hub.rewind_preview(&self.person.id, &session, &turn).await?;
                    vec![reply(id, Reply::CheckpointShown { turn, text, files })]
                }
                Request::TurnRewind {
                    session,
                    turn,
                    files,
                    conversation,
                    keep,
                } => {
                    if !files && !conversation {
                        return Ok(vec![error(
                            id,
                            ErrorCode::Invalid,
                            "Choose files, the conversation, or both to rewind.",
                        )]);
                    }
                    let (text, restored, failed) = hub
                        .rewind(&self.person.id, &session, &turn, files, conversation, &keep)
                        .await?;
                    vec![reply(
                        id,
                        Reply::TurnRewound {
                            text,
                            restored,
                            failed,
                        },
                    )]
                }
                Request::TurnRetry { session } => {
                    hub.retry(&self.person.id, &session, None).await?;
                    vec![reply(id, Reply::Ack {})]
                }
                Request::TurnEdit { session, text } => {
                    if text.trim().is_empty() {
                        return Ok(vec![error(id, ErrorCode::Invalid, "The message is empty.")]);
                    }
                    hub.retry(&self.person.id, &session, Some(text)).await?;
                    vec![reply(id, Reply::Ack {})]
                }
                Request::TurnInterrupt { session } => {
                    hub.interrupt(&self.person.id, &session).await;
                    vec![reply(id, Reply::Ack {})]
                }
                Request::PermissionAnswer { request, choice } => {
                    if hub.answer(&self.person.id, &request, Answer::Permission(choice), by) {
                        vec![reply(id, Reply::Ack {})]
                    } else {
                        vec![already_answered(id)]
                    }
                }
                Request::PlanDecide {
                    plan,
                    approve,
                    auto_edits,
                } => {
                    if hub.answer(
                        &self.person.id,
                        &plan,
                        Answer::Plan {
                            approve,
                            auto_edits,
                        },
                        by,
                    ) {
                        vec![reply(id, Reply::Ack {})]
                    } else {
                        vec![already_answered(id)]
                    }
                }
                Request::QuestionAnswer { question, answers } => {
                    if hub.answer(&self.person.id, &question, Answer::Question(answers), by) {
                        vec![reply(id, Reply::Ack {})]
                    } else {
                        vec![already_answered(id)]
                    }
                }
                Request::PersonalityGet {} => vec![reply(id, personality_shown(&self.me()))],
                Request::PersonalitySet { preset, custom } => {
                    let state = self.me();
                    let chosen = thrall_agent::personality::tidy(&preset, &custom);
                    match thrall_agent::personality::save(&state, &chosen) {
                        Ok(()) => vec![reply(id, personality_shown(&state))],
                        Err(why) => vec![error(id, ErrorCode::Internal, why)],
                    }
                }
                Request::ExperienceList {} => vec![reply(
                    id,
                    Reply::ExperienceListed {
                        items: thrall_agent::experience::listed(&self.me()),
                    },
                )],
                Request::ExperienceForget { lesson } => {
                    let state = self.me();
                    match thrall_agent::experience::forget(&state, &lesson) {
                        Ok(true) => vec![reply(
                            id,
                            Reply::ExperienceListed {
                                items: thrall_agent::experience::listed(&state),
                            },
                        )],
                        Ok(false) => vec![error(id, ErrorCode::NotFound, "There's no such lesson.")],
                        Err(why) => vec![error(id, ErrorCode::Internal, why)],
                    }
                }
                Request::SessionWatchAll {} => {
                    self.watch_all = true;
                    vec![reply(id, Reply::Ack {})]
                }
                Request::EngineStop {} => {
                    tracing::info!(client = by, "asked to stop the engine");
                    self.shared.stop.notify_one();
                    vec![reply(id, Reply::Ack {})]
                }
                Request::AudioTranscribe { file, session } => {
                    let text = hub
                        .transcribe_upload(&self.person.id, &file, session.as_deref())
                        .await?;
                    vec![reply(id, Reply::AudioTranscribed { text })]
                }
                Request::CommandList {} => {
                    let mut items: Vec<CommandItem> = commands::COMMANDS
                        .iter()
                        .map(|c| CommandItem {
                            name: c.name.to_owned(),
                            summary: c.summary.to_owned(),
                            args_hint: c.args_hint.to_owned(),
                            skill: false,
                        })
                        .collect();
                    // Built-ins Thrall carries out itself go as messages, like skills.
                    items.extend(commands::MESSAGE_COMMANDS.iter().map(|c| CommandItem {
                        name: c.name.to_owned(),
                        summary: c.summary.to_owned(),
                        args_hint: c.args_hint.to_owned(),
                        skill: true,
                    }));
                    // The person's own skills run as `/name …` (a project's work too, typed).
                    items.extend(hub.skill_commands(&self.person.id).into_iter().map(
                        |(name, summary)| CommandItem {
                            name,
                            summary,
                            args_hint: SKILL_ARGS_HINT.to_owned(),
                            skill: true,
                        },
                    ));
                    vec![reply(id, Reply::CommandListed { items })]
                }
                Request::MemoryList {} => {
                    let facts = hub.facts(&self.person.id).await?;
                    vec![reply(id, Reply::MemoryListed { facts })]
                }
                Request::MemoryForget { fact } => {
                    if hub.forget(&self.person.id, &fact).await? {
                        vec![reply(id, Reply::Ack {})]
                    } else {
                        vec![error(
                            id,
                            ErrorCode::NotFound,
                            "Thrall doesn't remember that any more.",
                        )]
                    }
                }
                Request::SkillRemove { name } => {
                    if let Err(why) = thrall_agent::settings::remove_skill(&self.me(), &name) {
                        return Ok(vec![error(id, ErrorCode::Invalid, why)]);
                    }
                    // A removed skill needn't stay listed as turned off.
                    let listed = hub
                        .state()
                        .config
                        .skills
                        .disabled
                        .iter()
                        .any(|d| d.eq_ignore_ascii_case(&name));
                    if listed {
                        hub.save_setting(
                            &format!("skills.{name}.enabled"),
                            &serde_json::Value::Bool(true),
                        )
                        .await?;
                    }
                    vec![reply(
                        id,
                        thrall_agent::settings::shown(&self.me(), hub.settings_pending()).await,
                    )]
                }
                Request::SkillGet { name } => {
                    match thrall_agent::settings::skill_detail(&self.me(), &name) {
                        Ok(skill) => vec![reply(id, Reply::SkillShown { skill })],
                        Err(why) => vec![error(id, ErrorCode::Invalid, why)],
                    }
                }
                Request::SkillEdit {
                    name,
                    description,
                    instructions,
                } => match thrall_agent::settings::edit_skill(
                    &self.me(),
                    &name,
                    &description,
                    &instructions,
                ) {
                    Ok(skill) => vec![reply(id, Reply::SkillShown { skill })],
                    Err(why) => vec![error(id, ErrorCode::Invalid, why)],
                },
                Request::SkillMakeSafe { name } => {
                    match thrall_agent::settings::make_skill_safe(&self.me(), &name).await {
                        Ok(answer) => vec![reply(id, answer)],
                        Err((code, why)) => vec![error(id, code, why)],
                    }
                }
                Request::CommandRun {
                    session,
                    name,
                    args,
                } => {
                    let markdown = hub
                        .command(&self.person.id, session.as_deref(), &name, &args)
                        .await?;
                    vec![reply(id, Reply::CommandResult { markdown })]
                }
                Request::PhoneList {}
                | Request::PhonePair { .. }
                | Request::PhoneAssign { .. }
                | Request::PhoneRemove { .. }
                    if self.peer != Peer::Local =>
                {
                    vec![error(
                        id,
                        ErrorCode::Unauthorized,
                        "Phones are added and removed on the computer Thrall runs on.",
                    )]
                }
                Request::PhoneList {} => vec![reply(id, self.shared.remote.listed(&self.me()))],
                Request::PhonePair { person } => {
                    if let Some(p) = &person
                        && !self.is_person(p).await?
                    {
                        vec![error(id, ErrorCode::NotFound, "There's no such person.")]
                    } else {
                        match self.shared.remote.pair(&self.me(), person) {
                            Ok(pairing) => vec![reply(id, pairing)],
                            Err(why) => vec![error(id, ErrorCode::Invalid, why)],
                        }
                    }
                }
                Request::PhoneAssign { phone, person } => {
                    let owner = self.shared.hub.state().owner_id.clone();
                    if !self.is_person(&person).await? {
                        vec![error(id, ErrorCode::NotFound, "There's no such person.")]
                    } else if self.shared.remote.assign(&phone, &person, &owner) {
                        vec![reply(id, self.shared.remote.listed(&self.me()))]
                    } else {
                        vec![error(id, ErrorCode::NotFound, "There's no such phone.")]
                    }
                }
                Request::PhoneRemove { phone } => {
                    if self.shared.remote.remove(&phone) {
                        vec![reply(id, self.shared.remote.listed(&self.me()))]
                    } else {
                        vec![error(id, ErrorCode::NotFound, "There's no such phone.")]
                    }
                }
                Request::PeopleList { since } => {
                    let mut listed = crate::people::listed(&self.shared, since).await?;
                    // Anyone else sees only themselves: their key, model, limit and spending.
                    if self.person.role != thrall_core::constants::person_role::OWNER
                        && let Reply::PeopleListed { items } = &mut listed
                    {
                        items.retain(|p| p.person == self.person.id);
                    }
                    vec![reply(id, listed)]
                }
                Request::PeopleAdd { name } => {
                    refused_or(id, crate::people::add(&self.shared, &name).await?)
                }
                Request::PeopleRename { person, name } => refused_or(
                    id,
                    crate::people::rename(&self.shared, &person, &name).await?,
                ),
                Request::PeopleRemove { person } => {
                    refused_or(id, crate::people::remove(&self.shared, &person).await?)
                }
                Request::PeopleTerms {
                    person,
                    own_key,
                    model,
                    monthly_limit,
                } => refused_or(
                    id,
                    crate::people::set_terms(
                        &self.shared,
                        &person,
                        own_key,
                        model.as_deref(),
                        monthly_limit,
                    )
                    .await?,
                ),
                Request::PeopleKey { person, key } => match self.whose_devices(person) {
                    Some(person) => match crate::people::set_key(
                        &self.shared,
                        &person,
                        key.as_deref().unwrap_or_default(),
                    )
                    .await?
                    {
                        Ok(()) => vec![reply(id, Reply::Ack {})],
                        Err(why) => vec![error(id, ErrorCode::Invalid, &why)],
                    },
                    None => vec![error(
                        id,
                        ErrorCode::Unauthorized,
                        "Only the person who runs this Thrall can set someone else's key.",
                    )],
                },
                Request::SigninCreate { person } => {
                    match self.shared.hub.state().store.get_person(&person).await? {
                        Some(p) if p.removed_at.is_none() => {
                            match self.shared.signins.create(&p.id) {
                                Some((code, expires)) => vec![reply(
                                    id,
                                    Reply::SigninCode {
                                        code,
                                        name: p.name,
                                        expires,
                                    },
                                )],
                                None => vec![error(
                                    id,
                                    ErrorCode::Internal,
                                    "Couldn't make a sign-in link: no random numbers.",
                                )],
                            }
                        }
                        _ => vec![error(id, ErrorCode::NotFound, "There's no such person.")],
                    }
                }
                Request::DeviceList { person } => match self.whose_devices(person) {
                    Some(person) => vec![reply(id, self.devices(&person).await?)],
                    None => vec![error(
                        id,
                        ErrorCode::Unauthorized,
                        "Only the person who runs this Thrall can see someone else's devices.",
                    )],
                },
                Request::DeviceRemove { device, person } => match self.whose_devices(person) {
                    Some(person) => {
                        let store = &self.shared.hub.state().store;
                        if store.remove_device(&person, &device, now_ms()).await? {
                            self.shared.signins.bump();
                            vec![reply(id, self.devices(&person).await?)]
                        } else {
                            vec![error(id, ErrorCode::NotFound, "There's no such device.")]
                        }
                    }
                    None => vec![error(
                        id,
                        ErrorCode::Unauthorized,
                        "Only the person who runs this Thrall can sign someone else out.",
                    )],
                },
                Request::FilePut { upload, name, data } => {
                    vec![self.file_put(id, upload, &name, &data).await]
                }
                Request::FolderPlaces {} => {
                    let p = crate::folders::places(&self.me()).await?;
                    vec![reply(
                        id,
                        Reply::FolderPlaces {
                            recent: p.recent,
                            default: p.default,
                            new_in: p.new_in,
                        },
                    )]
                }
                Request::FolderCreate { parent, name } => {
                    match crate::folders::create(&self.me(), &parent, &name).await {
                        Ok(path) => vec![reply(id, Reply::FolderCreated { path })],
                        Err(why) => vec![error(id, ErrorCode::Invalid, why)],
                    }
                }
                Request::AppLatest {} => {
                    let state = self.me();
                    let release =
                        crate::app_release::latest(state.data_dir.as_deref()).map(|(r, _)| r);
                    vec![reply(id, Reply::AppLatest { release })]
                }
                Request::AppGet { offset } => {
                    let state = self.me();
                    match crate::app_release::read(state.data_dir.as_deref(), offset).await {
                        Ok((name, offset, size, bytes)) => vec![reply(
                            id,
                            Reply::FilePart {
                                file: PHONE_APP_FILE_ID.to_owned(),
                                name,
                                offset,
                                size,
                                data: BASE64.encode(bytes),
                            },
                        )],
                        Err(why) => vec![error(id, ErrorCode::NotFound, why)],
                    }
                }
                Request::FolderList { session, path } => {
                    let s = session::get(&self.me(), &session).await?;
                    let ws = session::folder_of(&s);
                    match crate::folders::list(&ws, &path) {
                        Ok((path, items, more)) => vec![reply(
                            id,
                            Reply::FolderListed {
                                session,
                                path,
                                items,
                                more,
                            },
                        )],
                        Err(why) => vec![error(id, ErrorCode::NotFound, why)],
                    }
                }
                Request::FolderRead {
                    session,
                    path,
                    offset,
                } => {
                    let s = session::get(&self.me(), &session).await?;
                    let ws = session::folder_of(&s);
                    match crate::folders::read(&ws, &path, offset).await {
                        Ok((path, offset, size, bytes)) => vec![reply(
                            id,
                            Reply::FolderPart {
                                session,
                                path,
                                offset,
                                size,
                                data: BASE64.encode(bytes),
                            },
                        )],
                        Err(why) => vec![error(id, ErrorCode::NotFound, why)],
                    }
                }
                Request::FileGet { file, offset } => {
                    vec![file_get(id, &self.me(), &file, offset).await]
                }
                Request::SettingsGet {} => vec![reply(
                    id,
                    thrall_agent::settings::shown(&self.me(), hub.settings_pending()).await,
                )],
                Request::LocalFind {} => vec![reply(
                    id,
                    thrall_agent::settings::local_found(&hub.state()).await,
                )],
                Request::ProviderTest { supplier } => vec![reply(
                    id,
                    thrall_agent::settings::test_supplier(&hub.state(), supplier.trim()).await,
                )],
                Request::SettingsSet { key, value } => {
                    hub.save_setting(&key, &value.to_json()).await?;
                    vec![reply(
                        id,
                        thrall_agent::settings::shown(&self.me(), hub.settings_pending()).await,
                    )]
                }
            })
        }
        .await;
        result.unwrap_or_else(|e| vec![error(id, e.code(), e.to_string())])
    }

    /// `file.put`: start an upload, or add a piece to one this connection started.
    async fn file_put(
        &mut self,
        id: &str,
        upload: Option<String>,
        name: &str,
        data: &str,
    ) -> ServerFrame {
        let Ok(bytes) = BASE64.decode(data.trim()) else {
            return error(id, ErrorCode::Invalid, "The file piece isn't valid base64.");
        };
        if bytes.len() > FILE_CHUNK_BYTES {
            return error(
                id,
                ErrorCode::TooLarge,
                format!("Send files in pieces of at most {FILE_CHUNK_BYTES} bytes."),
            );
        }
        let state = self.me();
        let path = match upload {
            Some(upload) => {
                let path = self
                    .uploads
                    .contains(&upload)
                    .then(|| uploaded_file(&state, &upload))
                    .flatten();
                let Some(path) = path else {
                    return error(id, ErrorCode::NotFound, "There's no such upload to add to.");
                };
                path
            }
            None => {
                let Some(folder) = uploads::folder(state.my_dir().as_deref()) else {
                    return error(
                        id,
                        ErrorCode::Internal,
                        "This engine has no data folder to keep files in.",
                    );
                };
                let upload = new_id(id_prefix::FILE);
                let folder = folder.join(&upload);
                if let Err(e) = tokio::fs::create_dir_all(&folder).await {
                    tracing::warn!(%e, "couldn't make an upload folder");
                    return error(id, ErrorCode::Internal, "Couldn't save the file.");
                }
                self.uploads.insert(upload);
                folder.join(uploads::clean_name(name))
            }
        };
        let had = tokio::fs::metadata(&path)
            .await
            .map(|m| m.len())
            .unwrap_or(0);
        let size = had + bytes.len() as u64;
        if size > MAX_UPLOAD_BYTES as u64 {
            return error(
                id,
                ErrorCode::TooLarge,
                format!(
                    "Files can be at most {} MB.",
                    MAX_UPLOAD_BYTES / (1024 * 1024)
                ),
            );
        }
        let written = async {
            let mut file = tokio::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .await?;
            tokio::io::AsyncWriteExt::write_all(&mut file, &bytes).await?;
            tokio::io::AsyncWriteExt::flush(&mut file).await
        }
        .await;
        if let Err(e) = written {
            tracing::warn!(%e, "couldn't save a file piece");
            return error(id, ErrorCode::Internal, "Couldn't save the file.");
        }
        let file = path
            .parent()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        reply(id, Reply::FileSaved { file, name, size })
    }

    /// Stored events for `session` after the last one sent, oldest first.
    async fn replay(
        &mut self,
        session: &str,
    ) -> Result<Vec<ServerFrame>, thrall_agent::AgentError> {
        let state = self.me();
        let mut out = Vec::new();
        let Some(mut after) = self.attached.get(session).copied() else {
            return Ok(out);
        };
        loop {
            let page = state
                .store
                .events_after(&state.user_id, session, after, REPLAY_PAGE)
                .await?;
            let count = page.len();
            for stored in page {
                after = stored.seq;
                match serde_json::from_str::<Event>(&stored.payload) {
                    Ok(event) => out.push(ServerFrame::Event(EventFrame::new(
                        session,
                        Some(stored.seq),
                        stored.at,
                        event,
                    ))),
                    Err(e) => tracing::warn!(%e, seq = stored.seq, "skipping unreadable event"),
                }
            }
            if count < REPLAY_PAGE as usize {
                break;
            }
        }
        self.attached.insert(session.to_owned(), after);
        Ok(out)
    }

    /// The bus dropped frames for this client: resend what's stored for each open session.
    async fn catch_up(&mut self) -> Vec<ServerFrame> {
        let sessions: Vec<String> = self.attached.keys().cloned().collect();
        let mut out = Vec::new();
        for s in sessions {
            match self.replay(&s).await {
                Ok(frames) => out.extend(frames),
                Err(e) => tracing::warn!(%e, session = s, "couldn't catch a client up"),
            }
        }
        out
    }

    /// Whether a session is this connection's person's (looked up once, then remembered).
    async fn is_mine(&mut self, session: &str) -> bool {
        if let Some(mine) = self.whose.get(session) {
            return *mine;
        }
        let state = self.me();
        let mine = matches!(
            state.store.get_session(&state.user_id, session).await,
            Ok(Some(_))
        );
        self.whose.insert(session.to_owned(), mine);
        mine
    }

    /// Whether this client gets a live event, and the frame to send. Only events from its own
    /// person's sessions ever go out.
    async fn on_event(&mut self, frame: EventFrame) -> Option<ServerFrame> {
        if !self.attached.contains_key(&frame.session) && !self.is_mine(&frame.session).await {
            return None;
        }
        match self.attached.get_mut(&frame.session) {
            Some(last) => match frame.seq {
                // Already sent while replaying.
                Some(seq) if seq <= *last => None,
                Some(seq) => {
                    *last = seq;
                    Some(ServerFrame::Event(frame))
                }
                None => Some(ServerFrame::Event(frame)),
            },
            // Other sessions: everything but live text for a client watching them all, otherwise
            // only word that background work finished.
            None => {
                let wanted = if self.watch_all {
                    !frame.event.is_live_only() || matches!(frame.event, Event::Status { .. })
                } else {
                    matches!(frame.event, Event::AgentCompleted { .. })
                };
                wanted.then_some(ServerFrame::Event(frame))
            }
        }
    }
}

/// Answer `startup.get` / `startup.set`: whether Thrall starts at sign-in, through the desktop app
/// beside this engine.
fn startup(id: &str, request: &Request) -> ServerFrame {
    use thrall_core::constants::STARTUP_LABEL;
    use thrall_core::startup;
    let line = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().and_then(startup::command_line));
    let changed = match (request, &line) {
        (Request::StartupSet { .. }, None) => {
            return error(
                id,
                ErrorCode::Invalid,
                format!(
                    "\"{STARTUP_LABEL}\" needs Thrall's desktop app, installed beside the engine."
                ),
            );
        }
        (Request::StartupSet { on, .. }, Some(line)) => startup::set(on.then_some(line.as_str())),
        _ => Ok(()),
    };
    match changed.and_then(|()| startup::is_on()) {
        Ok(on) => reply(
            id,
            Reply::StartupShown {
                available: line.is_some(),
                on,
                label: STARTUP_LABEL.to_owned(),
            },
        ),
        Err(e) => error(id, ErrorCode::Internal, e.to_string()),
    }
}

/// personality.shown for whoever state acts for.
fn personality_shown(state: &thrall_agent::AppState) -> Reply {
    let chosen = thrall_agent::personality::load(state);
    Reply::PersonalityShown {
        preset: chosen.preset,
        custom: chosen.custom,
        presets: thrall_agent::personality::presets(),
    }
}

/// Requests only the engine's owner may make: they change Thrall for everyone (settings, Home
/// check-ins, starting with Windows, Thrall's folders, phones, people, stopping the engine, the
/// model in use, Thrall's sign-ins to services).
fn owner_only(request: &Request) -> bool {
    match request {
        Request::SettingsSet { .. }
        | Request::ProviderTest { .. }
        | Request::LocalFind {}
        | Request::HeartbeatGet {}
        | Request::HeartbeatSet { .. }
        | Request::HeartbeatRun {}
        | Request::StartupGet {}
        | Request::StartupSet { .. }
        | Request::FoldersGet {}
        | Request::FolderOpen { .. }
        | Request::UpdateInstall {}
        | Request::EngineStop {}
        | Request::PhoneList {}
        | Request::PhonePair { .. }
        | Request::PhoneAssign { .. }
        | Request::PhoneRemove { .. }
        | Request::PeopleAdd { .. }
        | Request::PeopleRename { .. }
        | Request::PeopleRemove { .. }
        | Request::PeopleTerms { .. }
        | Request::SigninCreate { .. } => true,
        Request::CommandRun { name, args, .. } => {
            let first = args.split_whitespace().next().unwrap_or_default();
            match name.as_str() {
                "login" | "logout" => true,
                "model" => !args.trim().is_empty(),
                "mcp" => {
                    first.eq_ignore_ascii_case("login") || first.eq_ignore_ascii_case("logout")
                }
                _ => false,
            }
        }
        _ => false,
    }
}

/// The answer, or the reason it was refused.
fn refused_or(id: &str, outcome: Result<Reply, crate::people::Refusal>) -> Vec<ServerFrame> {
    match outcome {
        Ok(answer) => vec![reply(id, answer)],
        Err(why) => vec![error(id, ErrorCode::Invalid, why)],
    }
}

fn already_answered(id: &str) -> ServerFrame {
    error(
        id,
        ErrorCode::AlreadyAnswered,
        "That was already answered (maybe on another device).",
    )
}
