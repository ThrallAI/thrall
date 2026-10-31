//! The phone link: how a paired phone reaches the engine from outside the computer.
//!
//! It has its own listener on 127.0.0.1 (a tunnel such as Tailscale Funnel forwards to it), only
//! while `[remote] enabled` is on. It serves one WebSocket path, and every message on it is
//! encrypted end to end with the Noise protocol, so the tunnel only ever sees ciphertext:
//!
//! - **Pairing** (`Noise_IKpsk1`): the QR gives the phone the engine's public key and a one-time
//!   secret. The phone's first message proves it holds the secret and carries its own public key,
//!   which the engine then remembers. A code works once, for a few minutes, and stops after a few
//!   wrong tries.
//! - **Connecting** (`Noise_IK`): a paired phone proves it holds its key; any other key is turned
//!   away. Removing a phone closes its connection and keeps it out.
//!
//! Inside the encrypted link it's the ordinary protocol (`hello`, requests, events), served by
//! the same code as local clients. Frames bigger than one Noise message go in pieces.
//!
//! The engine's key pair is in the data folder (next to the access token, which already gives
//! full control to anything that can read that folder).

use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Instant;

use axum::Router;
use axum::extract::ws::{Message, WebSocket};
use axum::extract::{State, WebSocketUpgrade};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64URL;
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use snow::{Builder, HandshakeState, TransportState};
use thrall_agent::AppState;
use thrall_core::constants::id_prefix;
use thrall_core::ids::{new_id, now_ms};
use thrall_protocol::constants::{HELLO_TIMEOUT_SECS, MAX_FRAME_BYTES, REMOTE_PATH};
use thrall_protocol::{PhoneInfo, RemoteInfo, Reply};
use tokio::sync::watch;

use crate::connection::{Incoming, Peer, Shared, serve_link};
use crate::constants::{
    LINK_CONNECT, LINK_FAIL_WINDOW, LINK_MAX_FAILS, LINK_PAIR, LINK_PIECE, LOCAL_HOST,
    NOISE_CONNECT, NOISE_MAX_MESSAGE, NOISE_PAIR, NOISE_PROLOGUE, PAIR_ID_BYTES, PAIR_LIFETIME,
    PAIR_LINK_PREFIX, PAIR_MAX_OPEN, PAIR_MAX_TRIES, PAIR_SECRET_BYTES, PHONE_FALLBACK_NAME,
    PHONE_NAME_MAX_CHARS, PHONES_FILE, PIECE_LAST, PIECE_MORE, REMOTE_CHECK, REMOTE_KEY_FILE,
};

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(text: &str) -> Option<Vec<u8>> {
    let text = text.trim();
    if !text.len().is_multiple_of(2) {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(text.get(i..i + 2)?, 16).ok())
        .collect()
}

fn random<const N: usize>() -> Option<[u8; N]> {
    let mut bytes = [0u8; N];
    getrandom::fill(&mut bytes).ok()?;
    Some(bytes)
}

/// A paired phone, as kept in `phones.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Phone {
    id: String,
    name: String,
    /// Its public key, hex.
    key: String,
    added: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_seen: Option<i64>,
    /// Whose phone it is (a user id). None: the owner's (phones paired before people existed).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    person: Option<String>,
}

/// A pairing code waiting to be scanned.
struct Code {
    id: [u8; PAIR_ID_BYTES],
    secret: [u8; PAIR_SECRET_BYTES],
    expires: Instant,
    tries: u32,
    /// Who the phone will belong to (`None`: the owner).
    person: Option<String>,
}

/// The phone listener while it runs.
struct Listener {
    addr: SocketAddr,
    port: u16,
    task: tokio::task::JoinHandle<()>,
    /// Set to true to close every phone connection.
    close: watch::Sender<bool>,
}

/// Paired phones, pairing codes and the phone listener.
pub struct Remote {
    private: Vec<u8>,
    public: Vec<u8>,
    data_dir: Option<PathBuf>,
    /// Port to listen on instead of `[remote] port` (tests use 0).
    port_override: Option<u16>,
    phones: Mutex<Vec<Phone>>,
    codes: Mutex<Vec<Code>>,
    fails: Mutex<VecDeque<Instant>>,
    /// Open connections per phone id.
    connected: Mutex<HashMap<String, usize>>,
    listener: Mutex<Option<Listener>>,
    /// Bumped when a phone is removed or given to someone else, so its open connections check
    /// whether they may stay.
    removed: watch::Sender<u64>,
}

