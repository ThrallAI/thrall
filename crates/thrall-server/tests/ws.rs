//! The protocol endpoint end to end: real sockets, a real hub, a fake AI provider.
// Integration tests are their own crate, so the in-tests unwrap exemption doesn't reach helpers.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::{SinkExt, StreamExt, stream};
use thrall_agent::{AppState, Hub};
use thrall_core::Config;
use thrall_llm::{
    ChatRequest, LlmError, LlmProvider, ReplyStream, StreamEvent, TokenUsage, ToolCallDelta,
};
use thrall_protocol::{
    ClientInfo, ClientKind, ErrorCode, Event, PermissionChoice, Reply, Request, RequestFrame,
    ServerFrame, SessionState,
};
use thrall_server::{Options, start};
use thrall_store::{SqliteStore, Store};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

type Script = Vec<Result<StreamEvent, LlmError>>;

/// Plays one scripted reply per call; the last one repeats.
struct FakeProvider {
    scripts: Mutex<VecDeque<Script>>,
}

impl LlmProvider for FakeProvider {
    fn name(&self) -> &str {
        "Fake"
    }

    fn stream(&self, _request: ChatRequest) -> ReplyStream {
        let mut scripts = self.scripts.lock().unwrap();
        let script = if scripts.len() > 1 {
            scripts.pop_front().unwrap()
        } else {
            scripts.front().cloned().unwrap_or_default()
        };
        stream::iter(script).boxed()
    }
}

fn usage() -> Result<StreamEvent, LlmError> {
    Ok(StreamEvent::Usage(TokenUsage {
        input_tokens: 10,
        cached_tokens: 0,
        output_tokens: 4,
        cost_usd: Some(0.002),
    }))
}

fn reply(text: &str) -> Script {
    vec![
        Ok(StreamEvent::Text(text.to_owned())),
        Ok(StreamEvent::Finished(Some("stop".into()))),
        usage(),
    ]
}

fn tool_call(name: &str, arguments: &str) -> Script {
    vec![
        Ok(StreamEvent::ToolCall(ToolCallDelta {
            index: 0,
            id: Some("c1".into()),
            name: Some(name.into()),
            arguments: arguments.into(),
        })),
        Ok(StreamEvent::Finished(Some("tool_calls".into()))),
        usage(),
    ]
}

const TOKEN: &str = "test-token";

/// A hub on a free port; returns the WebSocket URL.
async fn engine(scripts: Vec<Script>) -> (String, Arc<Hub>) {
    let store = SqliteStore::open_in_memory().await.unwrap();
    store.ensure_user("u_test", "Test", 0).await.unwrap();
    store.set_role("u_test", "owner").await.unwrap();
    let provider = Arc::new(FakeProvider {
        scripts: Mutex::new(scripts.into()),
    });
    let mut state = AppState::new(
        Config::default(),
        Arc::new(store),
        provider,
        "u_test".into(),
    )
    .unwrap();
    // Uploads are kept in the data folder.
    let data = std::env::temp_dir().join(format!("thrall-server-data-{}", std::process::id()));
    std::fs::create_dir_all(&data).unwrap();
    state.data_dir = Some(data);
    let hub = Hub::new(state, None);
    let running = start(
        hub.clone(),
        Options {
            port: 0,
            token: TOKEN.into(),
            version: "0.0.0-test".into(),
            extra_origins: Vec::new(),
            serve_web: true,
            remote_port: Some(0),
        },
    )
    .await
    .unwrap();
    (format!("ws://{}/v1/ws", running.addr), hub)
}

fn workspace(name: &str) -> String {
    let dir = std::env::temp_dir().join(format!("thrall-server-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.to_string_lossy().into_owned()
}

/// A folder that isn't there.
fn nowhere() -> String {
    std::env::temp_dir()
        .join("thrall-no-such-folder")
        .to_string_lossy()
        .into_owned()
}

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

struct Client {
    socket: Socket,
    next_id: u32,
    /// Events that arrived while a request waited for its reply, kept for `events_until`.
    held: std::collections::VecDeque<ServerFrame>,
}

impl Client {
    /// A client on an open socket that hasn't said hello yet.
    fn wrap(socket: Socket) -> Self {
        Self {
            socket,
            next_id: 0,
            held: std::collections::VecDeque::new(),
        }
    }

    async fn connect(url: &str, kind: ClientKind) -> Self {
        let (socket, _) = tokio_tungstenite::connect_async(url).await.unwrap();
        let mut client = Self::wrap(socket);
        let welcome = client
            .request(Request::Hello {
                protocols: vec![1],
                client: ClientInfo {
                    kind,
                    name: "Test".into(),
                    version: "0".into(),
                },
                token: TOKEN.into(),
            })
            .await;
        assert!(matches!(welcome, Reply::Welcome { protocol: 1, .. }));
        client
    }

    async fn send(&mut self, request: Request) -> String {
        self.next_id += 1;
        let id = format!("c-{}", self.next_id);
        let text = serde_json::to_string(&RequestFrame::new(&id, request)).unwrap();
        self.socket.send(Message::Text(text.into())).await.unwrap();
        id
    }

    async fn frame(&mut self) -> ServerFrame {
        if let Some(held) = self.held.pop_front() {
            return held;
        }
        loop {
            let msg = tokio::time::timeout(Duration::from_secs(10), self.socket.next())
                .await
                .expect("no frame within 10 s")
                .unwrap()
                .unwrap();
            if let Message::Text(text) = msg {
                return serde_json::from_str(text.as_str()).unwrap();
            }
        }
    }

    /// Send a request and return its reply. Events that arrive first are kept for later reads
    /// (an answer's `permission.resolved` can beat its `ack`).
    async fn request(&mut self, request: Request) -> Reply {
        let id = self.send(request).await;
        let mut events = Vec::new();
        let reply = loop {
            match self.frame().await {
                ServerFrame::Reply(r) if r.re == id => break r.reply,
                ServerFrame::Reply(_) => {}
                event @ ServerFrame::Event(_) => events.push(event),
            }
        };
        // Ahead of anything held from before, in the order they came.
        for event in events.into_iter().rev() {
            self.held.push_front(event);
        }
        reply
    }

    /// Events until (and including) the first that matches `stop`.
    async fn events_until(&mut self, stop: impl Fn(&Event) -> bool) -> Vec<Event> {
        let mut seen = Vec::new();
        loop {
            if let ServerFrame::Event(f) = self.frame().await {
                let done = stop(&f.event);
                seen.push(f.event);
                if done {
                    return seen;
                }
            }
        }
    }
}

fn opened(reply: Reply) -> String {
    match reply {
        Reply::SessionOpened(info) => info.session,
        other => panic!("expected session.opened, got {other:?}"),
    }
}

#[tokio::test]
async fn a_turn_streams_to_the_client() {
    let (url, _hub) = engine(vec![reply("Hello there."), reply("Greeting")]).await;
    let mut c = Client::connect(&url, ClientKind::Web).await;
    let session = opened(
        c.request(Request::SessionCreate {
            workspace: Some(workspace("turn")),
            title: None,
            mode: None,
        })
        .await,
    );
    let ack = c
        .request(Request::TurnSend {
            session: session.clone(),
            text: "hi".into(),
            files: Vec::new(),
        })
        .await;
    assert_eq!(ack, Reply::Ack {});
    let events = c
        .events_until(|e| {
            matches!(
                e,
                Event::Status {
                    state: SessionState::Idle,
                    ..
                }
            )
        })
        .await;
    let names: Vec<&str> = events.iter().map(|e| e.type_name()).collect();
    assert!(names.contains(&"turn.started"));
    assert!(names.contains(&"message.delta"));
    assert!(names.contains(&"turn.completed"));
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::MessageCompleted { text, .. } if text == "Hello there."))
    );
    assert!(events.iter().any(|e| matches!(
        e,
        Event::Status {
            state: SessionState::Working,
            ..
        }
    )));
}

