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
use axum::extract::{ConnectInfo, State, WebSocketUpgrade};
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
use thrall_protocol::{BlockedPcInfo, PcLinkInfo, PcRequestInfo, PhoneInfo, RemoteInfo, Reply};
use tokio::sync::watch;

use crate::connection::{Incoming, Peer, Shared, serve_link};
use thrall_core::constants::PC_DISCOVERY_PORT;

use crate::constants::{
    ANY_HOST, ASK_CODE_MODULUS, ASK_LIFETIME, ASK_MAX_OPEN, BLOCKED_PCS_FILE, DISCOVER_HELLO,
    LINK_ASK, LINK_CONNECT, LINK_FAIL_WINDOW, LINK_MAX_FAILS, LINK_PAIR, LINK_PIECE, LOCAL_HOST,
    MASTER_FALLBACK_NAME, NOISE_ASK, NOISE_CONNECT, NOISE_MAX_MESSAGE, NOISE_PAIR, NOISE_PROLOGUE,
    PAIR_ID_BYTES, PAIR_LIFETIME, PAIR_LINK_PREFIX, PAIR_MAX_OPEN, PAIR_MAX_TRIES,
    PAIR_SECRET_BYTES, PC_FALLBACK_NAME, PHONE_FALLBACK_NAME, PHONE_NAME_MAX_CHARS, PHONES_FILE,
    PIECE_LAST, PIECE_MORE, REMOTE_CHECK, REMOTE_KEY_FILE,
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

/// What a paired device is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Kind {
    /// A phone, paired from a QR.
    #[default]
    Phone,
    /// A PC joined as someone's companion (it asked, the owner accepted).
    Pc,
}

impl Kind {
    fn is_phone(&self) -> bool {
        *self == Kind::Phone
    }

    fn as_str(self) -> &'static str {
        match self {
            Kind::Phone => "phone",
            Kind::Pc => "pc",
        }
    }
}

/// A paired phone (or companion PC), as kept in `phones.json`.
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
    /// A phone or a PC (absent in files from before PCs: a phone).
    #[serde(default, skip_serializing_if = "Kind::is_phone")]
    kind: Kind,
}

/// A PC asking to join, waiting for the owner.
struct Ask {
    id: String,
    /// Its public key, hex.
    key: String,
    name: String,
    code: String,
    address: String,
    at: i64,
    expires: Instant,
    /// Tells the waiting link what the owner decided (dropped: the request ended).
    decide: tokio::sync::oneshot::Sender<Verdict>,
}

/// What the owner decided about a PC asking to join.
#[derive(Debug, PartialEq, Eq)]
enum Verdict {
    /// Let in, as this device.
    Accepted(String),
    /// Turned away.
    Declined,
}

/// A PC blocked from asking, as kept in `blocked-pcs.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Blocked {
    id: String,
    /// Its public key, hex.
    key: String,
    name: String,
    at: i64,
}

/// What came of a PC asking.
enum Opened {
    /// It's already joined, as this device.
    Known(String),
    /// Waiting for the owner, as this request.
    Waiting(String, tokio::sync::oneshot::Receiver<Verdict>),
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
    /// PCs asking to join, waiting for the owner.
    asks: Mutex<Vec<Ask>>,
    /// PCs blocked from asking.
    blocked: Mutex<Vec<Blocked>>,
    /// The PC listener (every network) while it runs.
    pc_listener: Mutex<Option<Listener>>,
    /// Answers companions looking for Thrall on the network, while the PC listener runs.
    finder: Mutex<Option<tokio::task::JoinHandle<()>>>,
    /// Bumped when the PCs asking change, so the owner's screens ask for the list again.
    pub(crate) asked: watch::Sender<u64>,
    /// People whose phones the owner turned off (their phone links close).
    phones_off: Mutex<std::collections::HashSet<String>>,
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
    /// A PC asking from the PC listener's port on the phone listener.
    NotHere,
    /// A blocked PC asking.
    Blocked,
    /// Too many PCs asking at once.
    Busy,
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
        let blocked = data_dir
            .as_ref()
            .and_then(|d| std::fs::read_to_string(d.join(BLOCKED_PCS_FILE)).ok())
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
            asks: Mutex::new(Vec::new()),
            blocked: Mutex::new(blocked),
            pc_listener: Mutex::new(None),
            finder: Mutex::new(None),
            asked: watch::channel(0).0,
            phones_off: Mutex::new(std::collections::HashSet::new()),
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
                kind: p.kind.as_str().to_owned(),
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
            .filter(|p| p.kind == Kind::Phone && p.person.as_deref().unwrap_or(owner) == person)
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

