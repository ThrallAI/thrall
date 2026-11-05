//! On a companion PC, `thrall runner` stands in for the local engine: it serves the web app on
//! this PC and carries each window's connection to the joined Thrall over the encrypted link.
//! The desktop app finds it the usual way (`engine.json` and the access token), so it works the
//! same here as on the Thrall PC; the person, the chats and the AI are the joined Thrall's.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::extract::ws::{Message, WebSocket};
use axum::extract::{DefaultBodyLimit, Request as HttpRequest, State, WebSocketUpgrade};
use axum::http::{HeaderMap, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use thrall_core::constants::id_prefix;
use thrall_core::ids::{new_id, now_ms};
use thrall_core::{EngineFile, Paths};
use thrall_protocol::constants::{FILE_CHUNK_BYTES, WS_PATH};
use thrall_protocol::{
    ClientKind, RemoteInfo, Reply, ReplyFrame, Request, RequestFrame, ServerFrame,
};

use crate::companion_client::{Joined, Link};
use crate::constants::{
    BRIDGE_CODE_LIFETIME, DESKTOP_ORIGINS, FILE_PATH, FILES_PATH, HEALTH_PATH, LOCAL_HOST,
    MAX_UPLOAD_BYTES, SIGNIN_PATH,
};

/// What the bridge's routes share.
#[derive(Clone)]
struct Bridge {
    joined: Arc<Joined>,
    /// This PC's access token (the desktop app shows it in hello; a signed-in window gets it too).
    token: Arc<String>,
    /// One-time sign-in codes for windows, with when they stop working (Unix ms).
    codes: Arc<Mutex<Vec<(String, i64)>>>,
    version: Arc<String>,
    origins: Arc<Vec<String>>,
    /// Told when the desktop app asks this PC's Thrall to stop.
    stop: Arc<tokio::sync::Notify>,
}

fn random_hex(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    if getrandom::fill(&mut buf).is_err() {
        return String::new();
    }
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

/// The running bridge.
pub struct Running {
    /// Where it listens.
    pub addr: SocketAddr,
    /// Told when the desktop app asks this PC's Thrall to stop.
    pub stop: Arc<tokio::sync::Notify>,
}

/// Start the bridge on 127.0.0.1 and describe it in `engine.json`, as an engine would.
pub async fn start(joined: Joined, paths: &Paths, version: &str) -> std::io::Result<Running> {
    // The access token stays the same across restarts, so a signed-in window stays signed in.
    let token = std::fs::read_to_string(&paths.token_file)
        .ok()
        .map(|t| t.trim().to_owned())
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| random_hex(32));
    if token.is_empty() {
        return Err(std::io::Error::other(
            "no random numbers for an access token",
        ));
    }
    std::fs::create_dir_all(&paths.home)?;
    std::fs::write(&paths.token_file, &token)?;
    let listener = tokio::net::TcpListener::bind((LOCAL_HOST, 0)).await?;
    let addr = listener.local_addr()?;
    let mut origins = vec![
        format!("http://{LOCAL_HOST}:{}", addr.port()),
        format!("http://localhost:{}", addr.port()),
    ];
    origins.extend(DESKTOP_ORIGINS.iter().map(|o| (*o).to_owned()));
    let stop = Arc::new(tokio::sync::Notify::new());
    let bridge = Bridge {
        joined: Arc::new(joined),
        token: Arc::new(token),
        codes: Arc::new(Mutex::new(Vec::new())),
        version: Arc::new(version.to_owned()),
        origins: Arc::new(origins),
        stop: stop.clone(),
    };
    let hosts = Arc::new(vec![
        format!("{LOCAL_HOST}:{}", addr.port()),
        format!("localhost:{}", addr.port()),
    ]);
    let router = Router::new()
        .route(WS_PATH, get(upgrade))
        .route(HEALTH_PATH, get(health))
        .route(
            FILES_PATH,
            post(upload).layer(DefaultBodyLimit::max(MAX_UPLOAD_BYTES)),
        )
        .route(FILE_PATH, get(download))
        .route(SIGNIN_PATH, post(sign_in))
        .with_state(bridge)
        .merge(crate::web::router())
        .layer(middleware::from_fn(move |req: HttpRequest, next: Next| {
            let hosts = hosts.clone();
            async move {
                if crate::host_allowed(req.headers(), &hosts) {
                    next.run(req).await
                } else {
                    (StatusCode::MISDIRECTED_REQUEST, "Unknown host.").into_response()
                }
            }
        }));
    tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, router).await {
            tracing::error!(%e, "the companion's web app stopped");
        }
    });
    EngineFile::new(LOCAL_HOST, addr.port(), WS_PATH, version)
        .write(&paths.engine_file)
        .map_err(std::io::Error::other)?;
    tracing::info!(%addr, "companion web app listening");
    Ok(Running { addr, stop })
}

/// Remove `engine.json` if it still describes this process.
pub fn stopped(paths: &Paths) {
    EngineFile::remove_if_mine(&paths.engine_file);
}