#[tokio::test]
async fn a_wrong_token_is_refused() {
    let (url, _hub) = engine(vec![reply("x")]).await;
    let (mut socket, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
    let hello = RequestFrame::new(
        "c-1",
        Request::Hello {
            protocols: vec![1],
            client: ClientInfo {
                kind: ClientKind::Tui,
                name: "Test".into(),
                version: "0".into(),
            },
            token: "wrong".into(),
        },
    );
    socket
        .send(Message::Text(serde_json::to_string(&hello).unwrap().into()))
        .await
        .unwrap();
    let Some(Ok(Message::Text(text))) = socket.next().await else {
        panic!("expected an error frame");
    };
    let frame: ServerFrame = serde_json::from_str(text.as_str()).unwrap();
    assert!(matches!(
        frame,
        ServerFrame::Reply(r) if matches!(r.reply, Reply::Error { code: ErrorCode::Unauthorized, .. })
    ));
    // Then the engine closes the connection.
    let next = tokio::time::timeout(Duration::from_secs(5), socket.next())
        .await
        .unwrap();
    assert!(matches!(
        next,
        None | Some(Ok(Message::Close(_))) | Some(Err(_))
    ));
}

#[tokio::test]
async fn other_web_pages_cannot_connect() {
    let (url, _hub) = engine(vec![reply("x")]).await;
    let mut request = url.as_str().into_client_request().unwrap();
    request
        .headers_mut()
        .insert("Origin", "https://evil.example".parse().unwrap());
    assert!(tokio_tungstenite::connect_async(request).await.is_err());
}

#[tokio::test]
async fn unknown_requests_are_unsupported() {
    let (url, _hub) = engine(vec![reply("x")]).await;
    let mut c = Client::connect(&url, ClientKind::Web).await;
    c.socket
        .send(Message::Text(
            r#"{"v":1,"id":"c-99","type":"teleport.now"}"#.into(),
        ))
        .await
        .unwrap();
    let ServerFrame::Reply(r) = c.frame().await else {
        panic!("expected a reply");
    };
    assert_eq!(r.re, "c-99");
    assert!(matches!(
        r.reply,
        Reply::Error {
            code: ErrorCode::Unsupported,
            ..
        }
    ));
}

#[tokio::test]
async fn one_answer_wins_across_clients() {
    let (url, _hub) = engine(vec![
        tool_call("shell", r#"{"command": "echo from-test"}"#),
        reply("Ran it."),
        reply("Ran a command"),
    ])
    .await;
    let mut terminal = Client::connect(&url, ClientKind::Tui).await;
    let mut browser = Client::connect(&url, ClientKind::Web).await;
    let session = opened(
        terminal
            .request(Request::SessionCreate {
                workspace: Some(workspace("answer")),
                title: None,
                mode: None,
            })
            .await,
    );
    opened(
        browser
            .request(Request::SessionOpen {
                session: session.clone(),
                after_seq: None,
            })
            .await,
    );
    terminal
        .request(Request::TurnSend {
            session: session.clone(),
            text: "run it".into(),
            files: Vec::new(),
        })
        .await;

    // Both see the card and the waiting state.
    let mut request_id = String::new();
    for client in [&mut terminal, &mut browser] {
        let events = client
            .events_until(|e| {
                matches!(
                    e,
                    Event::Status {
                        state: SessionState::WaitingForYou,
                        ..
                    }
                )
            })
            .await;
        for e in events {
            if let Event::PermissionRequest { request, .. } = e {
                request_id = request;
            }
        }
    }
    assert!(!request_id.is_empty());

    // The browser answers first; the terminal's late answer is refused.
    let ok = browser
        .request(Request::PermissionAnswer {
            request: request_id.clone(),
            choice: PermissionChoice::Yes,
        })
        .await;
    assert_eq!(ok, Reply::Ack {});
    let late = terminal
        .request(Request::PermissionAnswer {
            request: request_id.clone(),
            choice: PermissionChoice::No,
        })
        .await;
    assert!(matches!(
        late,
        Reply::Error {
            code: ErrorCode::AlreadyAnswered,
            ..
        }
    ));

    // Both are told who answered, and the turn finishes.
    for client in [&mut terminal, &mut browser] {
        let events = client
            .events_until(|e| matches!(e, Event::TurnCompleted { .. }))
            .await;
        assert!(events.iter().any(|e| matches!(
            e,
            Event::PermissionResolved { answered_by, choice: PermissionChoice::Yes, .. }
                if answered_by == "web"
        )));
    }
}

#[tokio::test]
async fn each_person_chooses_their_own_personality() {
    let (url, _hub) = engine(vec![reply("x")]).await;
    let mut c = Client::connect(&url, ClientKind::Web).await;
    let Reply::PersonalityShown { presets, .. } = c.request(Request::PersonalityGet {}).await
    else {
        panic!("expected personality.shown");
    };
    assert_eq!(presets[0].id, "standard", "the shipped one comes first");
    let set = c
        .request(Request::PersonalitySet {
            preset: "custom".into(),
            custom: "  Call me captain.  ".into(),
        })
        .await;
    assert!(matches!(
        set,
        Reply::PersonalityShown { ref preset, ref custom, .. }
            if preset == "custom" && custom == "Call me captain."
    ));
    // Kept: asked again, the choice is still there.
    assert!(matches!(
        c.request(Request::PersonalityGet {}).await,
        Reply::PersonalityShown { ref preset, .. } if preset == "custom"
    ));
    // Back to the standard one, so other tests sharing the data folder start from it.
    c.request(Request::PersonalitySet {
        preset: "standard".into(),
        custom: String::new(),
    })
    .await;
}
#[tokio::test]
async fn reopening_replays_only_what_was_missed() {
    let (url, _hub) = engine(vec![reply("First."), reply("Title"), reply("Second.")]).await;
    let mut c = Client::connect(&url, ClientKind::Web).await;
    let session = opened(
        c.request(Request::SessionCreate {
            workspace: Some(workspace("replay")),
            title: None,
            mode: None,
        })
        .await,
    );
    c.request(Request::TurnSend {
        session: session.clone(),
        text: "one".into(),
        files: Vec::new(),
    })
    .await;
    c.events_until(|e| matches!(e, Event::TurnCompleted { .. }))
        .await;

    // A new connection that already has everything up to now sees nothing replayed.
    let mut again = Client::connect(&url, ClientKind::Web).await;
    let last_seq = match again
        .request(Request::SessionList {
            limit: None,
            archived: false,
        })
        .await
    {
        Reply::SessionListed { items } => items[0].last_seq,
        other => panic!("expected session.listed, got {other:?}"),
    };
    assert!(last_seq > 0);
    opened(
        again
            .request(Request::SessionOpen {
                session: session.clone(),
                after_seq: Some(last_seq),
            })
            .await,
    );
    again
        .request(Request::TurnSend {
            session: session.clone(),
            text: "two".into(),
            files: Vec::new(),
        })
        .await;
    let events = again
        .events_until(|e| matches!(e, Event::TurnCompleted { .. }))
        .await;
    let users: Vec<&str> = events
        .iter()
        .filter_map(|e| match e {
            Event::MessageCompleted { text, .. } if text == "one" || text == "two" => {
                Some(text.as_str())
            }
            _ => None,
        })
        .collect();
    assert_eq!(users, vec!["two"], "the first turn must not be replayed");
}

/// A raw HTTP GET (no client library needed for two headers).
async fn http_get(ws_url: &str, path: &str, host: Option<&str>) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let addr = ws_url
        .trim_start_matches("ws://")
        .split('/')
        .next()
        .unwrap()
        .to_owned();
    let mut stream = tokio::net::TcpStream::connect(&addr).await.unwrap();
    let host = host.unwrap_or(&addr);
    let request = format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n");
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut out = Vec::new();
    stream.read_to_end(&mut out).await.unwrap();
    String::from_utf8_lossy(&out).into_owned()
}

#[tokio::test]
async fn the_page_has_safe_headers_and_never_the_token() {
    let (url, _hub) = engine(vec![reply("x")]).await;
    let page = http_get(&url, "/", None).await;
    assert!(page.starts_with("HTTP/1.1 200"), "{page}");
    // Anyone on the computer can load the page, so it must not hand out the owner's token.
    assert!(!page.contains(TOKEN), "the token must not be in the page");
    let lower = page.to_lowercase();
    assert!(lower.contains("content-security-policy"));
    // Connections go back to the engine, and to GitHub's API only for Check for updates.
    assert!(lower.contains("connect-src 'self' https://api.github.com;"));
    assert!(lower.contains("x-frame-options: deny"));
    // Paths that aren't files are the app's one page too.
    assert!(
        http_get(&url, "/some/view", None)
            .await
            .starts_with("HTTP/1.1 200")
    );
}

#[tokio::test]
async fn requests_for_another_host_are_refused() {
    let (url, _hub) = engine(vec![reply("x")]).await;
    let port = url
        .trim_start_matches("ws://127.0.0.1:")
        .split('/')
        .next()
        .unwrap()
        .to_owned();
    let page = http_get(&url, "/", Some(&format!("evil.example:{port}"))).await;
    assert!(page.starts_with("HTTP/1.1 421"), "{page}");
    assert!(!page.contains(TOKEN));
}

#[tokio::test]
async fn a_watcher_sees_every_session_without_live_text() {
    let (url, _hub) = engine(vec![reply("Hello."), reply("Greeting")]).await;
    let mut desktop = Client::connect(&url, ClientKind::Desktop).await;
    assert_eq!(
        desktop.request(Request::SessionWatchAll {}).await,
        Reply::Ack {}
    );
    let mut browser = Client::connect(&url, ClientKind::Web).await;
    let session = opened(
        browser
            .request(Request::SessionCreate {
                workspace: Some(workspace("watch")),
                title: None,
                mode: None,
            })
            .await,
    );
    browser
        .request(Request::TurnSend {
            session,
            text: "hi".into(),
            files: Vec::new(),
        })
        .await;
    let events = desktop
        .events_until(|e| {
            matches!(
                e,
                Event::Status {
                    state: SessionState::Idle,
                    ..
                }
            )
        })
        .await;
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::TurnCompleted { .. }))
    );
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, Event::MessageDelta { .. }))
    );
}

