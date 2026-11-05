//! A companion PC's side of its link: finding Thrall on the network, asking to join, and then
//! (as `thrall runner`) staying connected to do this PC's share of the work.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use snow::{Builder, TransportState};
use thrall_core::constants::{PC_DISCOVERY_PORT, id_prefix};
use thrall_core::ids::new_id;
use thrall_protocol::constants::REMOTE_PATH;
use thrall_protocol::{ClientInfo, ClientKind, Event, Reply, Request, RequestFrame, ServerFrame};
use thrall_tools::companion::{Call, Runner};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

use crate::constants::{
    ANY_HOST, ASK_LIFETIME, COMPANION_FILE, COMPANION_PING, DISCOVER_HELLO, LINK_ASK, LINK_CONNECT,
    NOISE_ASK, NOISE_CONNECT, NOISE_MAX_MESSAGE, NOISE_PROLOGUE, OUTPUT_FLUSH,
};
use crate::remote::{ask_code, open, seal};

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(text: &str) -> Option<Vec<u8>> {
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(text.get(i..i + 2)?, 16).ok())
        .collect()
}

/// This PC, joined to a Thrall: where it is, its key, and this PC's own key pair. Kept in the data
/// folder (`companion.json`), like the phone link's key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Joined {
    /// Where the Thrall is: `host:port`.
    pub address: String,
    /// Its computer's name.
    pub name: String,
    /// Its public key, hex.
    pub master_key: String,
    /// This PC's private key, hex.
    pub private: String,
    /// This PC's public key, hex.
    pub public: String,
    /// This PC's id there.
    pub device: String,
}

impl Joined {
    /// The joined Thrall saved in `dir`, if this PC joined one.
    pub fn load(dir: &Path) -> Option<Self> {
        let text = std::fs::read_to_string(dir.join(COMPANION_FILE)).ok()?;
        serde_json::from_str(&text).ok()
    }

    /// Save it in `dir`.
    pub fn save(&self, dir: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        let text = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        std::fs::write(dir.join(COMPANION_FILE), text)
    }

    /// Forget it (this PC leaves).
    pub fn forget(dir: &Path) -> std::io::Result<()> {
        match std::fs::remove_file(dir.join(COMPANION_FILE)) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            other => other,
        }
    }
}

/// A Thrall found on the network.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    /// Its computer's name.
    pub name: String,
    /// `host:port`.
    pub address: String,
}

/// Look for Thralls that let PCs join, on every network this PC is on, for `wait`.
pub async fn find(wait: Duration) -> Vec<Found> {
    let Ok(socket) = tokio::net::UdpSocket::bind((ANY_HOST, 0)).await else {
        return Vec::new();
    };
    if socket.set_broadcast(true).is_err()
        || socket
            .send_to(DISCOVER_HELLO, ("255.255.255.255", PC_DISCOVERY_PORT))
            .await
            .is_err()
    {
        return Vec::new();
    }
    #[derive(Deserialize)]
    struct Answer {
        thrall: u32,
        name: String,
        port: u16,
    }
    let deadline = Instant::now() + wait;
    let mut found: Vec<Found> = Vec::new();
    let mut buf = [0u8; 512];
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break;
        }
        let Ok(Ok((n, from))) = tokio::time::timeout(left, socket.recv_from(&mut buf)).await else {
            break;
        };
        let Some(answer) = buf
            .get(..n)
            .and_then(|b| serde_json::from_slice::<Answer>(b).ok())
            .filter(|a| a.thrall == 1)
        else {
            continue;
        };
        let address = format!("{}:{}", from.ip(), answer.port);
        if !found.iter().any(|f| f.address == address) {
            found.push(Found {
                name: answer.name,
                address,
            });
        }
    }
    found
}

/// `host` with the PC port when it has none (`kids-pc` → `kids-pc:7420`).
pub fn with_port(host: &str) -> String {
    let host = host.trim();
    if host
        .rsplit_once(':')
        .is_some_and(|(_, p)| p.parse::<u16>().is_ok())
    {
        host.to_owned()
    } else {
        format!("{host}:{}", thrall_core::constants::DEFAULT_PC_PORT)
    }
}