/// Why a handshake was turned away (logged; the phone only sees the connection close).
#[derive(Debug, PartialEq, Eq)]
enum Refused {
    Paused,
    Malformed,
    UnknownCode,
    WrongSecret,
    UnknownPhone,
    BadHandshake,
}

impl Remote {
    /// Load (or make) the engine's key pair and the paired phones from `data_dir`. Without a data
    /// folder (tests), the key lives only in memory.
    pub fn new(data_dir: Option<PathBuf>, port_override: Option<u16>) -> Self {
        let (private, public) = load_or_make_keys(data_dir.as_deref());
        let phones = data_dir
            .as_ref()
            .and_then(|d| std::fs::read_to_string(d.join(PHONES_FILE)).ok())
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        Self {
            private,
            public,
            data_dir,
            port_override,
            phones: Mutex::new(phones),
            codes: Mutex::new(Vec::new()),
            fails: Mutex::new(VecDeque::new()),
            connected: Mutex::new(HashMap::new()),
            listener: Mutex::new(None),
            removed: watch::channel(0).0,
        }
    }

    /// Whose a paired phone is: a user id, or `None` for the owner's.
    pub fn person_of(&self, phone: &str) -> Option<String> {
        lock(&self.phones)
            .iter()
            .find(|p| p.id == phone)
            .and_then(|p| p.person.clone())
    }

    /// The engine's public key (what a pairing QR carries).
    pub fn public_key(&self) -> &[u8] {
        &self.public
    }

    /// Where the phone listener is, while it runs.
    pub fn address(&self) -> Option<SocketAddr> {
        lock(&self.listener).as_ref().map(|l| l.addr)
    }

    /// Whether the phone listener is running.
    pub fn listening(&self) -> bool {
        lock(&self.listener).is_some()
    }

    fn save_phones(&self, phones: &[Phone]) {
        let Some(dir) = &self.data_dir else { return };
        match serde_json::to_string_pretty(phones) {
            Ok(text) => {
                if let Err(e) = std::fs::write(dir.join(PHONES_FILE), text) {
                    tracing::warn!(%e, "couldn't save the paired phones");
                }
            }
            Err(e) => tracing::warn!(%e, "couldn't save the paired phones"),
        }
    }

    /// `phone.listed`: the phones and the link's state.
    pub fn listed(&self, state: &AppState) -> Reply {
        let connected = lock(&self.connected).clone();
        let mut items: Vec<PhoneInfo> = lock(&self.phones)
            .iter()
            .map(|p| PhoneInfo {
                phone: p.id.clone(),
                name: p.name.clone(),
                added: p.added,
                last_seen: p.last_seen,
                connected: connected.get(&p.id).is_some_and(|n| *n > 0),
                person: p.person.clone().unwrap_or_else(|| state.owner_id.clone()),
            })
            .collect();
        items.sort_by_key(|p| std::cmp::Reverse(p.added));
        let config = &state.config.remote;
        Reply::PhoneListed {
            items,
            remote: RemoteInfo {
                enabled: config.enabled,
                address: config.address.trim().to_owned(),
                listening: self.listening(),
                port: self
                    .address()
                    .map_or(self.port_override.unwrap_or(config.port), |a| a.port()),
            },
        }
    }

