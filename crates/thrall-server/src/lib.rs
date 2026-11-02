//! The engine's network side: the protocol endpoint (`/v1/ws`), the web app and a health check.
//! It listens on 127.0.0.1 only. A connection needs the access token from the data folder, and a
//! browser connection must come from Thrall's own web app or the desktop app's window, so other
//! web pages can't drive Thrall. Paired phones come in through the encrypted phone link
//! ([`remote`]), on its own listener, only while remote access is on.

pub mod app_release;
pub mod connection;
pub mod constants;
pub mod error;
pub mod folders;
pub mod people;
pub mod remote;
pub mod signin;
pub mod token;
pub mod uploads;
pub mod web;

use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::extract::{Request, State, WebSocketUpgrade};
use axum::http::{HeaderMap, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use thrall_agent::Hub;
use thrall_agent::hub::uploaded_file;
use thrall_protocol::constants::{MAX_FRAME_BYTES, WS_PATH};

pub use connection::Shared;
pub use error::ServerError;
pub use thrall_core::EngineFile;

use crate::constants::{
    DESKTOP_ORIGINS, FILE_PATH, FILES_PATH, HEALTH_PATH, LOCAL_HOST, MAX_UPLOAD_BYTES, SIGNIN_PATH,
};

/// How to run the server.
pub struct Options {
    /// Port to listen on (0 picks a free one; tests use that).
    pub port: u16,
    /// Access token clients must present.
    pub token: String,
    /// Engine version, for `welcome` and the health check.
    pub version: String,
    /// Extra browser origins allowed to connect (the web app's dev server while developing).
    pub extra_origins: Vec<String>,
    /// Serve the built-in web app at `/`.
    pub serve_web: bool,
    /// Port for the phone link instead of `[remote] port` (tests use 0 for a free one).
    pub remote_port: Option<u16>,
}

/// A server that's listening.
pub struct Running {
    /// Where it listens.
    pub addr: SocketAddr,
    /// State every connection shares (the connection count, for idle shutdown).
    pub shared: Arc<Shared>,
    /// Finishes when the server stops.
    pub task: tokio::task::JoinHandle<()>,
}

#[derive(Clone)]
struct App {
    shared: Arc<Shared>,
    origins: Arc<Vec<String>>,
}

/// Start listening on 127.0.0.1 and serve until the task is dropped or aborted.
pub async fn start(hub: Arc<Hub>, options: Options) -> Result<Running, ServerError> {
    let listener = tokio::net::TcpListener::bind((LOCAL_HOST, options.port))
        .await
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::AddrInUse {
                ServerError::PortInUse { port: options.port }
            } else {
                ServerError::Listen(e)
            }
        })?;
    let addr = listener.local_addr().map_err(ServerError::Listen)?;
    let mut origins: Vec<String> = vec![
        format!("http://{LOCAL_HOST}:{}", addr.port()),
        format!("http://localhost:{}", addr.port()),
    ];
    origins.extend(DESKTOP_ORIGINS.iter().map(|o| (*o).to_owned()));
    origins.extend(options.extra_origins);
    let remote = remote::Remote::new(hub.state().data_dir.clone(), options.remote_port);
    let app = App {
        shared: Arc::new(Shared {
            hub,
            token: options.token,
            version: options.version,
            clients: std::sync::atomic::AtomicUsize::new(0),
            stop: tokio::sync::Notify::new(),
            remote: Arc::new(remote),
            signins: signin::SignIns::default(),
        }),
        origins: Arc::new(origins),
    };
    let shared = app.shared.clone();
    // The phone listener follows `[remote] enabled` until the server stops.
    let supervisor = remote::supervise(shared.clone());
    let hosts = Arc::new(vec![
        format!("{LOCAL_HOST}:{}", addr.port()),
        format!("localhost:{}", addr.port()),
    ]);
    let mut router = Router::new()
        .route(WS_PATH, get(upgrade))
        .route(HEALTH_PATH, get(health))
        .route(
            FILES_PATH,
            post(upload).layer(DefaultBodyLimit::max(MAX_UPLOAD_BYTES)),
        )
        .route(FILE_PATH, get(download))
        .route(SIGNIN_PATH, post(sign_in))
        .with_state(app);
    if options.serve_web {
        router = router.merge(web::router());
    }
    let router = router.layer(middleware::from_fn(move |req: Request, next: Next| {
        let hosts = hosts.clone();
        async move {
            if host_allowed(req.headers(), &hosts) {
                next.run(req).await
            } else {
                (StatusCode::MISDIRECTED_REQUEST, "Unknown host.").into_response()
            }
        }
    }));
    let task = tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, router).await {
            tracing::error!(%e, "server stopped");
        }
        supervisor.abort();
    });
    tracing::info!(%addr, "engine listening");
    Ok(Running { addr, shared, task })
}

async fn upload(State(app): State<App>, headers: HeaderMap, body: axum::body::Bytes) -> Response {
    if !origin_allowed(&headers, &app.origins) {
        return (
            StatusCode::FORBIDDEN,
            "This page can't send files to Thrall.",
        )
            .into_response();
    }
    let Some(state) = bearer(&app, &headers).await else {
        return uploads::refuse(
            StatusCode::UNAUTHORIZED,
            "Thrall didn't accept this upload's access token.",
        );
    };
    uploads::save(&headers, &body, uploads::folder(state.my_dir().as_deref()))
}