#[tokio::test]
async fn engine_stop_is_acknowledged() {
    let (url, _hub) = engine(vec![reply("x")]).await;
    let mut c = Client::connect(&url, ClientKind::Desktop).await;
    assert_eq!(c.request(Request::EngineStop {}).await, Reply::Ack {});
}

#[tokio::test]
async fn update_now_reaches_the_desktop_app() {
    let (url, _hub) = engine(vec![]).await;
    let mut web = Client::connect(&url, ClientKind::Web).await;
    assert!(
        matches!(
            web.request(Request::UpdateInstall {}).await,
            Reply::Error {
                code: ErrorCode::Invalid,
                ..
            }
        ),
        "no desktop app to install it"
    );
    let mut desktop = Client::connect(&url, ClientKind::Desktop).await;
    assert_eq!(web.request(Request::UpdateInstall {}).await, Reply::Ack {});
    let seen = desktop
        .events_until(|e| matches!(e, Event::UpdateRequested {}))
        .await;
    assert!(matches!(seen.last(), Some(Event::UpdateRequested {})));
}

#[tokio::test]
async fn home_is_built_in_reused_and_kept_out_of_the_list() {
    let (url, hub) = engine(vec![reply("Hello from Home")]).await;
    let mut c = Client::connect(&url, ClientKind::Web).await;
    let Reply::HomeShown { page, earlier } = c.request(Request::HomeGet {}).await else {
        panic!("expected home.shown");
    };
    assert!(page.home && page.title.starts_with("Home · "), "{page:?}");
    assert!(earlier.is_empty());
    let Reply::HomeShown { page: again, .. } = c.request(Request::HomeGet {}).await else {
        panic!("expected home.shown");
    };
    assert_eq!(again.session, page.session, "the same page all day");
    let data = hub.state().data_dir.clone().unwrap();
    assert_eq!(
        again.workspace.as_deref(),
        Some(data.join("home").display().to_string().as_str())
    );

    // It works like any session, but never lists as one.
    opened(
        c.request(Request::SessionOpen {
            session: page.session.clone(),
            after_seq: None,
        })
        .await,
    );
    assert_eq!(
        c.request(Request::TurnSend {
            session: page.session.clone(),
            text: "hi".into(),
            files: Vec::new(),
        })
        .await,
        Reply::Ack {}
    );
    c.events_until(|e| matches!(e, Event::TurnCompleted { .. }))
        .await;
    let Reply::SessionListed { items } = c
        .request(Request::SessionList {
            limit: None,
            archived: false,
        })
        .await
    else {
        panic!("expected session.listed");
    };
    assert!(items.iter().all(|s| s.session != page.session));

    // Built in: no renaming, deleting, pinning.
    for request in [
        Request::SessionRename {
            session: page.session.clone(),
            title: "Mine".into(),
        },
        Request::SessionDelete {
            session: page.session.clone(),
        },
        Request::SessionPin {
            session: page.session.clone(),
            pinned: true,
        },
        Request::SessionSetFolder {
            session: page.session.clone(),
            folder: workspace("home-moved"),
        },
    ] {
        assert!(matches!(
            c.request(request).await,
            Reply::Error {
                code: ErrorCode::Invalid,
                ..
            }
        ));
    }

    // Clearing starts a fresh page.
    let Reply::HomeShown {
        page: fresh,
        earlier,
    } = c.request(Request::HomeClear {}).await
    else {
        panic!("expected home.shown");
    };
    assert_ne!(fresh.session, page.session);
    assert!(earlier.is_empty());
}