    /// `phone.pair`: a fresh one-time code, as a link and a QR, for `person`'s phone (the caller
    /// checked they exist; `None` or the owner's id: the owner's).
    pub fn pair(&self, state: &AppState, person: Option<String>) -> Result<Reply, String> {
        let person = person.filter(|p| *p != state.owner_id);
        let config = &state.config.remote;
        if !config.enabled {
            return Err("Turn on remote access first (Settings → Phones).".to_owned());
        }
        let address = config.address.trim().trim_end_matches('/');
        if address.is_empty() {
            return Err(
                "Set the address phones use first (Settings → Phones), e.g. your Tailscale Funnel address."
                    .to_owned(),
            );
        }
        let (Some(id), Some(secret)) = (random::<PAIR_ID_BYTES>(), random::<PAIR_SECRET_BYTES>())
        else {
            return Err("Couldn't make a pairing code: no random numbers.".to_owned());
        };
        {
            let mut codes = lock(&self.codes);
            codes.retain(|c| c.expires > Instant::now());
            while codes.len() >= PAIR_MAX_OPEN {
                codes.remove(0);
            }
            codes.push(Code {
                id,
                secret,
                expires: Instant::now() + PAIR_LIFETIME,
                tries: 0,
                person,
            });
        }
        let link = pair_link(address, &self.public, &id, &secret);
        let qr = qrcode::QrCode::new(link.as_bytes()).map_err(|e| e.to_string())?;
        let width = qr.width();
        let rows = qr
            .to_colors()
            .chunks(width)
            .map(|row| {
                row.iter()
                    .map(|c| if *c == qrcode::Color::Dark { '1' } else { '0' })
                    .collect()
            })
            .collect();
        Ok(Reply::PhonePairing {
            link,
            qr: rows,
            expires: now_ms() + PAIR_LIFETIME.as_millis() as i64,
        })
    }

    /// `phone.remove`: forget a phone and close its connections. False if there's no such phone.
    pub fn remove(&self, phone: &str) -> bool {
        let mut phones = lock(&self.phones);
        let before = phones.len();
        phones.retain(|p| p.id != phone);
        if phones.len() == before {
            return false;
        }
        self.save_phones(&phones);
        drop(phones);
        self.removed.send_modify(|n| *n += 1);
        tracing::info!(phone, "phone removed");
        true
    }

    /// `phone.assign`: give a phone to `person` (the caller checked they exist; the owner's id
    /// is stored as no one, like phones from before people). Its open links close so it
    /// reconnects as them. False if there's no such phone.
    pub fn assign(&self, phone: &str, person: &str, owner: &str) -> bool {
        let mut phones = lock(&self.phones);
        let Some(p) = phones.iter_mut().find(|p| p.id == phone) else {
            return false;
        };
        p.person = (person != owner).then(|| person.to_owned());
        self.save_phones(&phones);
        drop(phones);
        self.removed.send_modify(|n| *n += 1);
        tracing::info!(phone, person, "phone given to a person");
        true
    }

    /// How many phones are paired to `person` (phones without a person are `owner`'s).
    pub fn phone_count(&self, person: &str, owner: &str) -> u32 {
        let n = lock(&self.phones)
            .iter()
            .filter(|p| p.person.as_deref().unwrap_or(owner) == person)
            .count();
        u32::try_from(n).unwrap_or(u32::MAX)
    }

    /// Forget every phone paired to `person` (they were removed); their links close.
    pub fn forget_person(&self, person: &str) {
        let mut phones = lock(&self.phones);
        let before = phones.len();
        phones.retain(|p| p.person.as_deref() != Some(person));
        if phones.len() == before {
            return;
        }
        self.save_phones(&phones);
        drop(phones);
        self.removed.send_modify(|n| *n += 1);
        tracing::info!(person, "a removed person's phones were forgotten");
    }

    fn is_paired(&self, phone: &str) -> bool {
        lock(&self.phones).iter().any(|p| p.id == phone)
    }

    fn paused(&self) -> bool {
        let mut fails = lock(&self.fails);
        while fails
            .front()
            .is_some_and(|t| t.elapsed() > LINK_FAIL_WINDOW)
        {
            fails.pop_front();
        }
        fails.len() >= LINK_MAX_FAILS
    }

    fn failed(&self) {
        lock(&self.fails).push_back(Instant::now());
    }

    /// Check a phone's first message. On success the handshake is ready for the engine's answer,
    /// with the phone's id and the answer's payload.
    /// A burst of failures pauses only pairing: a paired phone always gets through (its key can't
    /// be guessed), so a stranger flooding the link can't lock its owner out.
    fn accept(&self, first: &[u8]) -> Result<(HandshakeState, String, Vec<u8>), Refused> {
        let result = match first.split_first() {
            Some((&LINK_PAIR, _)) if self.paused() => return Err(Refused::Paused),
            Some((&LINK_PAIR, rest)) => self.accept_pairing(rest),
            Some((&LINK_CONNECT, rest)) => self.accept_phone(rest),
            _ => Err(Refused::Malformed),
        };
        if result.is_err() {
            self.failed();
        }
        result
    }