    /// Let `person`'s phones connect, or not (their open phone links close).
    pub fn phones_allowed(&self, person: &str, allowed: bool) {
        let changed = {
            let mut off = lock(&self.phones_off);
            if allowed {
                off.remove(person)
            } else {
                off.insert(person.to_owned())
            }
        };
        if changed && !allowed {
            self.removed.send_modify(|n| *n += 1);
        }
    }

    /// Whether `phone`'s person has phones turned off (PCs aren't phones).
    fn phone_muted(&self, phone: &str, owner_less: Option<&str>) -> bool {
        let phones = lock(&self.phones);
        let Some(p) = phones.iter().find(|p| p.id == phone) else {
            return false;
        };
        p.kind == Kind::Phone
            && p.person
                .as_deref()
                .or(owner_less)
                .is_some_and(|who| lock(&self.phones_off).contains(who))
    }

    /// Whether `device` is a PC that joined (not a phone).
    pub fn is_pc(&self, device: &str) -> bool {
        lock(&self.phones)
            .iter()
            .any(|p| p.id == device && p.kind == Kind::Pc)
    }

    /// The PCs that joined as `person`'s, oldest first (the owner's: `owner`).
    pub fn pcs_of(&self, person: &str, owner: &str) -> Vec<String> {
        let mut pcs: Vec<(i64, String)> = lock(&self.phones)
            .iter()
            .filter(|p| p.kind == Kind::Pc && p.person.as_deref().unwrap_or(owner) == person)
            .map(|p| (p.added, p.id.clone()))
            .collect();
        pcs.sort();
        pcs.into_iter().map(|(_, id)| id).collect()
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
                    kind: Kind::Phone,
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

    fn save_blocked(&self, blocked: &[Blocked]) {
        let Some(dir) = &self.data_dir else { return };
        match serde_json::to_string_pretty(blocked) {
            Ok(text) => {
                if let Err(e) = std::fs::write(dir.join(BLOCKED_PCS_FILE), text) {
                    tracing::warn!(%e, "couldn't save the blocked PCs");
                }
            }
            Err(e) => tracing::warn!(%e, "couldn't save the blocked PCs"),
        }
    }

    fn asks_changed(&self) {
        self.asked.send_modify(|n| *n += 1);
    }

    /// A PC that finished the handshake asks to join. A joined PC is let straight back in; a
    /// blocked one, or one too many, is turned away.
    fn ask_open(
        &self,
        key: &str,
        name: &str,
        code: &str,
        address: &str,
    ) -> Result<Opened, Refused> {
        if lock(&self.blocked).iter().any(|b| b.key == key) {
            return Err(Refused::Blocked);
        }
        if let Some(p) = lock(&self.phones).iter().find(|p| p.key == key) {
            return Ok(Opened::Known(p.id.clone()));
        }
        let mut asks = lock(&self.asks);
        asks.retain(|a| a.expires > Instant::now() && !a.decide.is_closed());
        // The same PC asking again replaces its older request.
        asks.retain(|a| a.key != key);
        if asks.len() >= ASK_MAX_OPEN {
            return Err(Refused::Busy);
        }
        let (decide, decided) = tokio::sync::oneshot::channel();
        let id = new_id(id_prefix::REQUEST);
        asks.push(Ask {
            id: id.clone(),
            key: key.to_owned(),
            name: name.to_owned(),
            code: code.to_owned(),
            address: address.to_owned(),
            at: now_ms(),
            expires: Instant::now() + ASK_LIFETIME,
            decide,
        });
        drop(asks);
        tracing::info!(request = id, name, address, "a PC asked to join");
        self.asks_changed();
        Ok(Opened::Waiting(id, decided))
    }

    /// A request ended without an answer (the PC gave up or it ran out of time).
    fn ask_ended(&self, request: &str) {
        let mut asks = lock(&self.asks);
        let before = asks.len();
        asks.retain(|a| a.id != request);
        let changed = asks.len() != before;
        drop(asks);
        if changed {
            self.asks_changed();
        }
    }

    /// Every request ends (PCs were switched off).
    fn asks_cleared(&self) {
        let had = !std::mem::take(&mut *lock(&self.asks)).is_empty();
        if had {
            self.asks_changed();
        }
    }

    /// `pc.requests`: who's asking, who's blocked, and the PC listener.
    pub fn requests(&self, state: &AppState) -> Reply {
        let mut asks = lock(&self.asks);
        asks.retain(|a| a.expires > Instant::now() && !a.decide.is_closed());
        let items = asks
            .iter()
            .map(|a| PcRequestInfo {
                request: a.id.clone(),
                name: a.name.clone(),
                code: a.code.clone(),
                address: a.address.clone(),
                at: a.at,
            })
            .collect();
        drop(asks);
        let mut blocked: Vec<BlockedPcInfo> = lock(&self.blocked)
            .iter()
            .map(|b| BlockedPcInfo {
                blocked: b.id.clone(),
                name: b.name.clone(),
                at: b.at,
            })
            .collect();
        blocked.sort_by_key(|b| std::cmp::Reverse(b.at));
        let config = &state.config.remote;
        let listener = lock(&self.pc_listener);
        Reply::PcRequests {
            items,
            blocked,
            pcs: PcLinkInfo {
                enabled: config.pcs,
                listening: listener.is_some(),
                port: listener
                    .as_ref()
                    .map_or(self.port_override.unwrap_or(config.pc_port), |l| {
                        l.addr.port()
                    }),
                name: computer_name(),
            },
        }
    }

    /// `pc.accept`: let a PC that's asking join as `person`'s (`None`: the owner's). When `code`
    /// is given it must be the one both screens show.
    pub fn accept_pc(
        &self,
        request: &str,
        person: Option<String>,
        code: Option<&str>,
    ) -> Result<(), String> {
        let mut asks = lock(&self.asks);
        let Some(at) = asks.iter().position(|a| a.id == request) else {
            return Err("That PC isn't asking any more.".to_owned());
        };
        if let Some(code) = code
            && digits(code) != digits(&asks[at].code)
        {
            return Err("That isn't the code the PC shows.".to_owned());
        }
        let ask = asks.remove(at);
        drop(asks);
        let mut phones = lock(&self.phones);
        let id = new_id(id_prefix::PHONE);
        phones.push(Phone {
            id: id.clone(),
            name: ask.name.clone(),
            key: ask.key,
            added: now_ms(),
            last_seen: None,
            person,
            kind: Kind::Pc,
        });
        self.save_phones(&phones);
        drop(phones);
        tracing::info!(device = id, name = ask.name, "a PC joined");
        // The PC may have just given up; it joined all the same and connects next time.
        let _ = ask.decide.send(Verdict::Accepted(id));
        self.asks_changed();
        Ok(())
    }

    /// `pc.decline`: turn a PC that's asking away, and with `block` keep it from asking again.
    /// False if it isn't asking.
    pub fn decline_pc(&self, request: &str, block: bool) -> bool {
        let mut asks = lock(&self.asks);
        let Some(at) = asks.iter().position(|a| a.id == request) else {
            return false;
        };
        let ask = asks.remove(at);
        drop(asks);
        if block {
            let mut blocked = lock(&self.blocked);
            blocked.retain(|b| b.key != ask.key);
            blocked.push(Blocked {
                id: new_id(id_prefix::PHONE),
                key: ask.key.clone(),
                name: ask.name.clone(),
                at: now_ms(),
            });
            self.save_blocked(&blocked);
        }
        tracing::info!(request, name = ask.name, block, "a PC was turned away");
        let _ = ask.decide.send(Verdict::Declined);
        self.asks_changed();
        true
    }

    /// `pc.unblock`: let a blocked PC ask again. False if there's no such PC.
    pub fn unblock_pc(&self, blocked: &str) -> bool {
        let mut list = lock(&self.blocked);
        let before = list.len();
        list.retain(|b| b.id != blocked);
        if list.len() == before {
            return false;
        }
        self.save_blocked(&list);
        true
    }
}

/// Only the digits of a code ("483 912" and "483912" are the same code).
fn digits(code: &str) -> String {
    code.chars().filter(char::is_ascii_digit).collect()
}

/// The code both screens show for a PC asking to join, from the finished handshake's hash: a
/// man in the middle makes two different handshakes, so the screens wouldn't match.
pub fn ask_code(handshake_hash: &[u8]) -> String {
    let mut first = [0u8; 4];
    for (to, from) in first.iter_mut().zip(handshake_hash) {
        *to = *from;
    }
    let n = u32::from_be_bytes(first) % ASK_CODE_MODULUS;
    format!("{:03} {:03}", n / 1000, n % 1000)
}

/// This computer's name, as companions looking for Thrall see it.
pub fn computer_name() -> String {
    ["COMPUTERNAME", "HOSTNAME"]
        .iter()
        .find_map(|v| std::env::var(v).ok().filter(|n| !n.trim().is_empty()))
        .or_else(|| {
            std::fs::read_to_string("/etc/hostname")
                .ok()
                .map(|n| n.trim().to_owned())
                .filter(|n| !n.is_empty())
        })
        .unwrap_or_else(|| MASTER_FALLBACK_NAME.to_owned())
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
    device_name(raw, PHONE_FALLBACK_NAME)
}

/// A device's name, cleaned: no control characters, trimmed, capped; `fallback` when empty.
fn device_name(raw: &str, fallback: &str) -> String {
    let name: String = raw
        .chars()
        .filter(|c| !c.is_control())
        .take(PHONE_NAME_MAX_CHARS)
        .collect();
    let name = name.trim();
    if name.is_empty() {
        fallback.to_owned()
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

/// Keep the phone listener in step with `[remote] enabled`, and the PC listener with
/// `[remote] pcs` (and their ports), for as long as the engine runs.
pub fn supervise(shared: Arc<Shared>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(REMOTE_CHECK);
        // The ports that last failed to open, so the warning isn't repeated every tick.
        let mut failed_port = None;
        let mut failed_pc_port = None;
        loop {
            tick.tick().await;
            let config = shared.hub.state().config.remote.clone();
            let remote = &shared.remote;
            let port = remote.port_override.unwrap_or(config.port);
            let running_port = lock(&remote.listener).as_ref().map(|l| l.port);
            match (config.enabled, running_port) {
                (true, Some(p)) if p == port => {}
                (true, _) => {
                    stop(&remote.listener);
                    match open_listener(shared.clone(), LOCAL_HOST, port, false).await {
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
                    stop(&remote.listener);
                    tracing::info!("phone link closed");
                }
                (false, None) => failed_port = None,
            }
            let pc_port = remote.port_override.unwrap_or(config.pc_port);
            let running_pc_port = lock(&remote.pc_listener).as_ref().map(|l| l.port);
            match (config.pcs, running_pc_port) {
                (true, Some(p)) if p == pc_port => {}
                (true, _) => {
                    stop_pcs(remote);
                    // Tests pick their own ports and stay on this computer.
                    let host = if remote.port_override.is_some() {
                        LOCAL_HOST
                    } else {
                        ANY_HOST
                    };
                    match open_listener(shared.clone(), host, pc_port, true).await {
                        Ok(listener) => {
                            tracing::info!(addr = %listener.addr, "PC link listening");
                            failed_pc_port = None;
                            if remote.port_override.is_none() {
                                let port = listener.addr.port();
                                *lock(&remote.finder) = Some(tokio::spawn(answer_finds(port)));
                            }
                            *lock(&remote.pc_listener) = Some(listener);
                        }
                        Err(e) => {
                            if failed_pc_port != Some(pc_port) {
                                tracing::warn!(%e, port = pc_port, "couldn't open the PC link");
                                failed_pc_port = Some(pc_port);
                            }
                        }
                    }
                }
                (false, Some(_)) => {
                    stop_pcs(remote);
                    tracing::info!("PC link closed");
                }
                (false, None) => failed_pc_port = None,
            }
        }
    })
}

fn stop(listener: &Mutex<Option<Listener>>) {
    if let Some(listener) = lock(listener).take() {
        let _ = listener.close.send(true);
        listener.task.abort();
    }
}

/// Close the PC listener, stop answering on the network, and end every request.
fn stop_pcs(remote: &Remote) {
    stop(&remote.pc_listener);
    if let Some(finder) = lock(&remote.finder).take() {
        finder.abort();
    }
    remote.asks_cleared();
}

/// Answer companions looking for Thrall on the network ([`DISCOVER_HELLO`] broadcasts) with this
/// computer's name and the PC listener's port. Nothing else gets an answer.
async fn answer_finds(port: u16) {
    let socket = match tokio::net::UdpSocket::bind((ANY_HOST, PC_DISCOVERY_PORT)).await {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(%e, "couldn't listen for companions looking for Thrall");
            return;
        }
    };
    let answer = serde_json::json!({ "thrall": 1, "name": computer_name(), "port": port })
        .to_string()
        .into_bytes();
    let mut buf = [0u8; 64];
    loop {
        // Errors (Windows reports an earlier answer that bounced) are passed over.
        let Ok((n, from)) = socket.recv_from(&mut buf).await else {
            continue;
        };
        if buf.get(..n) == Some(DISCOVER_HELLO) {
            let _ = socket.send_to(&answer, from).await;
        }
    }
}

#[derive(Clone)]
struct LinkApp {
    shared: Arc<Shared>,
    close: watch::Receiver<bool>,
    /// The PC listener: PCs may ask to join here.
    pcs: bool,
}

async fn open_listener(
    shared: Arc<Shared>,
    host: &str,
    port: u16,
    pcs: bool,
) -> std::io::Result<Listener> {
    let socket = tokio::net::TcpListener::bind((host, port)).await?;
    let addr = socket.local_addr()?;
    let (close, closed) = watch::channel(false);
    let router = Router::new()
        .route(REMOTE_PATH, get(upgrade))
        .with_state(LinkApp {
            shared,
            close: closed,
            pcs,
        });
    let task = tokio::spawn(async move {
        let service = router.into_make_service_with_connect_info::<SocketAddr>();
        if let Err(e) = axum::serve(socket, service).await {
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

/// Only the phone app and companions connect here: a browser page (it sends `Origin`) is turned
/// away.
async fn upgrade(
    State(app): State<LinkApp>,
    ConnectInfo(from): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    if headers.contains_key(header::ORIGIN) {
        return (StatusCode::FORBIDDEN, "Not for browsers.").into_response();
    }
    ws.max_message_size(NOISE_MAX_MESSAGE + PAIR_ID_BYTES + 1)
        .on_upgrade(move |socket| link(socket, app.shared, app.close, app.pcs, from))
}

/// One phone (or PC) connection: the handshake, then the protocol over the encrypted link. On
/// the PC listener a PC may instead ask to join.
async fn link(
    socket: WebSocket,
    shared: Arc<Shared>,
    close: watch::Receiver<bool>,
    pcs: bool,
    from: SocketAddr,
) {
    let (mut sink, mut stream) = socket.split();
    let deadline = std::time::Duration::from_secs(HELLO_TIMEOUT_SECS);
    let first = match tokio::time::timeout(deadline, stream.next()).await {
        Ok(Some(Ok(Message::Binary(bytes)))) => bytes,
        _ => return,
    };
    if let Some((&LINK_ASK, rest)) = first.split_first() {
        if pcs {
            ask_to_join(sink, stream, rest, &shared, from, close).await;
        } else {
            tracing::warn!(why = ?Refused::NotHere, "turned a phone connection away");
            shared.remote.failed();
            let _ = sink.close().await;
        }
        return;
    }
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

/// A PC asking to join: the rest of the `Noise_XX` handshake (its first message is `first`),
/// then it waits, link open, for the owner to accept, decline or block it. The answer is one
/// encrypted frame, `{"approved": true, "device": "<id>"}` or `{"approved": false}`, and the link
/// closes; an accepted PC then connects like a paired phone.
async fn ask_to_join(
    mut sink: WsSink,
    mut stream: WsStream,
    first: &[u8],
    shared: &Shared,
    from: SocketAddr,
    mut close: watch::Receiver<bool>,
) {
    // Asking isn't held up by a burst of failed connections (an old PC still trying its old key):
    // every request waits for the owner, and only a few wait at once.
    let remote = &shared.remote;
    let Ok(mut handshake) = responder(NOISE_ASK, &remote.private, None) else {
        return;
    };
    let mut buf = vec![0u8; NOISE_MAX_MESSAGE];
    if handshake.read_message(first, &mut buf).is_err() {
        remote.failed();
        let _ = sink.close().await;
        return;
    }
    let Ok(n) = handshake.write_message(&[], &mut buf) else {
        return;
    };
    if sink
        .send(Message::Binary(buf[..n].to_vec().into()))
        .await
        .is_err()
    {
        return;
    }
    let deadline = std::time::Duration::from_secs(HELLO_TIMEOUT_SECS);
    let third = match tokio::time::timeout(deadline, stream.next()).await {
        Ok(Some(Ok(Message::Binary(bytes)))) => bytes,
        _ => return,
    };
    let Ok(read) = handshake.read_message(&third, &mut buf) else {
        remote.failed();
        let _ = sink.close().await;
        return;
    };
    #[derive(Deserialize, Default)]
    struct Hello {
        #[serde(default)]
        name: String,
    }
    let hello: Hello = serde_json::from_slice(&buf[..read]).unwrap_or_default();
    let name = device_name(&hello.name, PC_FALLBACK_NAME);
    let Some(key) = handshake.get_remote_static().map(hex) else {
        return;
    };
    let code = ask_code(handshake.get_handshake_hash());
    let Ok(mut transport) = handshake.into_transport_mode() else {
        return;
    };
    let verdict = match remote.ask_open(&key, &name, &code, &from.ip().to_string()) {
        Ok(Opened::Known(device)) => Verdict::Accepted(device),
        Ok(Opened::Waiting(request, mut decided)) => {
            let expired = tokio::time::sleep(ASK_LIFETIME);
            tokio::pin!(expired);
            let verdict = loop {
                tokio::select! {
                    v = &mut decided => break v.ok(),
                    () = &mut expired => break None,
                    _ = close.changed() => break None,
                    msg = stream.next() => match msg {
                        Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break None,
                        Some(Ok(_)) => {}
                    },
                }
            };
            remote.ask_ended(&request);
            match verdict {
                Some(v) => v,
                None => {
                    let _ = sink.close().await;
                    return;
                }
            }
        }
        Err(why) => {
            tracing::warn!(?why, name, "turned a PC away");
            Verdict::Declined
        }
    };
    let answer = match &verdict {
        Verdict::Accepted(device) => serde_json::json!({ "approved": true, "device": device }),
        Verdict::Declined => serde_json::json!({ "approved": false }),
    };
    if let Ok(pieces) = seal(&mut transport, answer.to_string().as_bytes()) {
        for piece in pieces {
            if sink.send(Message::Binary(piece.into())).await.is_err() {
                break;
            }
        }
    }
    let _ = sink.close().await;
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
                if !shared.remote.is_paired(phone)
                    || shared.remote.person_of(phone) != whose
                    || shared.remote.phone_muted(phone, None)
                {
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