async fn open_socket(address: &str) -> Result<Socket, String> {
    let url = format!("ws://{address}{REMOTE_PATH}");
    let (socket, _) = tokio_tungstenite::connect_async(url)
        .await
        .map_err(|e| format!("couldn't reach Thrall at {address}: {e}"))?;
    Ok(socket)
}

async fn next_binary(socket: &mut Socket, wait: Duration) -> Option<Vec<u8>> {
    let deadline = Instant::now() + wait;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match tokio::time::timeout(left, socket.next()).await {
            Ok(Some(Ok(Message::Binary(bytes)))) => return Some(bytes.to_vec()),
            Ok(Some(Ok(Message::Close(_)))) | Ok(None) | Ok(Some(Err(_))) | Err(_) => return None,
            Ok(Some(Ok(_))) => {}
        }
    }
}

/// Ask the Thrall at `address` to let this PC (called `name`) join. `show_code` gets the code
/// to show the person while the owner decides. `None`: turned away.
pub async fn ask(
    address: &str,
    name: &str,
    show_code: impl FnOnce(&str),
) -> Result<Option<Joined>, String> {
    let pattern = NOISE_ASK.parse().map_err(|e| format!("{e:?}"))?;
    let keys = Builder::new(pattern)
        .generate_keypair()
        .map_err(|e| e.to_string())?;
    let mut socket = open_socket(address).await?;
    let pattern = NOISE_ASK.parse().map_err(|e| format!("{e:?}"))?;
    let mut hs = Builder::new(pattern)
        .local_private_key(&keys.private)
        .and_then(|b| b.prologue(NOISE_PROLOGUE))
        .and_then(|b| b.build_initiator())
        .map_err(|e| e.to_string())?;
    let mut buf = vec![0u8; NOISE_MAX_MESSAGE];
    let n = hs.write_message(&[], &mut buf).map_err(|e| e.to_string())?;
    let first = [&[LINK_ASK][..], &buf[..n]].concat();
    socket
        .send(Message::Binary(first.into()))
        .await
        .map_err(|e| e.to_string())?;
    let wait = Duration::from_secs(thrall_protocol::constants::HELLO_TIMEOUT_SECS);
    let second = next_binary(&mut socket, wait)
        .await
        .ok_or("Thrall turned this PC away (is it blocked?)")?;
    hs.read_message(&second, &mut buf)
        .map_err(|_| "that isn't a Thrall this PC can join")?;
    let hello = serde_json::json!({ "name": name }).to_string();
    let n = hs
        .write_message(hello.as_bytes(), &mut buf)
        .map_err(|e| e.to_string())?;
    socket
        .send(Message::Binary(buf[..n].to_vec().into()))
        .await
        .map_err(|e| e.to_string())?;
    let code = ask_code(hs.get_handshake_hash());
    let master_key = hs
        .get_remote_static()
        .map(hex)
        .ok_or("no key from Thrall")?;
    let mut transport = hs.into_transport_mode().map_err(|e| e.to_string())?;
    show_code(&code);
    let mut partial = Vec::new();
    let answer = loop {
        let bytes = next_binary(&mut socket, ASK_LIFETIME + Duration::from_secs(60))
            .await
            .ok_or("the request ran out, or Thrall closed it")?;
        if let Ok(Some(whole)) = open(&mut transport, &bytes, &mut partial) {
            break whole;
        }
    };
    let _ = socket.close(None).await;
    let answer: serde_json::Value = serde_json::from_slice(&answer).map_err(|e| e.to_string())?;
    if answer["approved"] != true {
        return Ok(None);
    }
    let device = answer["device"].as_str().unwrap_or_default().to_owned();
    Ok(Some(Joined {
        address: address.to_owned(),
        name: String::new(),
        master_key,
        private: hex(&keys.private),
        public: hex(&keys.public),
        device,
    }))
}

/// An open, encrypted link to the joined Thrall, speaking the ordinary protocol.
pub struct Link {
    socket: Socket,
    transport: TransportState,
    partial: Vec<u8>,
}