    fn accept_pairing(&self, rest: &[u8]) -> Result<(HandshakeState, String, Vec<u8>), Refused> {
        if rest.len() <= PAIR_ID_BYTES {
            return Err(Refused::Malformed);
        }
        let (id, message) = rest.split_at(PAIR_ID_BYTES);
        let (secret, person) = {
            let mut codes = lock(&self.codes);
            codes.retain(|c| c.expires > Instant::now());
            let code = codes
                .iter()
                .find(|c| c.id[..] == *id)
                .ok_or(Refused::UnknownCode)?;
            (code.secret, code.person.clone())
        };
        let mut handshake = responder(NOISE_PAIR, &self.private, Some(&secret))?;
        let mut payload = vec![0u8; NOISE_MAX_MESSAGE];
        let read = match handshake.read_message(message, &mut payload) {
            Ok(n) => n,
            Err(_) => {
                let mut codes = lock(&self.codes);
                if let Some(code) = codes.iter_mut().find(|c| c.id[..] == *id) {
                    code.tries += 1;
                }
                codes.retain(|c| c.tries < PAIR_MAX_TRIES);
                return Err(Refused::WrongSecret);
            }
        };
        let key = handshake
            .get_remote_static()
            .map(hex)
            .ok_or(Refused::BadHandshake)?;
        // The code is used up.
        lock(&self.codes).retain(|c| c.id[..] != *id);
        #[derive(Deserialize, Default)]
        struct Hello {
            #[serde(default)]
            name: String,
        }
        let hello: Hello = serde_json::from_slice(&payload[..read]).unwrap_or_default();
        let name = phone_name(&hello.name);
        let mut phones = lock(&self.phones);
        let id = match phones.iter_mut().find(|p| p.key == key) {
            // The same phone pairing again: keep its id, take the new name and person.
            Some(p) => {
                p.name = name.clone();
                p.person = person;
                p.id.clone()
            }
            None => {
                let id = new_id(id_prefix::PHONE);
                phones.push(Phone {
                    id: id.clone(),
                    name: name.clone(),
                    key,
                    added: now_ms(),
                    last_seen: None,
                    person,
                });
                id
            }
        };
        self.save_phones(&phones);
        drop(phones);
        tracing::info!(phone = id, name, "phone paired");
        let answer = serde_json::json!({ "phone": id }).to_string().into_bytes();
        Ok((handshake, id, answer))
    }

    fn accept_phone(&self, message: &[u8]) -> Result<(HandshakeState, String, Vec<u8>), Refused> {
        let mut handshake = responder(NOISE_CONNECT, &self.private, None)?;
        let mut payload = vec![0u8; NOISE_MAX_MESSAGE];
        handshake
            .read_message(message, &mut payload)
            .map_err(|_| Refused::BadHandshake)?;
        let key = handshake
            .get_remote_static()
            .map(hex)
            .ok_or(Refused::BadHandshake)?;
        let mut phones = lock(&self.phones);
        let phone = phones
            .iter_mut()
            .find(|p| p.key == key)
            .ok_or(Refused::UnknownPhone)?;
        phone.last_seen = Some(now_ms());
        let id = phone.id.clone();
        self.save_phones(&phones);
        Ok((handshake, id, Vec::new()))
    }
}

fn responder(
    pattern: &str,
    private: &[u8],
    secret: Option<&[u8; PAIR_SECRET_BYTES]>,
) -> Result<HandshakeState, Refused> {
    let params = pattern.parse().map_err(|_| Refused::BadHandshake)?;
    let mut builder = Builder::new(params)
        .local_private_key(private)
        .and_then(|b| b.prologue(NOISE_PROLOGUE))
        .map_err(|_| Refused::BadHandshake)?;
    if let Some(secret) = secret {
        builder = builder.psk(1, secret).map_err(|_| Refused::BadHandshake)?;
    }
    builder.build_responder().map_err(|_| Refused::BadHandshake)
}

