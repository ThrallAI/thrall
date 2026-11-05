//! Finding the engine, starting it if it isn't running, and a small protocol client: the app
//! watches every session's events (for the tray and notifications) and can ask the engine to stop.

use std::path::PathBuf;
use std::process::Stdio;
use std::time::Instant;

use futures::{SinkExt, StreamExt};
use thrall_core::constants::HOME_ENV_VAR;
use thrall_core::{EngineFile, Paths};
use thrall_protocol::constants::PROTOCOL_VERSION;
use thrall_protocol::{
    ClientInfo, ClientKind, EventFrame, Reply, Request, RequestFrame, ServerFrame, SessionInfo,
};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

use crate::constants::{
    ENGINE_ARG, ENGINE_EXE, ENGINE_EXE_ENV, ENGINE_POLL, ENGINE_START_TIMEOUT, PING_INTERVAL,
    QUIT_MESSAGE, QUIT_PHONES_ID, RUNNER_ARG, TITLE_LOOKUP,
};

/// Thrall's data folder (honours THRALL_HOME, like every other part of Thrall).
pub fn paths() -> anyhow::Result<Paths> {
    Ok(Paths::resolve(
        std::env::var_os(HOME_ENV_VAR).map(PathBuf::from),
    )?)
}

/// What the app learns from the engine.
pub enum Incoming {
    /// A session event.
    Event(Box<EventFrame>),
    /// Sessions known when the connection opened (for their titles).
    Sessions(Vec<SessionInfo>),
    /// How many phones are connected right now (asked for by Quit).
    PhonesConnected(usize),
}

/// A live connection: send requests with `send`; events arrive on `incoming`.
pub struct Connection {
    out: mpsc::UnboundedSender<String>,
    /// Ends when the connection drops.
    pub incoming: mpsc::UnboundedReceiver<Incoming>,
    /// Where the web app is.
    pub web_url: String,
    /// A one-time code that signs the window in (the page itself carries no token).
    pub signin: Option<String>,
    next_id: u64,
}

impl Connection {
    /// Fire a request without waiting for its reply.
    pub fn send(&mut self, request: Request) {
        self.next_id += 1;
        let frame = RequestFrame::new(format!("d-{}", self.next_id), request);
        if let Ok(text) = serde_json::to_string(&frame) {
            let _ = self.out.send(text);
        }
    }

    /// A sender other tasks can use (the Quit menu asks the engine to stop).
    pub fn sender(&self) -> mpsc::UnboundedSender<String> {
        self.out.clone()
    }
}

/// A frame asking the engine to stop.
pub fn stop_frame() -> String {
    serde_json::to_string(&RequestFrame::new("d-stop", Request::EngineStop {})).unwrap_or_default()
}

/// A frame asking which phones are paired (Quit counts the connected ones).
pub fn phones_frame() -> String {
    serde_json::to_string(&RequestFrame::new(QUIT_PHONES_ID, Request::PhoneList {}))
        .unwrap_or_default()
}

/// The Quit dialog's text, naming the phones that stopping the engine would disconnect.
pub fn quit_message(phones: usize) -> String {
    match phones {
        0 => QUIT_MESSAGE.to_owned(),
        1 => format!("{QUIT_MESSAGE}\n\n1 phone is connected now; stopping Thrall disconnects it."),
        n => format!(
            "{QUIT_MESSAGE}\n\n{n} phones are connected now; stopping Thrall disconnects them."
        ),
    }
}

/// Whether this PC is a companion of Thrall on another PC (it joined one): then `thrall runner`
/// stands in for the engine.
pub fn companion(paths: &Paths) -> bool {
    paths
        .home
        .join(thrall_core::constants::COMPANION_FILE)
        .exists()
}

/// Connect to the running engine (or, on a companion PC, its runner), starting it if it isn't
/// running.
pub async fn connect_or_start(paths: &Paths) -> anyhow::Result<Connection> {
    if let Ok(conn) = connect(paths).await {
        return Ok(conn);
    }
    start_engine(if companion(paths) {
        RUNNER_ARG
    } else {
        ENGINE_ARG
    })?;
    let started = Instant::now();
    loop {
        tokio::time::sleep(ENGINE_POLL).await;
        match connect(paths).await {
            Ok(conn) => return Ok(conn),
            Err(e) if started.elapsed() >= ENGINE_START_TIMEOUT => {
                anyhow::bail!("Thrall's engine didn't start: {e}")
            }
            Err(_) => {}
        }
    }
}

/// The `thrall` program beside this one.
pub fn engine_exe() -> PathBuf {
    std::env::var_os(ENGINE_EXE_ENV)
        .map(PathBuf::from)
        .or_else(|| {
            std::env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(|d| d.join(ENGINE_EXE)))
                .filter(|p| p.exists())
        })
        .unwrap_or_else(|| PathBuf::from(ENGINE_EXE))
}

