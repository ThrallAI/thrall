//! The phone link end to end: a pretend phone pairs from the QR link, connects over Noise and
//! speaks the ordinary protocol; wrong secrets, unknown keys, removed phones and browsers are
//! turned away.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use futures::{SinkExt, StreamExt, stream};
use snow::{Builder, Keypair, TransportState};
use thrall_agent::{AppState, Hub};
use thrall_core::Config;
use thrall_llm::{ChatRequest, LlmProvider, ReplyStream};
use thrall_protocol::{
    ClientInfo, ClientKind, ErrorCode, Reply, Request, RequestFrame, ServerFrame,
};
use thrall_server::remote::{open, seal};
use thrall_server::{Options, start};
use thrall_store::{SqliteStore, Store};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

const TOKEN: &str = "test-token";
const PAIR: &str = "Noise_IKpsk1_25519_ChaChaPoly_BLAKE2s";
const CONNECT: &str = "Noise_IK_25519_ChaChaPoly_BLAKE2s";
const PROLOGUE: &[u8] = b"thrall-remote-v1";
const ASK: &str = "Noise_XX_25519_ChaChaPoly_BLAKE2s";

struct Silent;

impl LlmProvider for Silent {
    fn name(&self) -> &str {
        "Silent"
    }

    fn stream(&self, _request: ChatRequest) -> ReplyStream {
        stream::iter(Vec::new()).boxed()
    }
}

/// People's own keys, in memory.
#[derive(Default)]
struct MemoryKeys(std::sync::Mutex<HashMap<String, String>>);

impl thrall_agent::people::OwnKeys for MemoryKeys {
    fn has(&self, person: &str) -> bool {
        self.0.lock().unwrap().contains_key(person)
    }
    fn provider(&self, person: &str) -> Option<Arc<dyn LlmProvider>> {
        self.has(person)
            .then(|| Arc::new(Silent) as Arc<dyn LlmProvider>)
    }
    fn set(&self, person: &str, key: &str) -> Result<(), String> {
        self.0
            .lock()
            .unwrap()
            .insert(person.to_owned(), key.to_owned());
        Ok(())
    }
    fn clear(&self, person: &str) -> Result<(), String> {
        self.0.lock().unwrap().remove(person);
        Ok(())
    }
}

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// An engine with remote access on; returns the local socket URL.
async fn engine(name: &str) -> String {
    engine_with(name, None).await
}

/// An engine with remote access on, and optionally a member ("u_sam") whose phone, with
/// `member_key`, is already paired. Returns the local socket URL.
async fn engine_with(name: &str, member_key: Option<&[u8]>) -> String {
    format!("ws://{}/v1/ws", engine_running(name, member_key).await.addr)
}