#[tokio::test]
async fn sessions_can_be_pinned_and_grouped() {
    let (url, _hub) = engine(vec![reply("x")]).await;
    let mut c = Client::connect(&url, ClientKind::Web).await;
    let session = opened(
        c.request(Request::SessionCreate {
            workspace: Some(workspace("groups")),
            title: None,
            mode: None,
        })
        .await,
    );
    let Reply::GroupListed { items } = c
        .request(Request::GroupCreate {
            name: " Clients ".into(),
        })
        .await
    else {
        panic!("expected group.listed");
    };
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].name, "Clients");
    let group = items[0].group.clone();

    for request in [
        Request::SessionMove {
            session: session.clone(),
            group: Some(group.clone()),
        },
        Request::SessionPin {
            session: session.clone(),
            pinned: true,
        },
    ] {
        assert_eq!(c.request(request).await, Reply::Ack {});
    }
    let Reply::SessionListed { items } = c
        .request(Request::SessionList {
            limit: None,
            archived: false,
        })
        .await
    else {
        panic!("expected session.listed");
    };
    assert_eq!(items[0].group.as_deref(), Some(group.as_str()));
    assert!(items[0].pinned);

    let empty_name = c.request(Request::GroupCreate { name: "  ".into() }).await;
    assert!(matches!(
        empty_name,
        Reply::Error {
            code: ErrorCode::Invalid,
            ..
        }
    ));
}

