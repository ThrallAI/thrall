//! The terminal's connection to the engine: protocol frames over a local WebSocket. Requests get
//! their replies through [`Client::request`]; session events arrive on the `events` channel.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use thrall_protocol::constants::{PING_INTERVAL_SECS, PROTOCOL_VERSION};
use thrall_protocol::{
    ClientInfo, ClientKind, EngineInfo, ErrorCode, EventFrame, Reply, Request, RequestFrame,
    ServerFrame,
};
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite::Message;

use crate::constants::{CLIENT_DISPLAY_NAME, REQUEST_TIMEOUT};

/// A request the engine refused, in words for the user.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct Refused {
    /// Stable code.
    pub code: ErrorCode,
    /// Plain-language explanation.
    pub message: String,
}

type Pending = Arc<Mutex<HashMap<String, oneshot::Sender<Reply>>>>;

/// Sends requests to the engine.
#[derive(Clone)]
pub struct Client {
    out: mpsc::UnboundedSender<String>,
    pending: Pending,
    next_id: Arc<AtomicU64>,
}

/// A live connection.
pub struct Connected {
    /// For requests.
    pub client: Client,
    /// Session events; closes when the connection drops.
    pub events: mpsc::UnboundedReceiver<EventFrame>,
    /// About the engine (version, model).
    pub engine: EngineInfo,
}

/// Connect to the engine at `url` as the terminal app and say hello.
pub async fn connect(url: &str, token: &str) -> anyhow::Result<Connected> {
    connect_as(url, token, ClientKind::Tui, CLIENT_DISPLAY_NAME).await
}

/// Connect as another kind of client (`thrall run` is `headless`).
pub async fn connect_as(
    url: &str,
    token: &str,
    kind: ClientKind,
    name: &str,
) -> anyhow::Result<Connected> {
    let (socket, _) = tokio_tungstenite::connect_async(url).await?;
    let (mut sink, mut stream) = socket.split();
    let (out, mut out_rx) = mpsc::unbounded_channel::<String>();
    let (events_tx, events) = mpsc::unbounded_channel::<EventFrame>();
    let pending: Pending = Arc::new(Mutex::new(HashMap::new()));

    // Writer: frames out, plus the heartbeat.
    tokio::spawn(async move {
        let mut ping = tokio::time::interval(Duration::from_secs(PING_INTERVAL_SECS));
        ping.tick().await;
        let mut pings = 0u64;
        loop {
            let text = tokio::select! {
                next = out_rx.recv() => match next {
                    Some(text) => text,
                    None => break,
                },
                _ = ping.tick() => {
                    pings += 1;
                    let frame = RequestFrame::new(format!("ping-{pings}"), Request::Ping {});
                    serde_json::to_string(&frame).unwrap_or_default()
                }
            };
            if sink.send(Message::Text(text.into())).await.is_err() {
                break;
            }
        }
        let _ = sink.close().await;
    });

    // Reader: replies to whoever asked, events to the app.
    let waiting = pending.clone();
    tokio::spawn(async move {
        while let Some(Ok(msg)) = stream.next().await {
            let Message::Text(text) = msg else { continue };
            match serde_json::from_str::<ServerFrame>(text.as_str()) {
                Ok(ServerFrame::Reply(r)) => {
                    let tx = waiting.lock().ok().and_then(|mut p| p.remove(&r.re));
                    if let Some(tx) = tx {
                        let _ = tx.send(r.reply);
                    }
                }
                Ok(ServerFrame::Event(e)) => {
                    if events_tx.send(e).is_err() {
                        break;
                    }
                }
                // Must ignore: an event type this build doesn't know yet.
                Err(e) => tracing::debug!(%e, "ignoring a frame this version doesn't understand"),
            }
        }
        // Dropping the senders tells the app the connection is gone.
        if let Ok(mut p) = waiting.lock() {
            p.clear();
        }
    });

    let client = Client {
        out,
        pending,
        next_id: Arc::new(AtomicU64::new(0)),
    };
    let hello = client
        .request(Request::Hello {
            protocols: vec![PROTOCOL_VERSION],
            client: ClientInfo {
                kind,
                name: name.to_owned(),
                version: env!("CARGO_PKG_VERSION").to_owned(),
            },
            token: token.to_owned(),
        })
        .await?;
    let Reply::Welcome { engine, .. } = hello else {
        anyhow::bail!("Thrall's engine answered hello with something unexpected.");
    };
    Ok(Connected {
        client,
        events,
        engine,
    })
}

impl Client {
    /// Send a request and wait for its reply. An `error` reply comes back as [`Refused`].
    pub async fn request(&self, request: Request) -> anyhow::Result<Reply> {
        let id = format!("c-{}", self.next_id.fetch_add(1, Ordering::SeqCst) + 1);
        let (tx, rx) = oneshot::channel();
        if let Ok(mut p) = self.pending.lock() {
            p.insert(id.clone(), tx);
        }
        let text = serde_json::to_string(&RequestFrame::new(&id, request))?;
        if self.out.send(text).is_err() {
            anyhow::bail!("Lost the connection to Thrall's engine.");
        }
        let reply = match tokio::time::timeout(REQUEST_TIMEOUT, rx).await {
            Ok(Ok(reply)) => reply,
            Ok(Err(_)) => anyhow::bail!("Lost the connection to Thrall's engine."),
            Err(_) => {
                if let Ok(mut p) = self.pending.lock() {
                    p.remove(&id);
                }
                anyhow::bail!("Thrall's engine didn't answer in time.");
            }
        };
        match reply {
            Reply::Error { code, message, .. } => Err(Refused { code, message }.into()),
            other => Ok(other),
        }
    }
}