/// [`engine_with`], with the running server (its hub and shared state).
async fn engine_running(name: &str, member_key: Option<&[u8]>) -> thrall_server::Running {
    let store = SqliteStore::open_in_memory().await.unwrap();
    store.ensure_user("u_test", "Test", 0).await.unwrap();
    store.set_role("u_test", "owner").await.unwrap();
    store.ensure_user("u_sam", "Sam", 1).await.unwrap();
    let mut config = Config::default();
    config.remote.enabled = true;
    config.remote.address = "https://pc.example.ts.net".into();
    config.remote.pcs = true;
    let mut state = AppState::new(config, Arc::new(store), Arc::new(Silent), "u_test".into())
        .unwrap()
        .with_own_keys(Arc::new(MemoryKeys::default()));
    let data = std::env::temp_dir().join(format!("thrall-remote-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&data);
    std::fs::create_dir_all(&data).unwrap();
    if let Some(key) = member_key {
        let hex: String = key.iter().map(|b| format!("{b:02x}")).collect();
        std::fs::write(
            data.join("phones.json"),
            format!(
                r#"[{{"id":"p_sam","name":"Sam's phone","key":"{hex}","added":1,"person":"u_sam"}}]"#
            ),
        )
        .unwrap();
    }
    state.data_dir = Some(data);
    let hub = Hub::new(state, None);
    start(
        hub,
        Options {
            port: 0,
            token: TOKEN.into(),
            version: "0.0.0-test".into(),
            extra_origins: Vec::new(),
            serve_web: false,
            remote_port: Some(0),
        },
    )
    .await
    .unwrap()
}

/// Write a request; read frames until its reply.
async fn ask(socket: &mut Socket, id: &str, request: Request) -> Reply {
    let text = serde_json::to_string(&RequestFrame::new(id, request)).unwrap();
    socket.send(Message::Text(text.into())).await.unwrap();
    loop {
        let msg = tokio::time::timeout(Duration::from_secs(10), socket.next())
            .await
            .expect("no reply within 10 s")
            .unwrap()
            .unwrap();
        if let Message::Text(text) = msg
            && let ServerFrame::Reply(r) = serde_json::from_str(text.as_str()).unwrap()
            && r.re == id
        {
            return r.reply;
        }
    }
}

fn hello(token: &str) -> Request {
    Request::Hello {
        protocols: vec![1],
        client: ClientInfo {
            kind: ClientKind::Android,
            name: "Thrall for Android".into(),
            version: "0".into(),
        },
        token: token.into(),
    }
}

/// The desktop side: a local client that can make pairing codes.
async fn desktop(url: &str) -> Socket {
    let (mut socket, _) = tokio_tungstenite::connect_async(url).await.unwrap();
    assert!(matches!(
        ask(&mut socket, "h", hello(TOKEN)).await,
        Reply::Welcome { .. }
    ));
    socket
}

/// The phone link's port, once it's listening.
async fn phone_port(desk: &mut Socket) -> u16 {
    for i in 0..50 {
        if let Reply::PhoneListed { remote, .. } =
            ask(desk, &format!("l{i}"), Request::PhoneList {}).await
            && remote.listening
        {
            return remote.port;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("the phone link never started");
}

struct Code {
    key: Vec<u8>,
    id: Vec<u8>,
    secret: [u8; 32],
}

async fn new_code(desk: &mut Socket, id: &str) -> Code {
    new_code_for(desk, id, None).await
}

/// A pairing code for `person`'s phone.
async fn new_code_for(desk: &mut Socket, id: &str, person: Option<&str>) -> Code {
    let request = Request::PhonePair {
        person: person.map(str::to_owned),
    };
    let Reply::PhonePairing { link, qr, expires } = ask(desk, id, request).await else {
        panic!("no pairing code");
    };
    assert!(
        !qr.is_empty() && qr.iter().all(|r| r.len() == qr.len()),
        "square QR"
    );
    assert!(expires > 0);
    let fields: HashMap<String, Vec<u8>> = link
        .strip_prefix("thrall://pair?")
        .unwrap()
        .split('&')
        .map(|kv| {
            let (k, v) = kv.split_once('=').unwrap();
            (k.to_owned(), URL_SAFE_NO_PAD.decode(v).unwrap())
        })
        .collect();
    assert_eq!(fields["a"], b"https://pc.example.ts.net");
    Code {
        key: fields["k"].clone(),
        id: fields["i"].clone(),
        secret: fields["s"].clone().try_into().unwrap(),
    }
}

fn phone_keys() -> Keypair {
    Builder::new(CONNECT.parse().unwrap())
        .generate_keypair()
        .unwrap()
}

/// A phone's encrypted connection.
struct Phone {
    socket: Socket,
    transport: TransportState,
    partial: Vec<u8>,
    /// What the engine said in its handshake answer.
    answer: Vec<u8>,
}

impl Phone {
    /// Run a handshake; `None` when the engine turns it away.
    async fn handshake(
        port: u16,
        keys: &Keypair,
        engine_key: &[u8],
        pairing: Option<(&[u8], &[u8; 32])>,
        name: &str,
    ) -> Option<Self> {
        let url = format!("ws://127.0.0.1:{port}/v1/remote");
        let (mut socket, _) = tokio_tungstenite::connect_async(url).await.unwrap();
        let builder = Builder::new(
            if pairing.is_some() { PAIR } else { CONNECT }
                .parse()
                .unwrap(),
        )
        .local_private_key(&keys.private)
        .unwrap()
        .remote_public_key(engine_key)
        .unwrap()
        .prologue(PROLOGUE)
        .unwrap();
        let builder = match pairing {
            Some((_, secret)) => builder.psk(1, secret).unwrap(),
            None => builder,
        };
        let mut hs = builder.build_initiator().unwrap();
        let mut message = vec![0u8; 65_535];
        let payload = serde_json::json!({ "name": name }).to_string();
        let n = hs.write_message(payload.as_bytes(), &mut message).unwrap();
        let mut first = match pairing {
            Some((id, _)) => [&[1u8][..], id].concat(),
            None => vec![2u8],
        };
        first.extend_from_slice(&message[..n]);
        socket.send(Message::Binary(first.into())).await.unwrap();
        let answer = match tokio::time::timeout(Duration::from_secs(10), socket.next()).await {
            Ok(Some(Ok(Message::Binary(bytes)))) => bytes,
            _ => return None,
        };
        let mut payload = vec![0u8; 65_535];
        let n = hs.read_message(&answer, &mut payload).unwrap();
        Some(Self {
            socket,
            transport: hs.into_transport_mode().unwrap(),
            partial: Vec::new(),
            answer: payload[..n].to_vec(),
        })
    }

    async fn send(&mut self, id: &str, request: Request) {
        let text = serde_json::to_string(&RequestFrame::new(id, request)).unwrap();
        for piece in seal(&mut self.transport, text.as_bytes()).unwrap() {
            self.socket
                .send(Message::Binary(piece.into()))
                .await
                .unwrap();
        }
    }

    /// The next whole frame; `None` once the engine closes the link.
    async fn frame(&mut self) -> Option<ServerFrame> {
        loop {
            let msg = tokio::time::timeout(Duration::from_secs(10), self.socket.next())
                .await
                .expect("nothing within 10 s");
            match msg {
                Some(Ok(Message::Binary(bytes))) => {
                    if let Some(whole) =
                        open(&mut self.transport, &bytes, &mut self.partial).unwrap()
                    {
                        return Some(serde_json::from_slice(&whole).unwrap());
                    }
                }
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return None,
                Some(Ok(_)) => {}
            }
        }
    }

    async fn ask(&mut self, id: &str, request: Request) -> Reply {
        self.send(id, request).await;
        loop {
            if let ServerFrame::Reply(r) = self.frame().await.expect("link closed")
                && r.re == id
            {
                return r.reply;
            }
        }
    }
}

#[tokio::test]
async fn a_phone_pairs_connects_and_moves_files() {
    let url = engine("pair").await;
    let mut desk = desktop(&url).await;
    let port = phone_port(&mut desk).await;
    let code = new_code(&mut desk, "p1").await;
    let keys = phone_keys();

    // Pairing: the engine learns the phone's key and answers with its id.
    let mut phone = Phone::handshake(
        port,
        &keys,
        &code.key,
        Some((&code.id, &code.secret)),
        "Nothing Phone",
    )
    .await
    .expect("pairing refused");
    let answer: serde_json::Value = serde_json::from_slice(&phone.answer).unwrap();
    let phone_id = answer["phone"].as_str().unwrap().to_owned();
    // No access token needed: the key already proved who this is.
    assert!(matches!(
        phone.ask("h", hello("")).await,
        Reply::Welcome { .. }
    ));
    assert!(matches!(
        phone
            .ask(
                "s",
                Request::SessionList {
                    limit: None,
                    archived: false
                }
            )
            .await,
        Reply::SessionListed { .. }
    ));
    // Phones can't add or remove phones.
    let Reply::Error { code: c, .. } = phone.ask("pp", Request::PhonePair { person: None }).await
    else {
        panic!("a phone made a pairing code");
    };
    assert_eq!(c, ErrorCode::Unauthorized);

    // A file bigger than one piece goes up in pieces and comes back the same.
    let file: Vec<u8> = (0..700_000u32).map(|i| (i % 253) as u8).collect();
    let mut upload = None;
    let mut size = 0;
    for (i, chunk) in file.chunks(512 * 1024).enumerate() {
        let reply = phone
            .ask(
                &format!("f{i}"),
                Request::FilePut {
                    upload: upload.clone(),
                    name: "../photo.jpg".into(),
                    data: STANDARD.encode(chunk),
                },
            )
            .await;
        let Reply::FileSaved {
            file,
            name,
            size: s,
        } = reply
        else {
            panic!("{reply:?}");
        };
        assert_eq!(name, "photo.jpg", "the name is cleaned");
        upload = Some(file);
        size = s;
    }
    assert_eq!(size, file.len() as u64);
    let id = upload.unwrap();
    let mut back = Vec::new();
    while back.len() < file.len() {
        let reply = phone
            .ask(
                &format!("g{}", back.len()),
                Request::FileGet {
                    file: id.clone(),
                    offset: back.len() as u64,
                },
            )
            .await;
        let Reply::FilePart { data, size, .. } = reply else {
            panic!("{reply:?}");
        };
        assert_eq!(size, file.len() as u64);
        back.extend(STANDARD.decode(data).unwrap());
    }
    assert_eq!(back, file);
    // Another connection can't add to that upload.
    let Reply::Error { code: c, .. } = ask(
        &mut desk,
        "fx",
        Request::FilePut {
            upload: Some(id),
            name: String::new(),
            data: STANDARD.encode(b"x"),
        },
    )
    .await
    else {
        panic!("added to someone else's upload");
    };
    assert_eq!(c, ErrorCode::NotFound);

    // The desktop sees the phone, connected.
    let Reply::PhoneListed { items, .. } = ask(&mut desk, "l", Request::PhoneList {}).await else {
        panic!();
    };
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].name, "Nothing Phone");
    assert!(items[0].connected);

    // The code was used up: pairing again with it fails.
    let other = phone_keys();
    assert!(
        Phone::handshake(port, &other, &code.key, Some((&code.id, &code.secret)), "x")
            .await
            .is_none()
    );

    // A second connection from the paired phone gets in without a code.
    let mut again = Phone::handshake(port, &keys, &code.key, None, "")
        .await
        .expect("paired phone refused");
    assert!(matches!(
        again.ask("h", hello("")).await,
        Reply::Welcome { .. }
    ));

    // Removing the phone closes its connections and keeps it out.
    assert!(matches!(
        ask(&mut desk, "rm", Request::PhoneRemove { phone: phone_id }).await,
        Reply::PhoneListed { .. }
    ));
    assert!(
        phone.frame().await.is_none(),
        "still connected after removal"
    );
    assert!(
        again.frame().await.is_none(),
        "still connected after removal"
    );
    assert!(
        Phone::handshake(port, &keys, &code.key, None, "")
            .await
            .is_none()
    );
}

#[tokio::test]
async fn wrong_secrets_strangers_and_browsers_are_turned_away() {
    let url = engine("refuse").await;
    let mut desk = desktop(&url).await;
    let port = phone_port(&mut desk).await;
    let code = new_code(&mut desk, "p1").await;
    let keys = phone_keys();

    // A key the engine has never paired.
    assert!(
        Phone::handshake(port, &keys, &code.key, None, "")
            .await
            .is_none()
    );

    // Five wrong secrets use the code up, so even the right one then fails.
    let wrong = [9u8; 32];
    for _ in 0..5 {
        assert!(
            Phone::handshake(port, &keys, &code.key, Some((&code.id, &wrong)), "")
                .await
                .is_none()
        );
    }
    assert!(
        Phone::handshake(port, &keys, &code.key, Some((&code.id, &code.secret)), "")
            .await
            .is_none(),
        "the code still worked after five wrong tries"
    );

    // A fresh code works.
    let fresh = new_code(&mut desk, "p2").await;
    assert!(
        Phone::handshake(
            port,
            &keys,
            &fresh.key,
            Some((&fresh.id, &fresh.secret)),
            ""
        )
        .await
        .is_some()
    );

    // A browser page can't open the phone link.
    let mut request = format!("ws://127.0.0.1:{port}/v1/remote")
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("Origin", "https://evil.example".parse().unwrap());
    assert!(tokio_tungstenite::connect_async(request).await.is_err());

    // Nothing on the phone link but the one path.
    let stray = format!("ws://127.0.0.1:{port}/v1/ws");
    assert!(tokio_tungstenite::connect_async(stray).await.is_err());
}

#[tokio::test]
async fn a_flood_of_strangers_pauses_pairing_but_not_paired_phones() {
    let url = engine("flood").await;
    let mut desk = desktop(&url).await;
    let port = phone_port(&mut desk).await;
    let code = new_code(&mut desk, "p1").await;
    let keys = phone_keys();
    assert!(
        Phone::handshake(port, &keys, &code.key, Some((&code.id, &code.secret)), "")
            .await
            .is_some()
    );
    for _ in 0..20 {
        let stranger = phone_keys();
        assert!(
            Phone::handshake(port, &stranger, &code.key, None, "")
                .await
                .is_none()
        );
    }
    let fresh = new_code(&mut desk, "p2").await;
    let other = phone_keys();
    assert!(
        Phone::handshake(
            port,
            &other,
            &fresh.key,
            Some((&fresh.id, &fresh.secret)),
            ""
        )
        .await
        .is_none(),
        "pairing should pause after a flood"
    );
    let mut phone = Phone::handshake(port, &keys, &code.key, None, "")
        .await
        .expect("the paired phone was locked out");
    assert!(matches!(
        phone.ask("h", hello("")).await,
        Reply::Welcome { .. }
    ));
}

#[tokio::test]
async fn pairing_needs_remote_access_and_an_address() {
    let store = SqliteStore::open_in_memory().await.unwrap();
    store.ensure_user("u_test", "Test", 0).await.unwrap();
    store.set_role("u_test", "owner").await.unwrap();
    let state = AppState::new(
        Config::default(),
        Arc::new(store),
        Arc::new(Silent),
        "u_test".into(),
    )
    .unwrap();
    let running = start(
        Hub::new(state, None),
        Options {
            port: 0,
            token: TOKEN.into(),
            version: "0".into(),
            extra_origins: Vec::new(),
            serve_web: false,
            remote_port: Some(0),
        },
    )
    .await
    .unwrap();
    let mut desk = desktop(&format!("ws://{}/v1/ws", running.addr)).await;
    let Reply::Error { message, .. } =
        ask(&mut desk, "p", Request::PhonePair { person: None }).await
    else {
        panic!("paired with remote access off");
    };
    assert!(message.contains("Turn on remote access"), "{message}");
    let Reply::PhoneListed { remote, items } = ask(&mut desk, "l", Request::PhoneList {}).await
    else {
        panic!();
    };
    assert!(items.is_empty());
    assert!(!remote.enabled && !remote.listening, "listening while off");
}

/// The session's id from a `session.opened` reply.
fn opened_id(reply: Reply) -> String {
    match reply {
        Reply::SessionOpened(info) => info.session,
        other => panic!("expected session.opened, got {other:?}"),
    }
}

fn listed(reply: Reply) -> Vec<String> {
    match reply {
        Reply::SessionListed { items } => items.into_iter().map(|s| s.session).collect(),
        other => panic!("expected session.listed, got {other:?}"),
    }
}

fn refused(reply: &Reply) -> bool {
    matches!(reply, Reply::Error { .. })
}

#[tokio::test]
async fn a_member_cant_reach_the_owners_sessions_files_or_settings() {
    let sam = phone_keys();
    let url = engine_with("people", Some(&sam.public)).await;
    let mut owner = desktop(&url).await;
    let port = phone_port(&mut owner).await;
    let engine_key = new_code(&mut owner, "code").await.key;
    let mut phone = Phone::handshake(port, &sam, &engine_key, None, "")
        .await
        .expect("Sam's paired phone was refused");

    // The phone is Sam's, a member.
    let Reply::Welcome { user, .. } = phone.ask("h", hello("")).await else {
        panic!("no welcome");
    };
    assert_eq!((user.id.as_str(), user.role.as_str()), ("u_sam", "member"));

    let create = || Request::SessionCreate {
        workspace: None,
        title: None,
        mode: None,
    };
    let mine = opened_id(ask(&mut owner, "c1", create()).await);
    let sams = opened_id(phone.ask("c2", create()).await);

    // Each sees only their own sessions.
    let list = || Request::SessionList {
        limit: None,
        archived: false,
    };
    assert_eq!(
        listed(ask(&mut owner, "l1", list()).await),
        vec![mine.clone()]
    );
    assert_eq!(listed(phone.ask("l2", list()).await), vec![sams.clone()]);

    // Sam can't open, message, rename, retry, rewind or delete the owner's session.
    for (i, request) in [
        Request::SessionOpen {
            session: mine.clone(),
            after_seq: None,
        },
        Request::TurnSend {
            session: mine.clone(),
            text: "hello".into(),
            files: Vec::new(),
        },
        Request::SessionRename {
            session: mine.clone(),
            title: "Sam's now".into(),
        },
        Request::TurnRetry {
            session: mine.clone(),
        },
        Request::SessionSetFolder {
            session: mine.clone(),
            folder: std::env::temp_dir().to_string_lossy().into_owned(),
        },
    ]
    .into_iter()
    .enumerate()
    {
        let reply = phone.ask(&format!("x{i}"), request).await;
        assert!(refused(&reply), "Sam got {reply:?}");
    }
    let _ = phone
        .ask(
            "del",
            Request::SessionDelete {
                session: mine.clone(),
            },
        )
        .await;
    assert_eq!(
        listed(ask(&mut owner, "l3", list()).await),
        vec![mine.clone()],
        "the owner's session survived"
    );

    // What changes Thrall for everyone is the owner's alone.
    for (i, request) in [
        Request::SettingsSet {
            key: "provider.model".into(),
            value: thrall_protocol::SettingValue::Text("x/y".into()),
        },
        Request::HeartbeatGet {},
        Request::StartupGet {},
        Request::StartupSet { on: false },
        Request::FoldersGet {},
        Request::UpdateInstall {},
        Request::FolderOpen {
            folder: thrall_protocol::ThrallFolder::Data,
        },
        Request::CommandRun {
            session: Some(sams.clone()),
            name: "model".into(),
            args: "x/y".into(),
        },
        Request::CommandRun {
            session: Some(sams.clone()),
            name: "login".into(),
            args: "chatgpt".into(),
        },
        Request::EngineStop {},
    ]
    .into_iter()
    .enumerate()
    {
        let reply = phone.ask(&format!("o{i}"), request).await;
        assert!(
            matches!(
                reply,
                Reply::Error {
                    code: ErrorCode::Unauthorized,
                    ..
                }
            ),
            "Sam got {reply:?}"
        );
    }
    // The owner may. No desktop app sits beside the test program, so it isn't available and
    // nothing is written.
    assert!(matches!(
        ask(&mut owner, "s1", Request::StartupGet {}).await,
        Reply::StartupShown {
            available: false,
            ..
        }
    ));
    assert!(matches!(
        ask(&mut owner, "s3", Request::FoldersGet {}).await,
        Reply::FoldersShown { .. }
    ));
    assert!(matches!(
        ask(&mut owner, "s2", Request::StartupSet { on: true }).await,
        Reply::Error {
            code: ErrorCode::Invalid,
            ..
        }
    ));

    // Files are each person's own.
    let Reply::FileSaved { file, .. } = phone
        .ask(
            "put",
            Request::FilePut {
                upload: None,
                name: "sam.txt".into(),
                data: STANDARD.encode(b"Sam's file"),
            },
        )
        .await
    else {
        panic!("Sam couldn't upload");
    };
    let get = |file: &str| Request::FileGet {
        file: file.to_owned(),
        offset: 0,
    };
    assert!(matches!(
        phone.ask("get1", get(&file)).await,
        Reply::FilePart { .. }
    ));
    assert!(
        refused(&ask(&mut owner, "get2", get(&file)).await),
        "the owner read Sam's file"
    );

    // Live events: Sam, watching everything, hears about his own session but never the owner's.
    assert!(matches!(
        phone.ask("w", Request::SessionWatchAll {}).await,
        Reply::Ack {}
    ));
    assert!(matches!(
        ask(
            &mut owner,
            "r1",
            Request::SessionRename {
                session: mine.clone(),
                title: "Private plans".into(),
            },
        )
        .await,
        Reply::Ack {}
    ));
    phone
        .send(
            "r2",
            Request::SessionRename {
                session: sams.clone(),
                title: "Sam's things".into(),
            },
        )
        .await;
    loop {
        match phone.frame().await.expect("link closed") {
            ServerFrame::Event(frame) => {
                assert_ne!(frame.session, mine, "Sam heard the owner's session");
                if frame.session == sams {
                    break;
                }
            }
            ServerFrame::Reply(_) => {}
        }
    }
}

/// Names and roles from a `people.listed` reply, plus each person's phone count.
fn people(reply: Reply) -> Vec<(String, String, u32)> {
    match reply {
        Reply::PeopleListed { items } => items
            .into_iter()
            .map(|p| (p.name, p.role, p.phones))
            .collect(),
        other => panic!("expected people.listed, got {other:?}"),
    }
}

fn person_id(reply: &Reply, name: &str) -> String {
    match reply {
        Reply::PeopleListed { items } => items
            .iter()
            .find(|p| p.name == name)
            .map(|p| p.person.clone())
            .expect("no such person in the list"),
        other => panic!("expected people.listed, got {other:?}"),
    }
}

#[tokio::test]
async fn the_owner_adds_renames_and_removes_people() {
    let sam = phone_keys();
    let url = engine_with("people-admin", Some(&sam.public)).await;
    let mut owner = desktop(&url).await;
    let port = phone_port(&mut owner).await;
    let engine_key = new_code(&mut owner, "code").await.key;
    let entry = |name: &str, role: &str, phones: u32| (name.to_owned(), role.to_owned(), phones);

    // The owner first, then everyone else; Sam's phone counts as Sam's.
    assert_eq!(
        people(ask(&mut owner, "p1", Request::PeopleList { since: None }).await),
        vec![entry("Test", "owner", 0), entry("Sam", "member", 1)]
    );

    // A member sees only themselves, and can't change who uses Thrall.
    let mut phone = Phone::handshake(port, &sam, &engine_key, None, "")
        .await
        .expect("Sam's paired phone was refused");
    assert!(matches!(
        phone.ask("h", hello("")).await,
        Reply::Welcome { .. }
    ));
    assert_eq!(
        people(phone.ask("me", Request::PeopleList { since: None }).await),
        vec![entry("Sam", "member", 1)]
    );
    for (i, request) in [
        Request::PeopleAdd {
            name: "Sneaky".into(),
        },
        Request::PeopleRemove {
            person: "u_test".into(),
        },
    ]
    .into_iter()
    .enumerate()
    {
        let reply = phone.ask(&format!("s{i}"), request).await;
        assert!(
            matches!(
                reply,
                Reply::Error {
                    code: ErrorCode::Unauthorized,
                    ..
                }
            ),
            "Sam got {reply:?}"
        );
    }

    // Adding: names are tidied, empty ones refused; new people are members.
    let added = ask(
        &mut owner,
        "a1",
        Request::PeopleAdd {
            name: "  Alex   Lee ".into(),
        },
    )
    .await;
    let alex = person_id(&added, "Alex Lee");
    assert!(alex.starts_with("pe_"));
    assert_eq!(people(added)[2], entry("Alex Lee", "member", 0));
    assert!(refused(
        &ask(&mut owner, "a2", Request::PeopleAdd { name: "  ".into() }).await
    ));

    // Renaming.
    let renamed = ask(
        &mut owner,
        "r1",
        Request::PeopleRename {
            person: alex.clone(),
            name: "Alexis".into(),
        },
    )
    .await;
    assert_eq!(people(renamed)[2], entry("Alexis", "member", 0));

    // The owner can't be removed, and nobody can be removed twice.
    assert!(refused(
        &ask(
            &mut owner,
            "x1",
            Request::PeopleRemove {
                person: "u_test".into()
            }
        )
        .await
    ));
    assert!(refused(
        &ask(
            &mut owner,
            "x2",
            Request::PeopleRemove {
                person: "pe_nobody".into()
            }
        )
        .await
    ));

    // Removing Sam: gone from the list, the phone's link closes and it can't come back.
    assert_eq!(
        people(
            ask(
                &mut owner,
                "x3",
                Request::PeopleRemove {
                    person: "u_sam".into()
                }
            )
            .await
        ),
        vec![entry("Test", "owner", 0), entry("Alexis", "member", 0)]
    );
    while phone.frame().await.is_some() {}
    assert!(
        Phone::handshake(port, &sam, &engine_key, None, "")
            .await
            .is_none(),
        "a removed person's phone got back in"
    );
    assert!(refused(
        &ask(
            &mut owner,
            "x4",
            Request::PeopleRemove {
                person: "u_sam".into()
            }
        )
        .await
    ));
}

/// One person's entry in a `people.listed` reply.
fn person_info(reply: &Reply, id: &str) -> thrall_protocol::PersonInfo {
    match reply {
        Reply::PeopleListed { items } => items
            .iter()
            .find(|p| p.person == id)
            .cloned()
            .expect("no such person in the list"),
        other => panic!("expected people.listed, got {other:?}"),
    }
}

fn is_error(reply: &Reply, want: ErrorCode) -> bool {
    matches!(reply, Reply::Error { code, .. } if *code == want)
}

#[tokio::test]
async fn the_owner_sets_terms_and_people_set_their_own_key() {
    let sam = phone_keys();
    let url = engine_with("people-terms", Some(&sam.public)).await;
    let mut owner = desktop(&url).await;
    let port = phone_port(&mut owner).await;
    let engine_key = new_code(&mut owner, "code").await.key;
    let terms = |person: &str, own_key: bool, model: Option<&str>, limit: Option<f64>| {
        Request::PeopleTerms {
            person: person.into(),
            own_key,
            model: model.map(str::to_owned),
            monthly_limit: limit,
        }
    };

    // The owner puts Sam on his own key, a smaller model and $5 a month.
    let listed = ask(
        &mut owner,
        "t1",
        terms("u_sam", true, Some(" small/model "), Some(5.0)),
    )
    .await;
    let info = person_info(&listed, "u_sam");
    assert_eq!(
        (
            info.own_key,
            info.key_set,
            info.model.as_deref(),
            info.monthly_limit
        ),
        (true, false, Some("small/model"), Some(5.0))
    );

    // Nonsense is refused, and the owner stays on the engine's key and model.
    for (i, bad) in [
        terms("u_sam", false, None, Some(-1.0)),
        terms("u_sam", false, Some("two words"), None),
        terms("u_test", true, None, None),
        terms("u_test", false, Some("small/model"), None),
        terms("pe_nobody", false, None, None),
    ]
    .into_iter()
    .enumerate()
    {
        let reply = ask(&mut owner, &format!("b{i}"), bad).await;
        assert!(refused(&reply), "got {reply:?}");
    }
    // A limit on the owner is allowed (a guard against runaway spending).
    let listed = ask(&mut owner, "t2", terms("u_test", false, None, Some(50.0))).await;
    assert_eq!(person_info(&listed, "u_test").monthly_limit, Some(50.0));

    // Sam, on his phone: he can't change terms or anyone else's key, but can set his own.
    let mut phone = Phone::handshake(port, &sam, &engine_key, None, "")
        .await
        .expect("Sam's paired phone was refused");
    assert!(matches!(
        phone.ask("h", hello("")).await,
        Reply::Welcome { .. }
    ));
    let reply = phone.ask("s1", terms("u_sam", false, None, None)).await;
    assert!(is_error(&reply, ErrorCode::Unauthorized), "got {reply:?}");
    let reply = phone
        .ask(
            "s2",
            Request::PeopleKey {
                person: Some("u_test".into()),
                key: Some("sk-not-yours".into()),
            },
        )
        .await;
    assert!(is_error(&reply, ErrorCode::Unauthorized), "got {reply:?}");
    let reply = phone
        .ask(
            "s3",
            Request::PeopleKey {
                person: None,
                key: Some("  sk-sams-own  ".into()),
            },
        )
        .await;
    assert!(matches!(reply, Reply::Ack {}), "got {reply:?}");
    // His phone sees his own terms, and that his key is set.
    let mine = phone.ask("s4", Request::PeopleList { since: None }).await;
    let me = person_info(&mine, "u_sam");
    assert!(me.own_key && me.key_set && me.monthly_limit == Some(5.0));

    // The owner sees it's set, never the key itself.
    let listed = ask(&mut owner, "l1", Request::PeopleList { since: None }).await;
    assert!(person_info(&listed, "u_sam").key_set);
    assert!(
        !serde_json::to_string(&listed)
            .unwrap()
            .contains("sk-sams-own")
    );

    // The owner's own key isn't set this way; the owner can clear Sam's.
    let reply = ask(
        &mut owner,
        "k1",
        Request::PeopleKey {
            person: None,
            key: Some("sk-owner".into()),
        },
    )
    .await;
    assert!(refused(&reply), "got {reply:?}");
    let reply = ask(
        &mut owner,
        "k2",
        Request::PeopleKey {
            person: Some("u_sam".into()),
            key: None,
        },
    )
    .await;
    assert!(matches!(reply, Reply::Ack {}), "got {reply:?}");
    let listed = ask(&mut owner, "l2", Request::PeopleList { since: None }).await;
    assert!(!person_info(&listed, "u_sam").key_set);
}

/// Who the engine says a phone is, from its `welcome`.
async fn welcomed_as(phone: &mut Phone, id: &str) -> String {
    match phone.ask(id, hello("")).await {
        Reply::Welcome { user, .. } => user.id,
        other => panic!("expected welcome, got {other:?}"),
    }
}

#[tokio::test]
async fn a_pairing_code_is_for_one_person_and_phones_can_change_hands() {
    let url = engine_with("people-phones", None).await;
    let mut owner = desktop(&url).await;
    let port = phone_port(&mut owner).await;

    // No code for someone who isn't here.
    let reply = ask(
        &mut owner,
        "none",
        Request::PhonePair {
            person: Some("pe_nobody".into()),
        },
    )
    .await;
    assert!(
        matches!(
            reply,
            Reply::Error {
                code: ErrorCode::NotFound,
                ..
            }
        ),
        "got {reply:?}"
    );

    // A code made for Sam pairs Sam's phone.
    let code = new_code_for(&mut owner, "c", Some("u_sam")).await;
    let keys = phone_keys();
    let phone = Phone::handshake(
        port,
        &keys,
        &code.key,
        Some((&code.id, &code.secret)),
        "Sam's phone",
    )
    .await
    .expect("pairing refused");
    let answer: serde_json::Value = serde_json::from_slice(&phone.answer).unwrap();
    let phone_id = answer["phone"].as_str().unwrap().to_owned();
    drop(phone);
    let mut phone = Phone::handshake(port, &keys, &code.key, None, "")
        .await
        .expect("Sam's phone was refused");
    assert_eq!(welcomed_as(&mut phone, "h1").await, "u_sam");
    let Reply::PhoneListed { items, .. } = ask(&mut owner, "l1", Request::PhoneList {}).await
    else {
        panic!("no phone list");
    };
    assert_eq!(items[0].person, "u_sam");

    // Given to the owner: the open link closes, and it comes back as the owner.
    let given = ask(
        &mut owner,
        "a1",
        Request::PhoneAssign {
            phone: phone_id.clone(),
            person: "u_test".into(),
        },
    )
    .await;
    let Reply::PhoneListed { items, .. } = given else {
        panic!("expected phone.listed, got {given:?}");
    };
    assert_eq!(items[0].person, "u_test");
    while phone.frame().await.is_some() {}
    let mut phone = Phone::handshake(port, &keys, &code.key, None, "")
        .await
        .expect("the phone was refused after changing hands");
    assert_eq!(welcomed_as(&mut phone, "h2").await, "u_test");

    // Not to someone who isn't here, not a phone that isn't, and never from a phone.
    for (i, (phone_id, person)) in [(phone_id.as_str(), "pe_nobody"), ("ph_nothing", "u_sam")]
        .into_iter()
        .enumerate()
    {
        let reply = ask(
            &mut owner,
            &format!("x{i}"),
            Request::PhoneAssign {
                phone: phone_id.to_owned(),
                person: person.to_owned(),
            },
        )
        .await;
        assert!(
            matches!(
                reply,
                Reply::Error {
                    code: ErrorCode::NotFound,
                    ..
                }
            ),
            "got {reply:?}"
        );
    }
    let reply = phone
        .ask(
            "x9",
            Request::PhoneAssign {
                phone: phone_id.clone(),
                person: "u_sam".into(),
            },
        )
        .await;
    assert!(
        matches!(
            reply,
            Reply::Error {
                code: ErrorCode::Unauthorized,
                ..
            }
        ),
        "got {reply:?}"
    );
}

/// The PC listener's port, once it's listening.
async fn pc_port(desk: &mut Socket) -> u16 {
    for i in 0..50 {
        if let Reply::PcRequests { pcs, .. } =
            ask(desk, &format!("pl{i}"), Request::PcRequests {}).await
            && pcs.listening
        {
            return pcs.port;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("the PC link never started");
}

/// The PCs asking now, and the blocked ones' names.
async fn pc_requests(
    desk: &mut Socket,
    id: &str,
) -> (Vec<thrall_protocol::PcRequestInfo>, Vec<String>) {
    match ask(desk, id, Request::PcRequests {}).await {
        Reply::PcRequests { items, blocked, .. } => {
            (items, blocked.into_iter().map(|b| b.name).collect())
        }
        other => panic!("expected pc.requests, got {other:?}"),
    }
}

/// Wait until a PC is asking; its request.
async fn first_request(desk: &mut Socket) -> thrall_protocol::PcRequestInfo {
    for i in 0..50 {
        if let Some(r) = pc_requests(desk, &format!("w{i}"))
            .await
            .0
            .into_iter()
            .next()
        {
            return r;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("no PC asked");
}

/// A PC asking to join, link open.
struct Asking {
    socket: Socket,
    transport: TransportState,
    /// The code it shows.
    code: String,
    /// The engine's key, learned in the handshake.
    engine_key: Vec<u8>,
}

impl Asking {
    /// Ask to join on `port`; `None` when the engine turns the handshake away.
    async fn start(port: u16, keys: &Keypair, name: &str) -> Option<Self> {
        let url = format!("ws://127.0.0.1:{port}/v1/remote");
        let (mut socket, _) = tokio_tungstenite::connect_async(url).await.unwrap();
        let mut hs = Builder::new(ASK.parse().unwrap())
            .local_private_key(&keys.private)
            .unwrap()
            .prologue(PROLOGUE)
            .unwrap()
            .build_initiator()
            .unwrap();
        let mut message = vec![0u8; 65_535];
        let n = hs.write_message(&[], &mut message).unwrap();
        let first = [&[3u8][..], &message[..n]].concat();
        socket.send(Message::Binary(first.into())).await.unwrap();
        let second = match tokio::time::timeout(Duration::from_secs(10), socket.next()).await {
            Ok(Some(Ok(Message::Binary(bytes)))) => bytes,
            _ => return None,
        };
        let mut payload = vec![0u8; 65_535];
        hs.read_message(&second, &mut payload).unwrap();
        let hello = serde_json::json!({ "name": name }).to_string();
        let n = hs.write_message(hello.as_bytes(), &mut message).unwrap();
        socket
            .send(Message::Binary(message[..n].to_vec().into()))
            .await
            .unwrap();
        let code = thrall_server::remote::ask_code(hs.get_handshake_hash());
        let engine_key = hs.get_remote_static().unwrap().to_vec();
        Some(Self {
            socket,
            transport: hs.into_transport_mode().unwrap(),
            code,
            engine_key,
        })
    }

    /// What the owner decided: the device id if let in, `None` if turned away.
    async fn verdict(mut self) -> Option<String> {
        let mut partial = Vec::new();
        loop {
            let msg = tokio::time::timeout(Duration::from_secs(10), self.socket.next())
                .await
                .expect("no answer within 10 s");
            match msg {
                Some(Ok(Message::Binary(bytes))) => {
                    if let Some(whole) = open(&mut self.transport, &bytes, &mut partial).unwrap() {
                        let answer: serde_json::Value = serde_json::from_slice(&whole).unwrap();
                        return (answer["approved"] == true)
                            .then(|| answer["device"].as_str().unwrap().to_owned());
                    }
                }
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => {
                    panic!("closed without an answer")
                }
                Some(Ok(_)) => {}
            }
        }
    }
}

#[tokio::test]
async fn a_pc_asks_to_join_and_the_owner_lets_it_in() {
    let url = engine("pc-accept").await;
    let mut desk = desktop(&url).await;
    let port = pc_port(&mut desk).await;
    let keys = phone_keys();
    let asking = Asking::start(port, &keys, "KIDS-PC").await.unwrap();

    // The owner sees it, with the same code the PC shows.
    let request = first_request(&mut desk).await;
    assert_eq!(request.name, "KIDS-PC");
    assert_eq!(request.code, asking.code);
    assert_eq!(request.code.len(), 7, "three digits, a space, three digits");

    // A typed code must be the PC's.
    let wrong = if asking.code == "000 000" {
        "111 111"
    } else {
        "000 000"
    };
    let reply = ask(
        &mut desk,
        "a1",
        Request::PcAccept {
            request: request.request.clone(),
            person: "u_sam".into(),
            code: Some(wrong.into()),
        },
    )
    .await;
    assert!(is_error(&reply, ErrorCode::Invalid), "got {reply:?}");
    let reply = ask(
        &mut desk,
        "a2",
        Request::PcAccept {
            request: request.request.clone(),
            person: "u_sam".into(),
            code: Some(asking.code.replace(' ', "")),
        },
    )
    .await;
    let Reply::PcRequests { items, .. } = reply else {
        panic!("expected pc.requests, got {reply:?}");
    };
    assert!(items.is_empty(), "still asking: {items:?}");
    let engine_key = asking.engine_key.clone();
    let device = asking.verdict().await.expect("let in");

    // It's listed as Sam's PC, and connects as Sam like a paired phone.
    let Reply::PhoneListed { items, .. } = ask(&mut desk, "l", Request::PhoneList {}).await else {
        panic!("no list");
    };
    let pc = items.iter().find(|p| p.phone == device).unwrap();
    assert_eq!(
        (pc.kind.as_str(), pc.person.as_str(), pc.name.as_str()),
        ("pc", "u_sam", "KIDS-PC")
    );
    let mut link = Phone::handshake(port, &keys, &engine_key, None, "")
        .await
        .expect("a joined PC connects");
    assert_eq!(welcomed_as(&mut link, "h").await, "u_sam");

    // Asking again, it's let straight back in.
    let again = Asking::start(port, &keys, "KIDS-PC").await.unwrap();
    assert_eq!(again.verdict().await.as_deref(), Some(device.as_str()));
}

#[tokio::test]
async fn pcs_can_be_turned_away_blocked_and_unblocked() {
    let url = engine("pc-block").await;
    let mut desk = desktop(&url).await;
    let port = pc_port(&mut desk).await;
    let keys = phone_keys();

    // Declined: told no, and may ask again.
    let asking = Asking::start(port, &keys, "STRANGER").await.unwrap();
    let request = first_request(&mut desk).await;
    let reply = ask(
        &mut desk,
        "d1",
        Request::PcDecline {
            request: request.request,
            block: false,
        },
    )
    .await;
    assert!(matches!(reply, Reply::PcRequests { .. }), "got {reply:?}");
    assert_eq!(asking.verdict().await, None);

    // Blocked: told no, then turned away without the owner being asked.
    let asking = Asking::start(port, &keys, "STRANGER").await.unwrap();
    let request = first_request(&mut desk).await;
    ask(
        &mut desk,
        "d2",
        Request::PcDecline {
            request: request.request,
            block: true,
        },
    )
    .await;
    assert_eq!(asking.verdict().await, None);
    let asking = Asking::start(port, &keys, "STRANGER").await.unwrap();
    assert_eq!(asking.verdict().await, None);
    let (items, blocked) = pc_requests(&mut desk, "r1").await;
    assert!(items.is_empty(), "a blocked PC was listed: {items:?}");
    assert_eq!(blocked, vec!["STRANGER".to_owned()]);

    // Unblocked: it may ask again.
    let Reply::PcRequests { blocked, .. } = ask(&mut desk, "r2", Request::PcRequests {}).await
    else {
        panic!("no list");
    };
    let reply = ask(
        &mut desk,
        "u",
        Request::PcUnblock {
            blocked: blocked[0].blocked.clone(),
        },
    )
    .await;
    assert!(matches!(reply, Reply::PcRequests { .. }), "got {reply:?}");
    let _asking = Asking::start(port, &keys, "STRANGER").await.unwrap();
    assert_eq!(first_request(&mut desk).await.name, "STRANGER");

    // The phone link never takes requests to join.
    let phone_port = phone_port(&mut desk).await;
    assert!(
        Asking::start(phone_port, &phone_keys(), "SNEAKY")
            .await
            .is_none()
    );
}

#[tokio::test]
async fn a_joined_pcs_sessions_work_in_its_folder_and_its_runner_does_the_work() {
    use thrall_server::companion_client::{self, Link};
    use thrall_tools::companion::{Base, Call, Checked, Ran, Runner};

    let running = engine_running("pc-runner", None).await;
    let hub = running.shared.hub.clone();
    let mut desk = desktop(&format!("ws://{}/v1/ws", running.addr)).await;
    let port = pc_port(&mut desk).await;

    // Sam's PC asks to join; the owner lets it in as Sam's.
    let (code_tx, code_rx) = tokio::sync::oneshot::channel();
    let asking = tokio::spawn(async move {
        companion_client::ask(&format!("127.0.0.1:{port}"), "SAMS-PC", move |code| {
            let _ = code_tx.send(code.to_owned());
        })
        .await
    });
    let code = code_rx.await.unwrap();
    let request = first_request(&mut desk).await;
    assert_eq!(request.code, code);
    ask(
        &mut desk,
        "a",
        Request::PcAccept {
            request: request.request,
            person: "u_sam".into(),
            code: Some(code),
        },
    )
    .await;
    let joined = asking.await.unwrap().unwrap().expect("let in");

    // Its runner connects, with its own folder.
    let pc_home = std::env::temp_dir().join(format!("thrall-pc-home-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&pc_home);
    std::fs::create_dir_all(&pc_home).unwrap();
    let base = Base {
        tools: thrall_tools::tools::core(),
        store: Arc::new(SqliteStore::open_in_memory().await.unwrap()),
        http: thrall_tools::http_client().unwrap(),
        data_dir: None,
        browser: Arc::new(thrall_tools::browser::Browsers::default()),
        home: Some(pc_home.clone()),
    };
    let runner = Arc::new(Runner::new(base));
    let home = pc_home.to_string_lossy().into_owned();
    {
        let (joined, home) = (joined.clone(), home.clone());
        tokio::spawn(async move { companion_client::run_once(&joined, runner, &home, "0").await });
    }
    let companions = hub.state().companions.clone();
    for _ in 0..100 {
        if companions.connected(&joined.device) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        companions.connected(&joined.device),
        "the runner never connected"
    );

    // A session started on the PC is Sam's, works on the PC, in the PC's folder.
    let mut window = Link::connect(&joined).await.unwrap();
    window
        .hello(thrall_protocol::ClientKind::Desktop, "0")
        .await
        .unwrap();
    let create = Request::SessionCreate {
        workspace: None,
        title: None,
        mode: None,
    };
    window
        .send(&serde_json::to_string(&RequestFrame::new("c", create)).unwrap())
        .await
        .unwrap();
    let info = loop {
        let text = window.next().await.unwrap().unwrap();
        if let Ok(ServerFrame::Reply(r)) = serde_json::from_str::<ServerFrame>(&text)
            && r.re == "c"
        {
            let Reply::SessionOpened(info) = r.reply else {
                panic!("expected session.opened, got {:?}", r.reply);
            };
            break info;
        }
    };
    assert_eq!(info.workspace.as_deref(), Some(home.as_str()));
    let stored = hub
        .state()
        .store
        .get_session("u_sam", &info.session)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.pc.as_deref(), Some(joined.device.as_str()));

    // A file edit is checked, then done, on the PC.
    let args = serde_json::json!({ "path": "hello.txt", "content": "made on Sam's PC" });
    let checked = companions
        .call(
            &joined.device,
            "u_sam",
            &info.session,
            &home,
            Call::Check {
                tool: "write_file".into(),
                args: args.clone(),
            },
            None,
        )
        .await
        .unwrap();
    let checked: Checked = serde_json::from_value(checked).unwrap();
    assert!(checked.precheck.is_none(), "{:?}", checked.precheck);
    assert!(
        serde_json::to_value(&checked.effect).unwrap()["kind"] == "edit",
        "{:?}",
        checked.effect
    );
    let ran = companions
        .call(
            &joined.device,
            "u_sam",
            &info.session,
            &home,
            Call::Run {
                tool: "write_file".into(),
                args,
            },
            None,
        )
        .await
        .unwrap();
    let ran: Ran = serde_json::from_value(ran).unwrap();
    assert!(ran.ok, "{}", ran.content);
    assert_eq!(
        std::fs::read_to_string(pc_home.join("hello.txt")).unwrap(),
        "made on Sam's PC"
    );

    // The Files view (on a phone, say) shows the PC's folder, not this computer's.
    let list = Request::FolderList {
        session: info.session.clone(),
        path: String::new(),
    };
    window
        .send(&serde_json::to_string(&RequestFrame::new("f", list)).unwrap())
        .await
        .unwrap();
    let names = loop {
        let text = window.next().await.unwrap().unwrap();
        if let Ok(ServerFrame::Reply(r)) = serde_json::from_str::<ServerFrame>(&text)
            && r.re == "f"
        {
            let Reply::FolderListed { items, .. } = r.reply else {
                panic!("expected folder.listed, got {:?}", r.reply);
            };
            break items.into_iter().map(|i| i.name).collect::<Vec<_>>();
        }
    };
    assert_eq!(names, vec!["hello.txt".to_owned()]);

    // A tool that doesn't run on PCs is refused there, and a PC that isn't connected says so.
    let refused = companions
        .call(
            &joined.device,
            "u_sam",
            &info.session,
            &home,
            Call::Run {
                tool: "web_search".into(),
                args: serde_json::json!({}),
            },
            None,
        )
        .await;
    assert!(refused.is_err());
    let away = companions
        .call(
            "p_nobody",
            "u_sam",
            &info.session,
            &home,
            Call::Stop { call: "x".into() },
            None,
        )
        .await;
    assert!(away.unwrap_err().contains("isn't connected"));
}

#[tokio::test]
async fn a_companion_window_uses_thrall_through_its_pc() {
    use thrall_server::companion_bridge;
    use thrall_server::companion_client;

    let running = engine_running("pc-bridge", None).await;
    let mut desk = desktop(&format!("ws://{}/v1/ws", running.addr)).await;
    let port = pc_port(&mut desk).await;
    let (code_tx, code_rx) = tokio::sync::oneshot::channel();
    let asking = tokio::spawn(async move {
        companion_client::ask(&format!("127.0.0.1:{port}"), "SAMS-PC", move |code| {
            let _ = code_tx.send(code.to_owned());
        })
        .await
    });
    let code = code_rx.await.unwrap();
    let request = first_request(&mut desk).await;
    ask(
        &mut desk,
        "a",
        Request::PcAccept {
            request: request.request,
            person: "u_sam".into(),
            code: Some(code),
        },
    )
    .await;
    let mut joined = asking.await.unwrap().unwrap().expect("let in");
    joined.name = "THRALL-PC".into();

    // The PC's own data folder: the bridge describes itself there like an engine.
    let home = std::env::temp_dir().join(format!("thrall-pc-data-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    let paths = thrall_core::Paths::under(&home);
    let bridge = companion_bridge::start(joined, &paths, "0").await.unwrap();
    let file = thrall_core::EngineFile::read(&paths.engine_file).expect("engine.json");
    assert_eq!(file.port, bridge.addr.port());
    let token = std::fs::read_to_string(&paths.token_file).unwrap();

    // Without the token: turned away. With it: welcomed as Sam, by the Thrall PC.
    let (mut stranger, _) = tokio_tungstenite::connect_async(&file.ws_url)
        .await
        .unwrap();
    assert!(matches!(
        ask(&mut stranger, "h", hello("wrong")).await,
        Reply::Error { .. }
    ));
    let (mut window, _) = tokio_tungstenite::connect_async(&file.ws_url)
        .await
        .unwrap();
    match ask(&mut window, "h", hello(token.trim())).await {
        Reply::Welcome { user, .. } => assert_eq!(user.id, "u_sam"),
        other => panic!("expected welcome, got {other:?}"),
    }

    // A window's sign-in link is the PC's own, and the code trades for the PC's token once.
    let Reply::SigninCode { code, .. } = ask(
        &mut window,
        "s",
        Request::SigninCreate {
            person: "u_sam".into(),
        },
    )
    .await
    else {
        panic!("no sign-in code");
    };
    let client = reqwest::Client::new();
    let signin = format!("http://127.0.0.1:{}/v1/signin", bridge.addr.port());
    let first: serde_json::Value = client
        .post(&signin)
        .json(&serde_json::json!({ "code": code, "name": "Thrall Desktop" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(first["token"].as_str(), Some(token.trim()));
    let again = client
        .post(&signin)
        .json(&serde_json::json!({ "code": code }))
        .send()
        .await
        .unwrap();
    assert_eq!(again.status(), reqwest::StatusCode::UNAUTHORIZED);

    // Requests go on to the Thrall PC: Sam's sessions are Sam's.
    assert!(matches!(
        ask(
            &mut window,
            "l",
            Request::SessionList {
                limit: None,
                archived: false
            }
        )
        .await,
        Reply::SessionListed { .. }
    ));
    companion_bridge::stopped(&paths);
    assert!(!paths.engine_file.exists());
}

#[tokio::test]
async fn the_owner_switches_controls_off_for_someone_and_phones_obey() {
    let sam = phone_keys();
    let url = engine_with("rules", Some(&sam.public)).await;
    let mut owner = desktop(&url).await;
    let port = phone_port(&mut owner).await;
    let engine_key = new_code(&mut owner, "code").await.key;
    let mut phone = Phone::handshake(port, &sam, &engine_key, None, "")
        .await
        .unwrap();
    assert!(matches!(
        phone.ask("h", hello("")).await,
        Reply::Welcome { .. }
    ));

    // Everything is allowed until switched off; the owner's own can't be.
    let listed = ask(&mut owner, "l", Request::PeopleList { since: None }).await;
    assert_eq!(
        person_info(&listed, "u_sam").rules,
        thrall_protocol::PersonRules::default()
    );
    let mut rules = thrall_protocol::PersonRules {
        no_phones: true,
        child: Some("13_15".into()),
        ..Default::default()
    };
    let refused = ask(
        &mut owner,
        "o",
        Request::PeopleRules {
            person: "u_test".into(),
            rules: rules.clone(),
        },
    )
    .await;
    assert!(is_error(&refused, ErrorCode::Invalid), "got {refused:?}");
    rules.child = Some("nine".into());
    let bad = ask(
        &mut owner,
        "b",
        Request::PeopleRules {
            person: "u_sam".into(),
            rules: rules.clone(),
        },
    )
    .await;
    assert!(is_error(&bad, ErrorCode::Invalid), "got {bad:?}");
    rules.child = Some("13_15".into());
    let set = ask(
        &mut owner,
        "s",
        Request::PeopleRules {
            person: "u_sam".into(),
            rules: rules.clone(),
        },
    )
    .await;
    assert_eq!(person_info(&set, "u_sam").rules, rules);

    // Sam's phone is disconnected, and can't come back while phones are off.
    assert!(phone.frame().await.is_none(), "the phone link stayed open");
    let mut again = Phone::handshake(port, &sam, &engine_key, None, "")
        .await
        .unwrap();
    assert!(!matches!(
        again.ask("h2", hello("")).await,
        Reply::Welcome { .. }
    ));
}

#[tokio::test]
async fn a_pc_its_thrall_forgot_is_told_to_join_again() {
    use thrall_server::companion_client::{self, Joined, Link};
    let url = engine("pc-forgot").await;
    let mut desk = desktop(&url).await;
    let port = pc_port(&mut desk).await;
    let engine_key = new_code(&mut desk, "k").await.key;
    let keys = phone_keys();
    let hex = |b: &[u8]| b.iter().map(|x| format!("{x:02x}")).collect::<String>();
    let stale = Joined {
        address: format!("127.0.0.1:{port}"),
        name: "THRALL-PC".into(),
        master_key: hex(&engine_key),
        private: hex(&keys.private),
        public: hex(&keys.public),
        device: "ph_gone".into(),
    };
    let Err(why) = Link::connect(&stale).await else {
        panic!("a PC nobody let in got in");
    };
    assert!(companion_client::is_not_joined(&why), "{why}");

    // Many such tries don't stop a PC from asking to join.
    for _ in 0..25 {
        let _ = Link::connect(&stale).await;
    }
    let (tx, rx) = tokio::sync::oneshot::channel();
    let asking = tokio::spawn(async move {
        companion_client::ask(&format!("127.0.0.1:{port}"), "SAMS-PC", move |c| {
            let _ = tx.send(c.to_owned());
        })
        .await
    });
    rx.await.expect("the request went through");
    asking.abort();
}