#[tokio::test]
async fn welcome_says_who_you_are_and_the_edition() {
    let (url, _hub) = engine(vec![reply("x")]).await;
    let (socket, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
    let mut c = Client::wrap(socket);
    let welcome = c
        .request(Request::Hello {
            protocols: vec![1],
            client: ClientInfo {
                kind: ClientKind::Web,
                name: "Test".into(),
                version: "0".into(),
            },
            token: TOKEN.into(),
        })
        .await;
    let Reply::Welcome { engine, user, .. } = welcome else {
        panic!("expected welcome, got {welcome:?}");
    };
    assert_eq!(engine.edition, "home");
    assert_eq!(
        (user.id.as_str(), user.name.as_str(), user.role.as_str()),
        ("u_test", "Test", "owner")
    );
}

#[tokio::test]
async fn a_session_moves_to_another_folder() {
    let (url, _hub) = engine(vec![reply("x")]).await;
    let mut c = Client::connect(&url, ClientKind::Web).await;
    let session = opened(
        c.request(Request::SessionCreate {
            workspace: Some(workspace("move-from")),
            title: None,
            mode: None,
        })
        .await,
    );
    let to = workspace("move-to");
    assert_eq!(
        c.request(Request::SessionSetFolder {
            session: session.clone(),
            folder: to.clone(),
        })
        .await,
        Reply::Ack {}
    );
    let Reply::SessionListed { items } = c
        .request(Request::SessionList {
            limit: None,
            archived: false,
        })
        .await
    else {
        panic!("expected session.listed");
    };
    let moved = items.iter().find(|s| s.session == session).unwrap();
    assert_eq!(moved.workspace.as_deref(), Some(to.as_str()));

    // A folder that isn't there is refused, and the session stays put.
    let missing = c
        .request(Request::SessionSetFolder {
            session: session.clone(),
            folder: nowhere(),
        })
        .await;
    assert!(matches!(
        missing,
        Reply::Error {
            code: ErrorCode::Invalid,
            ..
        }
    ));
}

#[tokio::test]
async fn retry_takes_back_the_last_turn_and_sends_it_again() {
    let (url, _hub) = engine(vec![reply("First."), reply("Title"), reply("Second.")]).await;
    let mut c = Client::connect(&url, ClientKind::Web).await;
    let session = opened(
        c.request(Request::SessionCreate {
            workspace: Some(workspace("retry")),
            title: None,
            mode: None,
        })
        .await,
    );
    // Nothing to retry before the first message.
    let early = c
        .request(Request::TurnRetry {
            session: session.clone(),
        })
        .await;
    assert!(matches!(
        early,
        Reply::Error {
            code: ErrorCode::NotFound,
            ..
        }
    ));

    c.request(Request::TurnSend {
        session: session.clone(),
        text: "one".into(),
        files: Vec::new(),
    })
    .await;
    // Clients offer retry once the session is idle (the turn has fully finished).
    let first = c
        .events_until(|e| {
            matches!(
                e,
                Event::Status {
                    state: SessionState::Idle,
                    ..
                }
            )
        })
        .await;
    let first_turn = first
        .iter()
        .find_map(|e| match e {
            Event::TurnStarted { turn, .. } => Some(turn.clone()),
            _ => None,
        })
        .unwrap();

    assert_eq!(
        c.request(Request::TurnEdit {
            session: session.clone(),
            text: "two".into(),
        })
        .await,
        Reply::Ack {}
    );
    let after = c
        .events_until(|e| matches!(e, Event::TurnCompleted { .. }))
        .await;
    assert!(
        after
            .iter()
            .any(|e| matches!(e, Event::TurnRetracted { turn } if *turn == first_turn))
    );
    assert!(
        after
            .iter()
            .any(|e| matches!(e, Event::MessageCompleted { text, .. } if text == "two"))
    );
}

/// A raw HTTP GET with an optional access token.
async fn http_get_auth(ws_url: &str, path: &str, token: Option<&str>) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let addr = ws_url
        .trim_start_matches("ws://")
        .split('/')
        .next()
        .unwrap()
        .to_owned();
    let mut stream = tokio::net::TcpStream::connect(&addr).await.unwrap();
    let auth = token
        .map(|t| format!("Authorization: Bearer {t}\r\n"))
        .unwrap_or_default();
    let request = format!("GET {path} HTTP/1.1\r\nHost: {addr}\r\n{auth}Connection: close\r\n\r\n");
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut out = Vec::new();
    stream.read_to_end(&mut out).await.unwrap();
    String::from_utf8_lossy(&out).into_owned()
}

/// A raw HTTP POST of `body` to /v1/files.
async fn upload(ws_url: &str, token: &str, name: &str, body: &[u8]) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let addr = ws_url
        .trim_start_matches("ws://")
        .split('/')
        .next()
        .unwrap()
        .to_owned();
    let mut stream = tokio::net::TcpStream::connect(&addr).await.unwrap();
    let head = format!(
        "POST /v1/files HTTP/1.1\r\nHost: {addr}\r\nAuthorization: Bearer {token}\r\n\
         X-File-Name: {name}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).await.unwrap();
    stream.write_all(body).await.unwrap();
    let mut out = Vec::new();
    stream.read_to_end(&mut out).await.unwrap();
    String::from_utf8_lossy(&out).into_owned()
}

#[tokio::test]
async fn an_uploaded_file_is_attached_to_the_message() {
    let (url, _hub) = engine(vec![reply("Got it."), reply("Title")]).await;
    let refused = upload(&url, "wrong", "notes.txt", b"secret").await;
    assert!(refused.starts_with("HTTP/1.1 401"), "{refused}");

    let saved = upload(&url, TOKEN, "my%20notes.txt", b"buy milk").await;
    assert!(saved.starts_with("HTTP/1.1 201"), "{saved}");
    let json: serde_json::Value =
        serde_json::from_str(saved.split("\r\n\r\n").nth(1).unwrap()).unwrap();
    assert_eq!(json["name"], "my notes.txt");
    let file = json["file"].as_str().unwrap().to_owned();

    // It can be fetched back with the token, and not without.
    let back = http_get_auth(&url, &format!("/v1/files/{file}"), Some(TOKEN)).await;
    assert!(back.starts_with("HTTP/1.1 200"), "{back}");
    assert!(back.ends_with("buy milk"));
    let lower = back.to_lowercase();
    assert!(lower.contains("x-content-type-options: nosniff"));
    assert!(
        lower.contains("content-disposition: attachment"),
        "text downloads, never shows as a page"
    );
    let no_token = http_get_auth(&url, &format!("/v1/files/{file}"), None).await;
    assert!(no_token.starts_with("HTTP/1.1 401"), "{no_token}");
    let missing = http_get_auth(&url, "/v1/files/f_nothere", Some(TOKEN)).await;
    assert!(missing.starts_with("HTTP/1.1 404"), "{missing}");

    let mut c = Client::connect(&url, ClientKind::Web).await;
    let session = opened(
        c.request(Request::SessionCreate {
            workspace: Some(workspace("upload")),
            title: None,
            mode: None,
        })
        .await,
    );
    // An id that isn't an upload is refused.
    let bad = c
        .request(Request::TurnSend {
            session: session.clone(),
            text: "look".into(),
            files: vec!["f_../../escape".into()],
        })
        .await;
    assert!(matches!(
        bad,
        Reply::Error {
            code: ErrorCode::NotFound,
            ..
        }
    ));

    c.request(Request::TurnSend {
        session: session.clone(),
        text: "read this".into(),
        files: vec![file],
    })
    .await;
    let attached = |e: &Event| {
        matches!(e, Event::MessageCompleted { text, files, .. }
            if text == "read this" && files.len() == 1 && files[0].name == "my notes.txt" && files[0].kind == "text")
    };
    let events = c
        .events_until(|e| {
            matches!(
                e,
                Event::Status {
                    state: SessionState::Idle,
                    ..
                }
            )
        })
        .await;
    assert!(events.iter().any(attached));

    // A retry attaches the same file again (named once, not twice).
    c.request(Request::TurnRetry { session }).await;
    let again = c
        .events_until(|e| matches!(e, Event::TurnCompleted { .. }))
        .await;
    assert!(again.iter().any(attached), "{again:?}");
}