async fn health(State(b): State<Bridge>) -> impl IntoResponse {
    (
        [(header::CACHE_CONTROL, "no-store")],
        axum::Json(serde_json::json!({ "ok": true, "version": *b.version, "companion": true })),
    )
}

fn bearer_ok(b: &Bridge, headers: &HeaderMap) -> bool {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .is_some_and(|t| t == b.token.as_str())
}

/// A window trades its one-time code for this PC's access token.
async fn sign_in(State(b): State<Bridge>, headers: HeaderMap, body: axum::body::Bytes) -> Response {
    if !crate::origin_allowed(&headers, &b.origins) {
        return crate::uploads::refuse(StatusCode::FORBIDDEN, "This page can't sign in to Thrall.");
    }
    let code = serde_json::from_slice::<serde_json::Value>(&body)
        .ok()
        .and_then(|v| v["code"].as_str().map(str::to_owned))
        .unwrap_or_default();
    let now = now_ms();
    let used = {
        let mut codes = b.codes.lock().unwrap_or_else(|p| p.into_inner());
        codes.retain(|(_, until)| *until > now);
        let before = codes.len();
        codes.retain(|(c, _)| *c != code);
        codes.len() != before
    };
    if !used {
        return crate::uploads::refuse(
            StatusCode::UNAUTHORIZED,
            "That sign-in link has been used or has run out.",
        );
    }
    axum::Json(serde_json::json!({ "token": *b.token })).into_response()
}

/// A fresh link to the joined Thrall for one HTTP request.
async fn open_link(b: &Bridge) -> Result<Link, String> {
    let mut link = Link::connect(&b.joined).await?;
    link.hello(ClientKind::Desktop, &b.version).await?;
    Ok(link)
}

/// Send `request` and wait for its reply.
async fn ask(link: &mut Link, request: Request) -> Result<Reply, String> {
    let id = new_id(id_prefix::REQUEST);
    let text =
        serde_json::to_string(&RequestFrame::new(&id, request)).map_err(|e| e.to_string())?;
    link.send(&text).await?;
    loop {
        let text = link.next().await.ok_or("Thrall closed the link")??;
        if let Ok(ServerFrame::Reply(r)) = serde_json::from_str::<ServerFrame>(&text)
            && r.re == id
        {
            return Ok(r.reply);
        }
    }
}

/// An attachment from a window: sent on to the joined Thrall in pieces.
async fn upload(State(b): State<Bridge>, headers: HeaderMap, body: axum::body::Bytes) -> Response {
    if !crate::origin_allowed(&headers, &b.origins) || !bearer_ok(&b, &headers) {
        return crate::uploads::refuse(
            StatusCode::UNAUTHORIZED,
            "Thrall didn't accept this upload.",
        );
    }
    let name = headers
        .get("x-file-name")
        .and_then(|v| v.to_str().ok())
        .map(|n| crate::uploads::clean_name(&urlencoding_decode(n)))
        .unwrap_or_default();
    let sent = async {
        let mut link = open_link(&b).await?;
        let mut upload: Option<String> = None;
        let mut saved = None;
        let chunks: Vec<&[u8]> = if body.is_empty() {
            vec![&[]]
        } else {
            body.chunks(FILE_CHUNK_BYTES).collect()
        };
        for chunk in chunks {
            let reply = ask(
                &mut link,
                Request::FilePut {
                    upload: upload.clone(),
                    name: name.clone(),
                    data: BASE64.encode(chunk),
                },
            )
            .await?;
            match reply {
                Reply::FileSaved { file, name, size } => {
                    upload = Some(file.clone());
                    saved = Some(serde_json::json!({ "file": file, "name": name, "size": size }));
                }
                Reply::Error { message, .. } => return Err(message),
                other => return Err(format!("unexpected answer: {other:?}")),
            }
        }
        saved.ok_or_else(|| "nothing was sent".to_owned())
    }
    .await;
    match sent {
        Ok(saved) => axum::Json(saved).into_response(),
        Err(why) => crate::uploads::refuse(StatusCode::BAD_GATEWAY, &why),
    }
}

/// `%xx` escapes undone (the web app sends the file's name encoded).
fn urlencoding_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(byte) = text
                .get(i + 1..i + 3)
                .and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(byte);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A file from the joined Thrall (an attachment, a picture Thrall made), fetched in pieces.
