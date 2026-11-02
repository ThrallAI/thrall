//! `thrall engine` (also `thrall serve`): the one engine every client connects to. It listens on
//! 127.0.0.1, writes `engine.json` so clients can find it, and runs until stopped.

use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Instant;

use thrall_agent::{Hub, Rebuild};
use thrall_core::{Config, Paths};
use thrall_server::constants::{DEFAULT_PORT, LOCAL_HOST};
use thrall_server::{EngineFile, Options, token};

use crate::cli::EngineArgs;
use crate::constants::{APP_NAME, EXIT_SETUP_NEEDED, IDLE_CHECK, IDLE_EXIT, PORT_ENV_VAR};
use crate::{discover, engine};

/// Run the engine until Ctrl+C.
pub async fn run(paths: &Paths, config: Config, args: EngineArgs) -> anyhow::Result<ExitCode> {
    if let Some(found) = discover::running(paths).await {
        println!(
            "{APP_NAME} is already running (version {}). Web app: {}",
            found.version, found.web_url
        );
        return Ok(ExitCode::SUCCESS);
    }
    let Some(state) = engine::build(paths, config).await? else {
        return Ok(ExitCode::from(EXIT_SETUP_NEEDED));
    };
    let hub = Hub::new(state, Some(rebuild(paths.clone())));
    hub.end_full_auto_from_before().await;
    let version = env!("CARGO_PKG_VERSION");
    let running = thrall_server::start(
        hub.clone(),
        Options {
            port: args
                .port
                .or_else(|| std::env::var(PORT_ENV_VAR).ok()?.parse().ok())
                .unwrap_or(DEFAULT_PORT),
            token: token::load_or_create(&paths.token_file)?,
            version: version.to_owned(),
            extra_origins: args.allow_origin,
            serve_web: true,
            remote_port: None,
        },
    )
    .await?;
    let file = EngineFile::new(
        LOCAL_HOST,
        running.addr.port(),
        thrall_protocol::constants::WS_PATH,
        version,
    );
    file.write(&paths.engine_file)?;
    println!(
        "{APP_NAME} engine {version} is running. Web app: {}",
        file.web_url
    );
    println!("Press Ctrl+C to stop it.");

    let idle = async {
        if !args.idle_exit {
            return std::future::pending().await;
        }
        let mut quiet_since = Instant::now();
        let mut check = tokio::time::interval(IDLE_CHECK);
        loop {
            check.tick().await;
            // While phones may connect, the engine stays up for them.
            let busy = running.shared.clients.load(Ordering::SeqCst) > 0
                || hub.state().any_running()
                || hub.state().config.remote.enabled;
            if busy {
                quiet_since = Instant::now();
            } else if quiet_since.elapsed() >= IDLE_EXIT {
                tracing::info!("no clients and nothing running; stopping");
                return;
            }
        }
    };
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = idle => {}
        _ = running.shared.stop.notified() => {}
        _ = running.task => {}
    }
    EngineFile::remove_if_mine(&paths.engine_file);
    hub.state().shut_down();
    // The browser is a separate process: close it, or it outlives the engine.
    hub.state().browsers.close_all().await;
    // So are programs Thrall left running in the background.
    hub.state().stop_background().await;
    println!("{APP_NAME} engine stopped.");
    Ok(ExitCode::SUCCESS)
}

/// How the hub rebuilds the engine when Thrall restarts itself: a fresh read of the config file.
pub fn rebuild(paths: Paths) -> Rebuild {
    let paths = Arc::new(paths);
    Box::new(move || {
        let paths = paths.clone();
        Box::pin(async move {
            let config = Config::load_or_create(&paths.config_file).map_err(|e| e.to_string())?;
            engine::build(&paths, config)
                .await
                .map_err(|e| e.to_string())?
                .ok_or_else(|| "no provider key is set".to_owned())
        })
    })
}