#[tokio::test]
async fn a_sessions_folder_can_be_browsed_and_read_but_not_left() {
    let (url, _hub) = engine(vec![reply("ok")]).await;
    let mut c = Client::connect(&url, ClientKind::Android).await;
    let ws = workspace("browse");
    std::fs::create_dir_all(std::path::Path::new(&ws).join("out")).unwrap();
    std::fs::write(std::path::Path::new(&ws).join("out/report.md"), "# Report").unwrap();
    let session = opened(
        c.request(Request::SessionCreate {
            workspace: Some(ws.clone()),
            title: None,
            mode: None,
        })
        .await,
    );
    let Reply::FolderListed { items, path, .. } = c
        .request(Request::FolderList {
            session: session.clone(),
            path: String::new(),
        })
        .await
    else {
        panic!("no listing");
    };
    assert!(
        items.iter().any(|e| e.name == "out" && e.folder),
        "{path} {items:?}"
    );
    let Reply::FolderPart {
        data, size, path, ..
    } = c
        .request(Request::FolderRead {
            session: session.clone(),
            path: "out/report.md".into(),
            offset: 0,
        })
        .await
    else {
        panic!("no file");
    };
    assert_eq!((path.as_str(), size), ("out/report.md", 8));
    use base64::Engine as _;
    assert_eq!(
        base64::engine::general_purpose::STANDARD
            .decode(data)
            .unwrap(),
        b"# Report"
    );
    let outside = c
        .request(Request::FolderRead {
            session,
            path: "../../Windows/win.ini".into(),
            offset: 0,
        })
        .await;
    assert!(
        matches!(
            outside,
            Reply::Error {
                code: ErrorCode::NotFound,
                ..
            }
        ),
        "{outside:?}"
    );
}
#[tokio::test]
async fn a_session_can_move_folder_and_one_without_a_folder_still_browses() {
    let (url, _hub) = engine(vec![reply("ok")]).await;
    let mut c = Client::connect(&url, ClientKind::Android).await;
    // No folder picked: browsing shows the folder the engine works in, not an error.
    let bare = opened(
        c.request(Request::SessionCreate {
            workspace: None,
            title: None,
            mode: None,
        })
        .await,
    );
    let listed = c
        .request(Request::FolderList {
            session: bare.clone(),
            path: String::new(),
        })
        .await;
    assert!(matches!(listed, Reply::FolderListed { .. }), "{listed:?}");
    // A folder that isn't there can't be picked.
    let missing = c
        .request(Request::SessionCreate {
            workspace: Some(nowhere()),
            title: None,
            mode: None,
        })
        .await;
    assert!(
        matches!(
            missing,
            Reply::Error {
                code: ErrorCode::Invalid,
                ..
            }
        ),
        "{missing:?}"
    );
    // The folders offered include ones sessions use.
    let ws = workspace("move");
    std::fs::write(std::path::Path::new(&ws).join("moved.txt"), "here").unwrap();
    let Reply::FolderPlaces { recent, .. } = c.request(Request::FolderPlaces {}).await else {
        panic!("no places");
    };
    assert!(
        !recent.contains(&ws),
        "not used by a session yet: {recent:?}"
    );
    // /folder moves the session; every client hears about it; browsing follows.
    let Reply::CommandResult { markdown } = c
        .request(Request::CommandRun {
            session: Some(bare.clone()),
            name: "folder".into(),
            args: ws.clone(),
        })
        .await
    else {
        panic!("no command result");
    };
    assert!(markdown.contains("now works in"), "{markdown}");
    let events = c
        .events_until(|e| {
            matches!(
                e,
                Event::SessionUpdated {
                    workspace: Some(_),
                    ..
                }
            )
        })
        .await;
    assert!(
        events.iter().any(|e| matches!(
            e,
            Event::SessionUpdated { workspace: Some(w), .. } if *w == ws
        )),
        "{events:?}"
    );
    let Reply::FolderListed { items, .. } = c
        .request(Request::FolderList {
            session: bare.clone(),
            path: String::new(),
        })
        .await
    else {
        panic!("no listing");
    };
    assert!(items.iter().any(|e| e.name == "moved.txt"), "{items:?}");
    let Reply::FolderPlaces { recent, .. } = c.request(Request::FolderPlaces {}).await else {
        panic!("no places");
    };
    assert_eq!(recent.first(), Some(&ws));
    // A new folder can be made in an offered folder, once, with a plain name only.
    let made = c
        .request(Request::FolderCreate {
            parent: ws.clone(),
            name: " Woodland book ".into(),
        })
        .await;
    let Reply::FolderCreated { path } = made else {
        panic!("not made: {made:?}");
    };
    assert!(std::path::Path::new(&path).is_dir());
    assert!(path.ends_with("Woodland book"), "{path}");
    for (parent, name) in [
        (ws.clone(), "Woodland book"),
        (ws.clone(), "../escape"),
        (ws.clone(), "CON"),
        (nowhere(), "fresh"),
    ] {
        let refused = c
            .request(Request::FolderCreate {
                parent,
                name: name.into(),
            })
            .await;
        assert!(
            matches!(
                refused,
                Reply::Error {
                    code: ErrorCode::Invalid,
                    ..
                }
            ),
            "{name}: {refused:?}"
        );
    }
    assert!(!std::path::Path::new(&ws).join("../escape").exists());
    // A folder that isn't there leaves the session where it was.
    let Reply::CommandResult { markdown } = c
        .request(Request::CommandRun {
            session: Some(bare),
            name: "folder".into(),
            args: nowhere(),
        })
        .await
    else {
        panic!("no command result");
    };
    assert!(markdown.contains("There's no folder"), "{markdown}");
}