impl Link {
    /// Connect as the joined PC.
    pub async fn connect(joined: &Joined) -> Result<Self, String> {
        let private = unhex(&joined.private).ok_or("this PC's key is damaged; join again")?;
        let master = unhex(&joined.master_key).ok_or("Thrall's key is damaged; join again")?;
        let mut socket = open_socket(&joined.address).await?;
        let pattern = NOISE_CONNECT.parse().map_err(|e| format!("{e:?}"))?;
        let mut hs = Builder::new(pattern)
            .local_private_key(&private)
            .and_then(|b| b.remote_public_key(&master))
            .and_then(|b| b.prologue(NOISE_PROLOGUE))
            .and_then(|b| b.build_initiator())
            .map_err(|e| e.to_string())?;
        let mut buf = vec![0u8; NOISE_MAX_MESSAGE];
        let n = hs.write_message(&[], &mut buf).map_err(|e| e.to_string())?;
        let first = [&[LINK_CONNECT][..], &buf[..n]].concat();
        socket
            .send(Message::Binary(first.into()))
            .await
            .map_err(|e| e.to_string())?;
        let wait = Duration::from_secs(thrall_protocol::constants::HELLO_TIMEOUT_SECS);
        let answer = next_binary(&mut socket, wait)
            .await
            .ok_or("Thrall didn't let this PC in (was it removed?)")?;
        hs.read_message(&answer, &mut buf)
            .map_err(|_| "that isn't the Thrall this PC joined")?;
        Ok(Self {
            transport: hs.into_transport_mode().map_err(|e| e.to_string())?,
            socket,
            partial: Vec::new(),
        })
    }