fn load_or_make_keys(data_dir: Option<&std::path::Path>) -> (Vec<u8>, Vec<u8>) {
    let path = data_dir.map(|d| d.join(REMOTE_KEY_FILE));
    if let Some(text) = path.as_ref().and_then(|p| std::fs::read_to_string(p).ok()) {
        let mut lines = text.lines();
        if let (Some(private), Some(public)) =
            (lines.next().and_then(unhex), lines.next().and_then(unhex))
        {
            return (private, public);
        }
    }
    let made = NOISE_CONNECT
        .parse()
        .ok()
        .and_then(|params| Builder::new(params).generate_keypair().ok());
    let Some(pair) = made else {
        // Never expected: without a key no phone can pair, which the listing shows.
        tracing::error!("couldn't make the phone link's key pair");
        return (Vec::new(), Vec::new());
    };
    if let Some(path) = path {
        let text = format!("{}\n{}\n", hex(&pair.private), hex(&pair.public));
        if let Err(e) = std::fs::write(&path, text) {
            tracing::warn!(%e, "couldn't save the phone link's key pair");
        }
    }
    (pair.private, pair.public)
}

/// The link a pairing QR holds: the address, the engine's key, the code's id and its secret.
fn pair_link(address: &str, public: &[u8], id: &[u8], secret: &[u8]) -> String {
    format!(
        "{PAIR_LINK_PREFIX}a={}&k={}&i={}&s={}",
        B64URL.encode(address),
        B64URL.encode(public),
        B64URL.encode(id),
        B64URL.encode(secret)
    )
}

/// A phone's name, cleaned: no control characters, trimmed, capped.
fn phone_name(raw: &str) -> String {
    let name: String = raw
        .chars()
        .filter(|c| !c.is_control())
        .take(PHONE_NAME_MAX_CHARS)
        .collect();
    let name = name.trim();
    if name.is_empty() {
        PHONE_FALLBACK_NAME.to_owned()
    } else {
        name.to_owned()
    }
}

/// A frame as encrypted pieces, each one Noise message: a marker byte (more to come, or last)
/// then up to [`LINK_PIECE`] bytes of the frame.
pub fn seal(transport: &mut TransportState, frame: &[u8]) -> Result<Vec<Vec<u8>>, snow::Error> {
    let mut chunks: Vec<&[u8]> = frame.chunks(LINK_PIECE).collect();
    if chunks.is_empty() {
        chunks.push(&[]);
    }
    let last = chunks.len() - 1;
    let mut out = Vec::with_capacity(chunks.len());
    let mut plain = Vec::with_capacity(LINK_PIECE + 1);
    for (i, chunk) in chunks.into_iter().enumerate() {
        plain.clear();
        plain.push(if i == last { PIECE_LAST } else { PIECE_MORE });
        plain.extend_from_slice(chunk);
        let mut message = vec![0u8; NOISE_MAX_MESSAGE];
        let n = transport.write_message(&plain, &mut message)?;
        message.truncate(n);
        out.push(message);
    }
    Ok(out)
}

/// Why an encrypted piece couldn't be used.
#[derive(Debug, PartialEq, Eq)]
pub enum OpenError {
    /// It didn't decrypt (tampered with, out of order, or not for this link).
    Undecryptable,
    /// No marker byte.
    Empty,
    /// The frame grew past the limit.
    TooLarge,
}

/// Decrypt one piece into `frame`. Returns the whole frame once its last piece arrives.
pub fn open(
    transport: &mut TransportState,
    message: &[u8],
    frame: &mut Vec<u8>,
) -> Result<Option<Vec<u8>>, OpenError> {
    let mut plain = vec![0u8; NOISE_MAX_MESSAGE];
    let n = transport
        .read_message(message, &mut plain)
        .map_err(|_| OpenError::Undecryptable)?;
    let (&marker, body) = plain[..n].split_first().ok_or(OpenError::Empty)?;
    if frame.len() + body.len() > MAX_FRAME_BYTES * 2 {
        return Err(OpenError::TooLarge);
    }
    frame.extend_from_slice(body);
    Ok((marker == PIECE_LAST).then(|| std::mem::take(frame)))
}

