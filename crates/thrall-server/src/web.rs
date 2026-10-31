//! The web app, built into the binary. The page gets the access token written into it as it's
//! served; other web pages can't read it (browsers keep cross-site responses private) and the
//! Host check in `lib.rs` stops DNS-rebinding pages from posing as this one.

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use rust_embed::RustEmbed;

use crate::constants::{ASSETS_PREFIX, INDEX_FILE, TOKEN_PLACEHOLDER};

#[derive(RustEmbed)]
#[folder = "$OUT_DIR/web"]
struct Assets;

/// Security headers on every page: no framing, no sniffing, connections only back to the engine.
const CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; \
                   img-src 'self' data: blob:; media-src 'self' blob:; font-src 'self'; connect-src 'self'; \
                   frame-ancestors 'none'; base-uri 'none'; form-action 'none'";

#[derive(Clone)]
struct Web {
    index: Arc<String>,
}

/// Routes that serve the web app.
pub fn router(token: &str) -> Router {
    let index = Assets::get(INDEX_FILE)
        .map(|f| String::from_utf8_lossy(&f.data).replace(TOKEN_PLACEHOLDER, token))
        .unwrap_or_default();
    Router::new()
        .route("/", get(index_page))
        .route("/{*path}", get(asset))
        .with_state(Web {
            index: Arc::new(index),
        })
}

fn secured(mut response: Response) -> Response {
    let h = response.headers_mut();
    h.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(CSP),
    );
    h.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    h.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    h.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    response
}

async fn index_page(State(web): State<Web>) -> Response {
    secured(
        (
            [
                (header::CONTENT_TYPE, "text/html; charset=utf-8"),
                (header::CACHE_CONTROL, "no-store"),
            ],
            web.index.as_str().to_owned(),
        )
            .into_response(),
    )
}

async fn asset(State(web): State<Web>, Path(path): Path<String>) -> Response {
    match Assets::get(&path) {
        Some(file) if path != INDEX_FILE => {
            let cache = if path.starts_with(ASSETS_PREFIX) {
                // Built files have content hashes in their names.
                "public, max-age=31536000, immutable"
            } else {
                "no-cache"
            };
            let mime = file.metadata.mimetype().to_owned();
            secured(
                Response::builder()
                    .header(header::CONTENT_TYPE, mime)
                    .header(header::CACHE_CONTROL, cache)
                    .body(Body::from(file.data.into_owned()))
                    .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response()),
            )
        }
        // The app has one page: anything else that isn't a file is that page.
        _ if !path.starts_with(ASSETS_PREFIX) => index_page(State(web)).await,
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}
