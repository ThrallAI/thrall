//! The bench's page and its JSON API, on 127.0.0.1 only. Requests must name this machine as
//! their host (no other site can reach it through the browser), and everything that starts or
//! changes something takes JSON (which a page from another site can't send here).

use std::sync::Arc;

use axum::extract::{Path, Request, State};
use axum::http::{StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::json;

use crate::app::Bench;
use crate::model::{Preset, RunRequest};

/// The page.
const PAGE: &str = include_str!("page.html");

/// The bench's routes.
pub fn router(bench: Arc<Bench>, port: u16) -> Router {
    Router::new()
        .route("/", get(|| async { Html(PAGE) }))
        .route("/favicon.ico", get(|| async { StatusCode::NO_CONTENT }))
        .route("/api/tasks", get(tasks))
        .route("/api/models", get(models))
        .route("/api/presets", get(presets).post(save_preset))
        .route("/api/runs", get(history).post(start))
        .route("/api/runs/{id}", get(run))
        .route("/api/runs/{id}/stop", post(stop))
        .layer(middleware::from_fn(move |req, next| {
            only_this_machine(req, next, port)
        }))
        .with_state(bench)
}

async fn only_this_machine(req: Request, next: Next, port: u16) -> Response {
    let host = req
        .headers()
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or_default();
    if host == format!("127.0.0.1:{port}") || host == format!("localhost:{port}") {
        next.run(req).await
    } else {
        (
            StatusCode::FORBIDDEN,
            "The bench only answers this computer.",
        )
            .into_response()
    }
}

fn refused(why: String) -> Response {
    (StatusCode::BAD_REQUEST, Json(json!({ "error": why }))).into_response()
}

async fn tasks(State(bench): State<Arc<Bench>>) -> Response {
    Json(bench.tasks()).into_response()
}

async fn models(State(bench): State<Arc<Bench>>) -> Response {
    match bench.models().await {
        Ok(models) => Json(models).into_response(),
        Err(e) => refused(format!("couldn't get OpenRouter's models: {e}")),
    }
}

async fn presets(State(bench): State<Arc<Bench>>) -> Response {
    Json(bench.presets()).into_response()
}

async fn save_preset(State(bench): State<Arc<Bench>>, Json(preset): Json<Preset>) -> Response {
    match bench.save_preset(preset) {
        Ok(()) => Json(json!({ "ok": true })).into_response(),
        Err(e) => refused(e),
    }
}

async fn history(State(bench): State<Arc<Bench>>) -> Response {
    Json(bench.history()).into_response()
}

async fn start(State(bench): State<Arc<Bench>>, Json(request): Json<RunRequest>) -> Response {
    match bench.start(request) {
        Ok(id) => Json(json!({ "id": id })).into_response(),
        Err(e) => refused(e),
    }
}

async fn run(State(bench): State<Arc<Bench>>, Path(id): Path<String>) -> Response {
    match bench.run(&id) {
        Some(run) => Json(run).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "no such run" })),
        )
            .into_response(),
    }
}

async fn stop(State(bench): State<Arc<Bench>>, Path(id): Path<String>) -> Response {
    Json(json!({ "stopped": bench.stop(&id) })).into_response()
}