/// Keep the phone listener in step with `[remote] enabled` (and its port) for as long as the
/// engine runs.
pub fn supervise(shared: Arc<Shared>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(REMOTE_CHECK);
        // The port that last failed to open, so the warning isn't repeated every tick.
        let mut failed_port = None;
        loop {
            tick.tick().await;
            let config = shared.hub.state().config.remote.clone();
            let remote = &shared.remote;
            let port = remote.port_override.unwrap_or(config.port);
            let running_port = lock(&remote.listener).as_ref().map(|l| l.port);
            match (config.enabled, running_port) {
                (true, Some(p)) if p == port => {}
                (true, _) => {
                    stop(remote);
                    match open_listener(shared.clone(), port).await {
                        Ok(listener) => {
                            tracing::info!(addr = %listener.addr, "phone link listening");
                            failed_port = None;
                            *lock(&remote.listener) = Some(listener);
                        }
                        Err(e) => {
                            if failed_port != Some(port) {
                                tracing::warn!(%e, port, "couldn't open the phone link");
                                failed_port = Some(port);
                            }
                        }
                    }
                }
                (false, Some(_)) => {
                    stop(remote);
                    tracing::info!("phone link closed");
                }
                (false, None) => failed_port = None,
            }
        }
    })
}

fn stop(remote: &Remote) {
    if let Some(listener) = lock(&remote.listener).take() {
        let _ = listener.close.send(true);
        listener.task.abort();
    }
}

#[derive(Clone)]
struct LinkApp {
    shared: Arc<Shared>,
    close: watch::Receiver<bool>,
}

async fn open_listener(shared: Arc<Shared>, port: u16) -> std::io::Result<Listener> {
    let socket = tokio::net::TcpListener::bind((LOCAL_HOST, port)).await?;
    let addr = socket.local_addr()?;
    let (close, closed) = watch::channel(false);
    let router = Router::new()
        .route(REMOTE_PATH, get(upgrade))
        .with_state(LinkApp {
            shared,
            close: closed,
        });
    let task = tokio::spawn(async move {
        if let Err(e) = axum::serve(socket, router).await {
            tracing::error!(%e, "phone link stopped");
        }
    });
    Ok(Listener {
        addr,
        port,
        task,
        close,
    })
}

/// Only the phone app connects here: a browser page (it sends `Origin`) is turned away.
async fn upgrade(State(app): State<LinkApp>, headers: HeaderMap, ws: WebSocketUpgrade) -> Response {
    if headers.contains_key(header::ORIGIN) {
        return (StatusCode::FORBIDDEN, "Not for browsers.").into_response();
    }
    ws.max_message_size(NOISE_MAX_MESSAGE + PAIR_ID_BYTES + 1)
        .on_upgrade(move |socket| link(socket, app.shared, app.close))
}

/// One phone connection: the handshake, then the protocol over the encrypted link.
async fn link(socket: WebSocket, shared: Arc<Shared>, close: watch::Receiver<bool>) {
    let (mut sink, mut stream) = socket.split();
    let deadline = std::time::Duration::from_secs(HELLO_TIMEOUT_SECS);
    let first = match tokio::time::timeout(deadline, stream.next()).await {
        Ok(Some(Ok(Message::Binary(bytes)))) => bytes,
        _ => return,
    };
    let (mut handshake, phone, answer) = match shared.remote.accept(&first) {
        Ok(accepted) => accepted,
        Err(why) => {
            tracing::warn!(?why, "turned a phone connection away");
            let _ = sink.close().await;
            return;
        }
    };
    let mut reply = vec![0u8; NOISE_MAX_MESSAGE];
    let transport = match handshake.write_message(&answer, &mut reply) {
        Ok(n) => {
            reply.truncate(n);
            match handshake.into_transport_mode() {
                Ok(t) => t,
                Err(_) => return,
            }
        }
        Err(_) => return,
    };
    if sink.send(Message::Binary(reply.into())).await.is_err() {
        return;
    }
    *lock(&shared.remote.connected)
        .entry(phone.clone())
        .or_default() += 1;
    pump(sink, stream, transport, shared.clone(), &phone, close).await;
    if let Some(n) = lock(&shared.remote.connected).get_mut(&phone) {
        *n = n.saturating_sub(1);
    }
}