/// A raw HTTP POST of a JSON body to `path`.
async fn http_post_json(ws_url: &str, path: &str, json: &str) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let addr = ws_url
        .trim_start_matches("ws://")
        .split('/')
        .next()
        .unwrap()
        .to_owned();
    let mut stream = tokio::net::TcpStream::connect(&addr).await.unwrap();
    let head = format!(
        "POST {path} HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n",
        json.len()
    );
    stream.write_all(head.as_bytes()).await.unwrap();
    stream.write_all(json.as_bytes()).await.unwrap();
    let mut out = Vec::new();
    stream.read_to_end(&mut out).await.unwrap();
    String::from_utf8_lossy(&out).into_owned()
}

/// Say hello with `token`; the socket and what came back.
async fn hello_with(url: &str, token: &str) -> (Client, Reply) {
    let (socket, _) = tokio_tungstenite::connect_async(url).await.unwrap();
    let mut client = Client::wrap(socket);
    let answer = client
        .request(Request::Hello {
            protocols: vec![1],
            client: ClientInfo {
                kind: ClientKind::Web,
                name: "Test".into(),
                version: "0".into(),
            },
            token: token.into(),
        })
        .await;
    (client, answer)
}

/// A sign-in code for `person`, traded for a device token the way a browser does.
async fn sign_in(url: &str, owner: &mut Client, person: &str) -> String {
    let Reply::SigninCode { code, .. } = owner
        .request(Request::SigninCreate {
            person: person.into(),
        })
        .await
    else {
        panic!("no sign-in code");
    };
    let body = format!(r#"{{"code":"{code}","name":"Edge on Windows"}}"#);
    let answer = http_post_json(url, "/v1/signin", &body).await;
    assert!(answer.starts_with("HTTP/1.1 200"), "{answer}");
    let json: serde_json::Value =
        serde_json::from_str(answer.split("\r\n\r\n").nth(1).unwrap()).unwrap();
    // A code works once.
    let again = http_post_json(url, "/v1/signin", &body).await;
    assert!(again.starts_with("HTTP/1.1 401"), "{again}");
    json["token"].as_str().unwrap().to_owned()
}

fn refused_as(reply: &Reply, code: ErrorCode) -> bool {
    matches!(reply, Reply::Error { code: c, .. } if *c == code)
}

/// Waits until the engine closes the socket (events and replies before that are skipped).
async fn closed(client: &mut Client) {
    loop {
        match tokio::time::timeout(Duration::from_secs(10), client.socket.next())
            .await
            .expect("still open after 10 s")
        {
            Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return,
            Some(Ok(_)) => {}
        }
    }
}

#[tokio::test]
async fn a_sign_in_link_signs_a_browser_in_as_its_person() {
    let (url, _hub) = engine(vec![reply("Hi.")]).await;
    let mut owner = Client::connect(&url, ClientKind::Web).await;
    let Reply::PeopleListed { items } = owner
        .request(Request::PeopleAdd { name: "Sis".into() })
        .await
    else {
        panic!("couldn't add a person");
    };
    let sis = items[1].person.clone();

    // Nobody gets a link for someone who isn't here; a wrong code signs nobody in.
    assert!(refused_as(
        &owner
            .request(Request::SigninCreate {
                person: "pe_nobody".into()
            })
            .await,
        ErrorCode::NotFound
    ));
    let wrong = http_post_json(&url, "/v1/signin", r#"{"code":"00112233"}"#).await;
    assert!(wrong.starts_with("HTTP/1.1 401"), "{wrong}");

    // The browser that opens the link is Sis from then on.
    let token = sign_in(&url, &mut owner, &sis).await;
    let (mut browser, welcome) = hello_with(&url, &token).await;
    let Reply::Welcome { user, .. } = welcome else {
        panic!("Sis's browser was refused: {welcome:?}");
    };
    assert_eq!(
        (user.id.as_str(), user.role.as_str()),
        (sis.as_str(), "member")
    );

    // Sis sees her own browser (marked as this one) but nobody else's, and can't make links.
    let Reply::DeviceListed { items, .. } =
        browser.request(Request::DeviceList { person: None }).await
    else {
        panic!("no device list");
    };
    assert_eq!(items.len(), 1);
    assert!(items[0].current);
    assert_eq!(items[0].name, "Edge on Windows");
    let device = items[0].device.clone();
    assert!(refused_as(
        &browser
            .request(Request::DeviceList {
                person: Some("u_test".into())
            })
            .await,
        ErrorCode::Unauthorized
    ));
    assert!(refused_as(
        &browser
            .request(Request::SigninCreate {
                person: sis.clone()
            })
            .await,
        ErrorCode::Unauthorized
    ));

    // Her files are hers: uploaded with her token, the owner's token doesn't find them.
    let saved = upload(&url, &token, "sis.txt", b"hers").await;
    assert!(saved.starts_with("HTTP/1.1 201"), "{saved}");
    let json: serde_json::Value =
        serde_json::from_str(saved.split("\r\n\r\n").nth(1).unwrap()).unwrap();
    let file = json["file"].as_str().unwrap().to_owned();
    let hers = http_get_auth(&url, &format!("/v1/files/{file}"), Some(&token)).await;
    assert!(hers.starts_with("HTTP/1.1 200"), "{hers}");
    let owners = http_get_auth(&url, &format!("/v1/files/{file}"), Some(TOKEN)).await;
    assert!(owners.starts_with("HTTP/1.1 404"), "{owners}");

    // The owner signs that browser out: its connection closes and the token stops working.
    let Reply::DeviceListed { items, .. } = owner
        .request(Request::DeviceRemove {
            device: device.clone(),
            person: Some(sis.clone()),
        })
        .await
    else {
        panic!("couldn't sign the browser out");
    };
    assert!(items.is_empty());
    closed(&mut browser).await;
    let (_, again) = hello_with(&url, &token).await;
    assert!(refused_as(&again, ErrorCode::Unauthorized), "{again:?}");

    // Removing a person signs all their browsers out.
    let token = sign_in(&url, &mut owner, &sis).await;
    let (mut browser, welcome) = hello_with(&url, &token).await;
    assert!(matches!(welcome, Reply::Welcome { .. }));
    assert!(matches!(
        owner
            .request(Request::PeopleRemove {
                person: sis.clone()
            })
            .await,
        Reply::PeopleListed { .. }
    ));
    closed(&mut browser).await;
    let (_, again) = hello_with(&url, &token).await;
    assert!(refused_as(&again, ErrorCode::Unauthorized), "{again:?}");
}

fn set_mode(session: &str, mode: &str, until: Option<&str>, minutes: Option<u32>) -> Request {
    Request::SessionSetMode {
        session: session.into(),
        mode: mode.into(),
        until: until.map(Into::into),
        minutes,
    }
}

/// Each session's mode, as the list shows it.
async fn modes(c: &mut Client) -> Vec<(String, String)> {
    let Reply::SessionListed { items } = c
        .request(Request::SessionList {
            limit: None,
            archived: false,
        })
        .await
    else {
        panic!("no session list");
    };
    items.into_iter().map(|s| (s.session, s.mode)).collect()
}

#[tokio::test]
async fn full_auto_is_the_owners_only() {
    let (url, _hub) = engine(vec![reply("Hi.")]).await;
    let mut owner = Client::connect(&url, ClientKind::Web).await;
    let Reply::PeopleListed { items } = owner
        .request(Request::PeopleAdd { name: "Sis".into() })
        .await
    else {
        panic!("couldn't add a person");
    };
    let sis = items[1].person.clone();
    let token = sign_in(&url, &mut owner, &sis).await;
    let (mut browser, _) = hello_with(&url, &token).await;

    // Hiding it in a menu isn't enough: the engine refuses it, both ways of asking.
    let hers = opened(
        browser
            .request(Request::SessionCreate {
                workspace: Some(workspace("full-auto-sis")),
                title: None,
                mode: None,
            })
            .await,
    );
    let refused = browser
        .request(set_mode(&hers, "full_auto", None, None))
        .await;
    assert!(matches!(refused, Reply::Error { .. }), "{refused:?}");
    let refused = browser
        .request(Request::SessionCreate {
            workspace: Some(workspace("full-auto-sis2")),
            title: None,
            mode: Some("full_auto".into()),
        })
        .await;
    assert!(matches!(refused, Reply::Error { .. }), "{refused:?}");
    assert!(modes(&mut browser).await.iter().all(|(_, m)| m == "ask"));
    // Her other modes still work.
    assert_eq!(
        browser
            .request(set_mode(&hers, "auto_edits", None, None))
            .await,
        Reply::Ack {}
    );

    // The owner may.
    let mine = opened(
        owner
            .request(Request::SessionCreate {
                workspace: Some(workspace("full-auto-owner")),
                title: None,
                mode: None,
            })
            .await,
    );
    assert_eq!(
        owner
            .request(set_mode(&mine, "full_auto", None, None))
            .await,
        Reply::Ack {}
    );
}

#[tokio::test]
async fn full_auto_ends_when_the_task_ends() {
    let (url, _hub) = engine(vec![reply("Done."), reply("Title")]).await;
    let mut c = Client::connect(&url, ClientKind::Web).await;
    let session = opened(
        c.request(Request::SessionCreate {
            workspace: Some(workspace("full-auto-task")),
            title: None,
            mode: None,
        })
        .await,
    );
    assert_eq!(
        c.request(set_mode(&session, "full_auto", None, None)).await,
        Reply::Ack {}
    );
    assert_eq!(modes(&mut c).await[0].1, "full_auto");
    c.request(Request::TurnSend {
        session: session.clone(),
        text: "go".into(),
        files: Vec::new(),
    })
    .await;
    let events = c
        .events_until(|e| matches!(e, Event::Notice { text, .. } if text.contains("task is done")))
        .await;
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::SessionUpdated { mode: Some(m), .. } if m == "ask")),
        "{events:?}"
    );
    assert_eq!(modes(&mut c).await[0].1, "ask");
}