    /// Send one frame.
    pub async fn send(&mut self, text: &str) -> Result<(), String> {
        let pieces = seal(&mut self.transport, text.as_bytes()).map_err(|e| e.to_string())?;
        for piece in pieces {
            self.socket
                .send(Message::Binary(piece.into()))
                .await
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    /// The next whole frame; `None` when the link closed.
    pub async fn next(&mut self) -> Option<Result<String, String>> {
        loop {
            match self.socket.next().await? {
                Ok(Message::Binary(bytes)) => {
                    match open(&mut self.transport, &bytes, &mut self.partial) {
                        Ok(Some(whole)) => {
                            return Some(String::from_utf8(whole).map_err(|e| e.to_string()));
                        }
                        Ok(None) => {}
                        Err(why) => return Some(Err(format!("{why:?}"))),
                    }
                }
                Ok(Message::Close(_)) => return None,
                Ok(_) => {}
                Err(e) => return Some(Err(e.to_string())),
            }
        }
    }

    /// Say hello as `kind` and wait for the welcome.
    pub async fn hello(&mut self, kind: ClientKind, version: &str) -> Result<(), String> {
        let hello = Request::Hello {
            protocols: vec![1],
            client: ClientInfo {
                kind,
                name: "Thrall companion".to_owned(),
                version: version.to_owned(),
            },
            token: String::new(),
        };
        self.send(&frame("hello", hello)).await?;
        loop {
            let text = self.next().await.ok_or("Thrall closed the link")??;
            if let Ok(ServerFrame::Reply(r)) = serde_json::from_str::<ServerFrame>(&text)
                && r.re == "hello"
            {
                return match r.reply {
                    Reply::Welcome { .. } => Ok(()),
                    Reply::Error { message, .. } => Err(message),
                    other => Err(format!("unexpected answer to hello: {other:?}")),
                };
            }
        }
    }
}

fn frame(id: &str, request: Request) -> String {
    serde_json::to_string(&RequestFrame::new(id, request)).unwrap_or_default()
}

/// Be this PC's runner on one connection, until it closes: tell Thrall this PC's folder (`home`),
/// then run the calls it sends.
pub async fn run_once(
    joined: &Joined,
    runner: Arc<Runner>,
    home: &str,
    version: &str,
) -> Result<(), String> {
    let mut link = Link::connect(joined).await?;
    link.hello(ClientKind::Companion, version).await?;
    link.send(&frame(
        "ready",
        Request::RunnerReady {
            home: home.to_owned(),
        },
    ))
    .await?;
    tracing::info!(
        address = joined.address,
        "connected to Thrall as its companion"
    );
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<String>();
    let mut ping = tokio::time::interval(COMPANION_PING);
    loop {
        tokio::select! {
            text = link.next() => {
                let text = match text {
                    None => return Ok(()),
                    Some(t) => t?,
                };
                let Ok(ServerFrame::Event(event)) = serde_json::from_str::<ServerFrame>(&text) else {
                    continue;
                };
                if let Event::RunnerCall { call, person, in_session, workspace, job } = event.event {
                    tokio::spawn(handle(
                        runner.clone(),
                        out_tx.clone(),
                        home.to_owned(),
                        call,
                        person,
                        in_session,
                        workspace,
                        job,
                    ));
                }
            }
            out = out_rx.recv() => {
                if let Some(text) = out {
                    link.send(&text).await?;
                }
            }
            _ = ping.tick() => link.send(&frame(&new_id(id_prefix::REQUEST), Request::Ping {})).await?,
        }
    }
}

/// A folder call (the Files view, picking or making a folder), answered on this PC. `home` is
/// this PC's Thrall folder.
async fn folder_call(job: Call, workspace: &str, home: &str) -> Result<serde_json::Value, String> {
    use base64::Engine as _;
    match job {
        Call::List { path } => {
            let (path, items, more) = crate::folders::list(Path::new(workspace), &path)?;
            Ok(serde_json::json!({ "path": path, "items": items, "more": more }))
        }
        Call::Part { path, offset } => {
            let (path, offset, size, bytes) =
                crate::folders::read(Path::new(workspace), &path, offset).await?;
            let data = base64::engine::general_purpose::STANDARD.encode(bytes);
            Ok(serde_json::json!({ "path": path, "offset": offset, "size": size, "data": data }))
        }
        Call::Places { recent } => {
            let recent: Vec<String> = recent
                .into_iter()
                .filter(|p| Path::new(p).is_dir())
                .take(crate::constants::PLACES_MAX)
                .collect();
            Ok(serde_json::json!({ "recent": recent, "default": home, "new_in": home }))
        }
        Call::MakeFolder { parent, name } => {
            // This PC is the person's own: a new folder can go in any folder that's there, and
            // in its Thrall folder (made when first used).
            let path = crate::folders::make_in(&parent, &name, &[home])?;
            Ok(serde_json::json!({ "path": path }))
        }
        _ => Err("not a folder call".to_owned()),
    }
}

/// Run one call and send back its output and answer.
#[allow(clippy::too_many_arguments)]
async fn handle(
    runner: Arc<Runner>,
    out: mpsc::UnboundedSender<String>,
    home: String,
    call: String,
    person: String,
    session: String,
    workspace: String,
    job: serde_json::Value,
) {
    let (lines_tx, mut lines_rx) = mpsc::unbounded_channel::<String>();
    let forward = {
        let out = out.clone();
        let call = call.clone();
        tokio::spawn(async move {
            while let Some(text) = lines_rx.recv().await {
                let request = Request::RunnerOutput {
                    call: call.clone(),
                    text,
                };
                let _ = out.send(frame(&new_id(id_prefix::REQUEST), request));
            }
        })
    };
    let result = match serde_json::from_value::<Call>(job) {
        Ok(job) if job.is_folder_call() => folder_call(job, &workspace, &home).await,
        Ok(job) => {
            runner
                .handle(
                    &call,
                    job,
                    Path::new(&workspace),
                    &person,
                    &session,
                    lines_tx,
                )
                .await
        }
        Err(e) => Err(format!(
            "this PC's Thrall doesn't understand that ({e}); update it"
        )),
    };
    // Lines still on their way go first.
    let _ = tokio::time::timeout(OUTPUT_FLUSH, forward).await;
    let (result, error) = match result {
        Ok(v) => (Some(v), None),
        Err(e) => (None, Some(e)),
    };
    let _ = out.send(frame(
        &new_id(id_prefix::REQUEST),
        Request::RunnerDone {
            call,
            result,
            error,
        },
    ));
}