/// The engine acting for whoever a request's `Authorization: Bearer` token says (the access
/// token: the owner; a device token: its person).
async fn bearer(app: &App, headers: &HeaderMap) -> Option<thrall_agent::AppState> {
    let given = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or_default();
    match signin::local_person(&app.shared, given).await {
        Ok(found) => found.map(|(person, _)| app.shared.hub.state().for_user(&person.id)),
        Err(e) => {
            tracing::warn!(%e, "couldn't check a request's token");
            None
        }
    }
}

/// What a browser sends to sign in with a one-time code.
#[derive(serde::Deserialize)]
struct SignIn {
    code: String,
    #[serde(default)]
    name: String,
}

/// `POST /v1/signin`: trade a sign-in code for a device token.
async fn sign_in(State(app): State<App>, headers: HeaderMap, body: axum::body::Bytes) -> Response {
    if !origin_allowed(&headers, &app.origins) {
        return uploads::refuse(StatusCode::FORBIDDEN, "This page can't sign in to Thrall.");
    }
    let Ok(asked) = serde_json::from_slice::<SignIn>(&body) else {
        return uploads::refuse(StatusCode::BAD_REQUEST, "That isn't a sign-in.");
    };
    match signin::redeem(&app.shared, asked.code.trim(), &asked.name).await {
        Ok(Some((token, person))) => (
            [(header::CACHE_CONTROL, "no-store")],
            axum::Json(serde_json::json!({
                "token": token,
                "person": { "id": person.id, "name": person.name },
            })),
        )
            .into_response(),
        Ok(None) => uploads::refuse(
            StatusCode::UNAUTHORIZED,
            "That sign-in link has been used or has run out. Ask for a new one.",
        ),
        Err(e) => {
            tracing::warn!(%e, "couldn't sign a browser in");
            uploads::refuse(StatusCode::INTERNAL_SERVER_ERROR, "Couldn't sign in.")
        }
    }
}

async fn download(
    State(app): State<App>,
    axum::extract::Path(id): axum::extract::Path<String>,
    headers: HeaderMap,
) -> Response {
    if !origin_allowed(&headers, &app.origins) {
        return (
            StatusCode::FORBIDDEN,
            "This page can't read Thrall's files.",
        )
            .into_response();
    }
    let Some(state) = bearer(&app, &headers).await else {
        return uploads::refuse(
            StatusCode::UNAUTHORIZED,
            "Thrall didn't accept this request's access token.",
        );
    };
    uploads::send(uploaded_file(&state, &id)).await
}

async fn health(State(app): State<App>) -> impl IntoResponse {
    (
        [(header::CACHE_CONTROL, "no-store")],
        axum::Json(serde_json::json!({ "ok": true, "version": app.shared.version })),
    )
}

/// Only requests addressed to this engine by its own name. A page on another site that rebinds
/// its domain to 127.0.0.1 still sends its own name as `Host`, so it's turned away.
fn host_allowed(headers: &HeaderMap, allowed: &[String]) -> bool {
    headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .is_some_and(|h| allowed.iter().any(|a| a.eq_ignore_ascii_case(h)))
}

/// Browsers send `Origin`; other clients (terminal, desktop shell, scripts) don't. A browser
/// connection is accepted only from Thrall's own pages.
fn origin_allowed(headers: &HeaderMap, allowed: &[String]) -> bool {
    match headers.get(header::ORIGIN) {
        None => true,
        Some(value) => value
            .to_str()
            .map(|o| {
                allowed
                    .iter()
                    .any(|a| a.eq_ignore_ascii_case(o.trim_end_matches('/')))
            })
            .unwrap_or(false),
    }
}

async fn upgrade(State(app): State<App>, headers: HeaderMap, ws: WebSocketUpgrade) -> Response {
    if !origin_allowed(&headers, &app.origins) {
        tracing::warn!(origin = ?headers.get(header::ORIGIN), "refused a connection from another web page");
        return (StatusCode::FORBIDDEN, "This page can't connect to Thrall.").into_response();
    }
    let shared = app.shared.clone();
    ws.max_message_size(MAX_FRAME_BYTES * 2)
        .on_upgrade(move |socket| connection::serve(socket, shared))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    #[test]
    fn hosts_are_checked() {
        let allowed = vec!["127.0.0.1:7417".to_owned()];
        let mut h = HeaderMap::new();
        assert!(!host_allowed(&h, &allowed), "no Host header");
        h.insert(header::HOST, HeaderValue::from_static("127.0.0.1:7417"));
        assert!(host_allowed(&h, &allowed));
        h.insert(header::HOST, HeaderValue::from_static("evil.example:7417"));
        assert!(!host_allowed(&h, &allowed));
    }

    #[test]
    fn origins_are_checked() {
        let allowed = vec!["http://127.0.0.1:7417".to_owned()];
        let mut h = HeaderMap::new();
        assert!(origin_allowed(&h, &allowed), "no Origin: not a browser");
        h.insert(
            header::ORIGIN,
            HeaderValue::from_static("http://127.0.0.1:7417"),
        );
        assert!(origin_allowed(&h, &allowed));
        h.insert(
            header::ORIGIN,
            HeaderValue::from_static("https://evil.example"),
        );
        assert!(!origin_allowed(&h, &allowed));
        h.insert(
            header::ORIGIN,
            HeaderValue::from_static("http://127.0.0.1:9999"),
        );
        assert!(!origin_allowed(&h, &allowed));
    }
}