#[tokio::test]
async fn full_auto_until_switched_off_outlasts_the_task_and_one_chat_has_it() {
    let (url, _hub) = engine(vec![reply("Done."), reply("Title")]).await;
    let mut c = Client::connect(&url, ClientKind::Web).await;
    let chat = || Request::SessionCreate {
        workspace: Some(workspace("full-auto-two")),
        title: None,
        mode: None,
    };
    let first = opened(c.request(chat()).await);
    let second = opened(c.request(chat()).await);
    assert_eq!(
        c.request(set_mode(&first, "full_auto", Some("manual"), None))
            .await,
        Reply::Ack {}
    );
    c.request(Request::TurnSend {
        session: first.clone(),
        text: "go".into(),
        files: Vec::new(),
    })
    .await;
    c.events_until(|e| {
        matches!(
            e,
            Event::Status {
                state: SessionState::Idle,
                ..
            }
        )
    })
    .await;
    let mode_of =
        |all: &[(String, String)], id: &str| all.iter().find(|(s, _)| s == id).unwrap().1.clone();
    assert_eq!(mode_of(&modes(&mut c).await, &first), "full_auto");

    // Switching it on in the second chat switches the first back to Ask.
    assert_eq!(
        c.request(set_mode(&second, "full_auto", Some("time"), Some(30)))
            .await,
        Reply::Ack {}
    );
    let all = modes(&mut c).await;
    assert_eq!(mode_of(&all, &first), "ask");
    assert_eq!(mode_of(&all, &second), "full_auto");

    // Nonsense limits are refused.
    for (until, minutes) in [
        (Some("time"), None),
        (Some("time"), Some(0)),
        (Some("soon"), None),
    ] {
        let r = c
            .request(set_mode(&second, "full_auto", until, minutes))
            .await;
        assert!(
            matches!(r, Reply::Error { .. }),
            "{until:?} {minutes:?}: {r:?}"
        );
    }
}