async fn download(
    State(b): State<Bridge>,
    axum::extract::Path(id): axum::extract::Path<String>,
    headers: HeaderMap,
) -> Response {
    if !bearer_ok(&b, &headers) {
        return crate::uploads::refuse(StatusCode::UNAUTHORIZED, "Sign in to see this file.");
    }
    let got = async {
        let mut link = open_link(&b).await?;
        let mut bytes = Vec::new();
        let name = loop {
            let reply = ask(
                &mut link,
                Request::FileGet {
                    file: id.clone(),
                    offset: bytes.len() as u64,
                },
            )
            .await?;
            match reply {
                Reply::FilePart {
                    name, size, data, ..
                } => {
                    let piece = BASE64.decode(data).map_err(|e| e.to_string())?;
                    let done = piece.is_empty();
                    bytes.extend_from_slice(&piece);
                    if done || bytes.len() as u64 >= size {
                        break name;
                    }
                }
                Reply::Error { message, .. } => return Err(message),
                other => return Err(format!("unexpected answer: {other:?}")),
            }
        };
        Ok::<_, String>((name, bytes))
    }
    .await;
    match got {
        Ok((name, bytes)) => {
            let dir = std::env::temp_dir().join(format!("thrall-companion-{}", std::process::id()));
            let path: PathBuf = dir.join(&id).join(crate::uploads::clean_name(&name));
            let saved = path
                .parent()
                .map(std::fs::create_dir_all)
                .unwrap_or(Ok(()))
                .and_then(|()| std::fs::write(&path, &bytes));
            match saved {
                Ok(()) => crate::uploads::send(Some(path)).await,
                Err(e) => crate::uploads::refuse(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
            }
        }
        Err(why) => crate::uploads::refuse(StatusCode::NOT_FOUND, &why),
    }
}

/// A window connects: its hello must carry this PC's token; then every frame goes to the joined
/// Thrall and back. A few requests are this PC's own and answered here: a window's sign-in link,
/// stopping this PC's Thrall, and the phone count the Quit dialog shows.
async fn upgrade(State(b): State<Bridge>, headers: HeaderMap, ws: WebSocketUpgrade) -> Response {
    if !crate::origin_allowed(&headers, &b.origins) {
        return (StatusCode::FORBIDDEN, "This page can't connect to Thrall.").into_response();
    }
    ws.on_upgrade(move |socket| carry(socket, b))
}

fn local_reply(id: &str, reply: Reply) -> Message {
    let frame = ServerFrame::Reply(ReplyFrame::new(id, reply));
    Message::Text(serde_json::to_string(&frame).unwrap_or_default().into())
}

async fn carry(mut socket: WebSocket, b: Bridge) {
    let hello = match socket.recv().await {
        Some(Ok(Message::Text(text))) => text,
        _ => return,
    };
    let Ok(frame) = serde_json::from_str::<RequestFrame>(hello.as_str()) else {
        return;
    };
    let Request::Hello {
        protocols,
        client,
        token,
    } = frame.request
    else {
        return;
    };
    if token != *b.token {
        let refused = ServerFrame::Reply(ReplyFrame::error(
            &frame.id,
            thrall_protocol::ErrorCode::Unauthorized,
            "This window isn't signed in to Thrall.",
        ));
        let _ = socket
            .send(Message::Text(
                serde_json::to_string(&refused).unwrap_or_default().into(),
            ))
            .await;
        return;
    }
    let mut link = match Link::connect(&b.joined).await {
        Ok(l) => l,
        Err(why) => {
            let refused = ServerFrame::Reply(ReplyFrame::error(
                &frame.id,
                thrall_protocol::ErrorCode::Internal,
                format!("Couldn't reach Thrall on {}: {why}", b.joined.name),
            ));
            let _ = socket
                .send(Message::Text(
                    serde_json::to_string(&refused).unwrap_or_default().into(),
                ))
                .await;
            return;
        }
    };
    // The joined Thrall knows this PC by its key; the token stays here.
    let hello = RequestFrame::new(
        &frame.id,
        Request::Hello {
            protocols,
            client,
            token: String::new(),
        },
    );
    if link
        .send(&serde_json::to_string(&hello).unwrap_or_default())
        .await
        .is_err()
    {
        return;
    }
    loop {
        tokio::select! {
            msg = socket.recv() => {
                let text = match msg {
                    Some(Ok(Message::Text(text))) => text,
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                    Some(Ok(_)) => continue,
                };
                let local = serde_json::from_str::<RequestFrame>(text.as_str()).ok().and_then(|f| {
                    match f.request {
                        Request::SigninCreate { .. } => {
                            let code = random_hex(16);
                            let expires = now_ms() + BRIDGE_CODE_LIFETIME.as_millis() as i64;
                            b.codes
                                .lock()
                                .unwrap_or_else(|p| p.into_inner())
                                .push((code.clone(), expires));
                            Some(local_reply(
                                &f.id,
                                Reply::SigninCode {
                                    code,
                                    name: b.joined.name.clone(),
                                    expires,
                                },
                            ))
                        }
                        Request::EngineStop {} => {
                            b.stop.notify_one();
                            Some(local_reply(&f.id, Reply::Ack {}))
                        }
                        Request::PhoneList {} => Some(local_reply(
                            &f.id,
                            Reply::PhoneListed {
                                items: Vec::new(),
                                remote: RemoteInfo {
                                    enabled: false,
                                    address: String::new(),
                                    listening: false,
                                    port: 0,
                                },
                            },
                        )),
                        _ => None,
                    }
                });
                match local {
                    Some(reply) => {
                        if socket.send(reply).await.is_err() {
                            break;
                        }
                    }
                    None => {
                        if link.send(text.as_str()).await.is_err() {
                            break;
                        }
                    }
                }
            }
            text = link.next() => match text {
                Some(Ok(text)) => {
                    if socket.send(Message::Text(text.into())).await.is_err() {
                        break;
                    }
                }
                _ => break,
            },
        }
    }
    let _ = socket.send(Message::Close(None)).await;
}