/// Start `thrall <what>` (the engine, or a companion's runner) beside this program, detached so it
/// can outlive the app.
fn start_engine(what: &str) -> anyhow::Result<()> {
    let exe = engine_exe();
    let mut command = std::process::Command::new(&exe);
    if let Some(home) = Paths::user_home() {
        command.current_dir(home);
    }
    command
        .arg(what)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(crate::constants::DETACHED_FLAGS);
    }
    command
        .spawn()
        .map_err(|e| anyhow::anyhow!("Couldn't start {}: {e}", exe.display()))?;
    Ok(())
}

/// Connect to the engine described by engine.json, say hello and watch every session.
pub async fn connect(paths: &Paths) -> anyhow::Result<Connection> {
    let file = EngineFile::read(&paths.engine_file)
        .ok_or_else(|| anyhow::anyhow!("no engine is running"))?;
    let token = std::fs::read_to_string(&paths.token_file)?
        .trim()
        .to_owned();
    let (socket, _) = tokio_tungstenite::connect_async(&file.ws_url).await?;
    let (mut sink, mut stream) = socket.split();

    let hello = RequestFrame::new(
        "d-hello",
        Request::Hello {
            protocols: vec![PROTOCOL_VERSION],
            client: ClientInfo {
                kind: ClientKind::Desktop,
                name: "Thrall Desktop".to_owned(),
                version: env!("CARGO_PKG_VERSION").to_owned(),
            },
            token,
        },
    );
    sink.send(Message::Text(serde_json::to_string(&hello)?.into()))
        .await?;
    let me = loop {
        match stream.next().await {
            Some(Ok(Message::Text(text))) => {
                if let Ok(ServerFrame::Reply(r)) =
                    serde_json::from_str::<ServerFrame>(text.as_str())
                {
                    match r.reply {
                        Reply::Welcome { user, .. } => break user.id,
                        other => anyhow::bail!("the engine refused: {other:?}"),
                    }
                }
            }
            Some(Ok(_)) => {}
            _ => anyhow::bail!("the engine closed the connection"),
        }
    };
    // The web app's page carries no token, so the window signs in with a one-time link.
    let ask = RequestFrame::new("d-signin", Request::SigninCreate { person: me });
    sink.send(Message::Text(serde_json::to_string(&ask)?.into()))
        .await?;
    let signin = loop {
        match stream.next().await {
            Some(Ok(Message::Text(text))) => {
                if let Ok(ServerFrame::Reply(r)) =
                    serde_json::from_str::<ServerFrame>(text.as_str())
                    && r.re == "d-signin"
                {
                    match r.reply {
                        Reply::SigninCode { code, .. } => break Some(code),
                        other => {
                            tracing::warn!(?other, "the engine gave no sign-in link");
                            break None;
                        }
                    }
                }
            }
            Some(Ok(_)) => {}
            _ => anyhow::bail!("the engine closed the connection"),
        }
    };

    let (out, mut out_rx) = mpsc::unbounded_channel::<String>();
    let (in_tx, incoming) = mpsc::unbounded_channel::<Incoming>();
    tokio::spawn(async move {
        let mut ping = tokio::time::interval(PING_INTERVAL);
        ping.tick().await;
        loop {
            let text = tokio::select! {
                next = out_rx.recv() => match next {
                    Some(t) => t,
                    None => break,
                },
                _ = ping.tick() => serde_json::to_string(&RequestFrame::new("d-ping", Request::Ping {}))
                    .unwrap_or_default(),
            };
            if sink.send(Message::Text(text.into())).await.is_err() {
                break;
            }
        }
    });
    tokio::spawn(async move {
        while let Some(Ok(msg)) = stream.next().await {
            let Message::Text(text) = msg else { continue };
            let sent = match serde_json::from_str::<ServerFrame>(text.as_str()) {
                Ok(ServerFrame::Event(e)) => in_tx.send(Incoming::Event(Box::new(e))),
                Ok(ServerFrame::Reply(r)) => match r.reply {
                    Reply::SessionListed { items } => in_tx.send(Incoming::Sessions(items)),
                    // Home's pages aren't in the session list.
                    Reply::HomeShown { page, earlier } => {
                        let mut items = earlier;
                        items.push(page);
                        in_tx.send(Incoming::Sessions(items))
                    }
                    Reply::PhoneListed { items, .. } if r.re == QUIT_PHONES_ID => in_tx.send(
                        Incoming::PhonesConnected(items.iter().filter(|p| p.connected).count()),
                    ),
                    _ => Ok(()),
                },
                // An event this version doesn't know: ignore it.
                Err(_) => Ok(()),
            };
            if sent.is_err() {
                break;
            }
        }
    });

    let mut conn = Connection {
        out,
        incoming,
        web_url: file.web_url,
        signin,
        next_id: 0,
    };
    conn.send(Request::SessionWatchAll {});
    conn.send(Request::SessionList {
        limit: Some(TITLE_LOOKUP),
        archived: false,
    });
    conn.send(Request::HomeGet {});
    Ok(conn)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_quit_message_names_connected_phones() {
        assert_eq!(quit_message(0), QUIT_MESSAGE);
        assert!(
            quit_message(1).ends_with("1 phone is connected now; stopping Thrall disconnects it.")
        );
        assert!(quit_message(3).contains("3 phones are connected now"));
    }
}
