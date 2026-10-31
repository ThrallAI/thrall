//! thrall-bench: a developer tool, never shipped. Runs the same prompt through Thrall's engine in
//! side-by-side lanes (one model alone, or split mode's Thinker + Doer) and measures each: time,
//! model calls, tokens in/out/cached, cost per model, and whether the task's check passes.
//!
//! Each lane has its own throwaway database and its own copy of the task, under Thrall's data
//! folder in `bench/runs/`; the real Thrall (its database, chats and files) is never touched, and
//! it doesn't need to be stopped. Lanes use the same OpenRouter key as Thrall: runs cost money,
//! so each lane is stopped at its budget.
//!
//! Usage (from the repo root): `cargo run -p thrall-bench -- [--port 7425] [--tasks bench/tasks]`,
//! then open http://127.0.0.1:7425/.

mod app;
mod constants;
mod lane;
mod model;
mod web;

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, bail};
use thrall_core::constants::HOME_ENV_VAR;
use thrall_core::{Config, Paths, ProviderKind, secrets};

use crate::constants::{BENCH_DIR, BENCH_PORT, DEFAULT_TASKS_DIR, KEY_HELP, RUNS_DIR};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn,thrall_bench=info")),
        )
        .init();
    let mut port = BENCH_PORT;
    let mut tasks_dir = PathBuf::from(DEFAULT_TASKS_DIR);
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--port" => port = args.next().context("--port needs a number")?.parse()?,
            "--tasks" => tasks_dir = PathBuf::from(args.next().context("--tasks needs a folder")?),
            other => bail!("unknown option {other} (use --port and --tasks)"),
        }
    }

    let paths = Paths::resolve(std::env::var_os(HOME_ENV_VAR).map(PathBuf::from))?;
    let base = if paths.config_file.exists() {
        Config::load_or_create(&paths.config_file)?
    } else {
        Config::default()
    };
    let provider = ProviderKind::OpenRouter;
    let api_key = match std::env::var(provider.key_env_var()) {
        Ok(key) if !key.trim().is_empty() => key.trim().to_owned(),
        _ => secrets::get_key(provider)?.context(KEY_HELP)?,
    };
    let dir = paths.home.join(BENCH_DIR);
    std::fs::create_dir_all(dir.join(RUNS_DIR))?;

    let bench = Arc::new(app::Bench::new(
        dir.clone(),
        tasks_dir.clone(),
        base,
        api_key,
    ));
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
    println!("Thrall bench: http://127.0.0.1:{port}/");
    println!("  tasks from {}", tasks_dir.display());
    println!("  runs saved in {}", dir.join(RUNS_DIR).display());
    axum::serve(listener, web::router(bench, port)).await?;
    Ok(())
}