type WsSink = futures::stream::SplitSink<WebSocket, Message>;
type WsStream = futures::stream::SplitStream<WebSocket>;

/// Carry protocol frames between the socket (encrypted) and the connection code (plain).
async fn pump(
    mut sink: WsSink,
    mut stream: WsStream,
    mut transport: TransportState,
    shared: Arc<Shared>,
    phone: &str,
    mut close: watch::Receiver<bool>,
) {
    // Whose it is when the link opens: given to someone else, it must reconnect as them.
    let mut removed = shared.remote.removed.subscribe();
    let whose = shared.remote.person_of(phone);
    let (to_conn, incoming) = futures::channel::mpsc::unbounded::<Incoming>();
    let (outgoing, mut from_conn) = futures::channel::mpsc::unbounded::<String>();
    let conn = tokio::spawn(serve_link(
        incoming,
        outgoing,
        shared.clone(),
        Peer::Phone {
            id: phone.to_owned(),
        },
    ));
    let mut frame = Vec::new();
    'link: loop {
        tokio::select! {
            msg = stream.next() => match msg {
                Some(Ok(Message::Binary(bytes))) => match open(&mut transport, &bytes, &mut frame) {
                    Ok(Some(whole)) => match String::from_utf8(whole) {
                        Ok(text) => {
                            if to_conn.unbounded_send(Incoming::Text(text)).is_err() {
                                break;
                            }
                        }
                        Err(_) => break,
                    },
                    Ok(None) => {
                        let _ = to_conn.unbounded_send(Incoming::Alive);
                    }
                    Err(why) => {
                        tracing::warn!(?why, phone, "closing a phone link");
                        break;
                    }
                },
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                Some(Ok(_)) => {
                    let _ = to_conn.unbounded_send(Incoming::Alive);
                }
            },
            text = from_conn.next() => {
                let Some(text) = text else { break };
                let Ok(pieces) = seal(&mut transport, text.as_bytes()) else { break };
                for piece in pieces {
                    if sink.send(Message::Binary(piece.into())).await.is_err() {
                        break 'link;
                    }
                }
            }
            _ = close.changed() => break,
            _ = removed.changed() => {
                if !shared.remote.is_paired(phone) || shared.remote.person_of(phone) != whose {
                    break;
                }
            }
        }
    }
    drop(to_conn);
    let _ = sink.close().await;
    let _ = conn.await;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair_of_transports() -> (TransportState, TransportState) {
        let params: snow::params::NoiseParams = NOISE_CONNECT.parse().unwrap();
        let a = Builder::new(params.clone()).generate_keypair().unwrap();
        let b = Builder::new(params.clone()).generate_keypair().unwrap();
        let mut init = Builder::new(params.clone())
            .local_private_key(&a.private)
            .unwrap()
            .remote_public_key(&b.public)
            .unwrap()
            .prologue(NOISE_PROLOGUE)
            .unwrap()
            .build_initiator()
            .unwrap();
        let mut resp = Builder::new(params)
            .local_private_key(&b.private)
            .unwrap()
            .prologue(NOISE_PROLOGUE)
            .unwrap()
            .build_responder()
            .unwrap();
        let (mut m, mut p) = (vec![0u8; NOISE_MAX_MESSAGE], vec![0u8; NOISE_MAX_MESSAGE]);
        let n = init.write_message(&[], &mut m).unwrap();
        resp.read_message(&m[..n], &mut p).unwrap();
        let n = resp.write_message(&[], &mut m).unwrap();
        init.read_message(&m[..n], &mut p).unwrap();
        (
            init.into_transport_mode().unwrap(),
            resp.into_transport_mode().unwrap(),
        )
    }

    #[test]
    fn big_frames_go_in_pieces_and_come_back_whole() {
        let (mut a, mut b) = pair_of_transports();
        let frame: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
        let pieces = seal(&mut a, &frame).unwrap();
        assert_eq!(pieces.len(), 200_000usize.div_ceil(LINK_PIECE));
        assert!(pieces.iter().all(|p| p.len() <= NOISE_MAX_MESSAGE));
        let mut partial = Vec::new();
        let mut whole = None;
        for p in &pieces {
            whole = open(&mut b, p, &mut partial).unwrap();
        }
        assert_eq!(whole.unwrap(), frame);
        // A small frame is one piece.
        let one = seal(&mut a, b"{}").unwrap();
        assert_eq!(one.len(), 1);
        assert_eq!(open(&mut b, &one[0], &mut partial).unwrap().unwrap(), b"{}");
    }

    #[test]
    fn a_tampered_or_replayed_piece_is_refused() {
        let (mut a, mut b) = pair_of_transports();
        let replay = seal(&mut a, b"hello").unwrap().remove(0);
        let mut partial = Vec::new();
        assert!(open(&mut b, &replay, &mut partial).unwrap().is_some());
        // The same piece again is out of order.
        assert_eq!(
            open(&mut b, &replay, &mut partial),
            Err(OpenError::Undecryptable)
        );
        let mut fresh = seal(&mut a, b"hello").unwrap().remove(0);
        fresh[3] ^= 1;
        assert_eq!(
            open(&mut b, &fresh, &mut partial),
            Err(OpenError::Undecryptable)
        );
    }

    #[test]
    fn phones_belong_to_a_person_or_else_the_owner() {
        let dir = std::env::temp_dir().join(format!("thrall-remote-people-{}", now_ms()));
        std::fs::create_dir_all(&dir).unwrap();
        // A phone paired before people existed has no person; a newer one names its person.
        std::fs::write(
            dir.join(PHONES_FILE),
            r#"[{"id":"p_old","name":"Old","key":"aa","added":1},
                {"id":"p_new","name":"New","key":"bb","added":2,"person":"u_sam"}]"#,
        )
        .unwrap();
        let remote = Remote::new(Some(dir.clone()), Some(0));
        assert_eq!(remote.person_of("p_old"), None);
        assert_eq!(remote.person_of("p_new").as_deref(), Some("u_sam"));
        assert_eq!(remote.person_of("p_missing"), None);
        // Saving keeps the person, and doesn't add one to phones without.
        remote.save_phones(&lock(&remote.phones));
        let saved = std::fs::read_to_string(dir.join(PHONES_FILE)).unwrap();
        assert_eq!(saved.matches("\"person\"").count(), 1, "{saved}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn phone_names_are_cleaned() {
        assert_eq!(
            phone_name("  Nothing Phone (4a) Pro "),
            "Nothing Phone (4a) Pro"
        );
        assert_eq!(phone_name("a\u{0}b\nc"), "abc");
        assert_eq!(phone_name(""), PHONE_FALLBACK_NAME);
        assert_eq!(phone_name(&"x".repeat(500)).len(), PHONE_NAME_MAX_CHARS);
    }

    #[test]
    fn keys_are_made_once_and_kept() {
        let dir = std::env::temp_dir().join(format!("thrall-remote-key-{}", now_ms()));
        std::fs::create_dir_all(&dir).unwrap();
        let first = Remote::new(Some(dir.clone()), None);
        let again = Remote::new(Some(dir.clone()), None);
        assert_eq!(first.public_key().len(), 32);
        assert_eq!(first.public_key(), again.public_key());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_pairing_link_carries_what_the_phone_needs() {
        let link = pair_link("https://pc.example.ts.net", &[1; 32], &[2; 16], &[3; 32]);
        let query = link.strip_prefix(PAIR_LINK_PREFIX).unwrap();
        let fields: HashMap<&str, Vec<u8>> = query
            .split('&')
            .map(|kv| {
                let (k, v) = kv.split_once('=').unwrap();
                (k, B64URL.decode(v).unwrap())
            })
            .collect();
        assert_eq!(fields["a"], b"https://pc.example.ts.net");
        assert_eq!(fields["k"], vec![1; 32]);
        assert_eq!(fields["i"], vec![2; 16]);
        assert_eq!(fields["s"], vec![3; 32]);
    }
}
