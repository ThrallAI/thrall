//! This PC as a companion of another Thrall: `thrall join` finds it and asks to join (the owner
//! lets it in there), `thrall runner` stays connected and does this PC's share of the work, and
//! `thrall leave` forgets the joined Thrall.

use std::process::ExitCode;
use std::sync::Arc;

use thrall_core::Paths;
use thrall_server::companion_bridge;
use thrall_server::companion_client::{self, Joined};
use thrall_tools::companion::{Base, Runner};

use thrall_protocol::{ClientKind, Request};

use crate::constants::{
    COMPANION_FIND_WAIT, COMPANION_RETRY_MAX, COMPANION_RETRY_MIN, COMPANION_STOP_POLL,
    COMPANION_STOP_TRIES, OPEN_CLIENT_NAME,
};
use crate::{attach, discover};

/// This computer's name, as the owner sees it when it asks to join.
fn this_pc() -> String {
    thrall_server::remote::computer_name()
}

/// `thrall join [address]`: find Thrall on the network (or use `address`), ask to join, show the
/// code, and wait for the owner. With `find`, only list what's there. With `machine`, print plain
/// lines for the desktop app instead of sentences.
pub async fn join(
    paths: &Paths,
    address: Option<String>,
    find: bool,
    machine: bool,
) -> anyhow::Result<ExitCode> {
    let say = |human: &str, line: &str| {
        if machine {
            println!("{line}");
        } else {
            println!("{human}");
        }
    };
    if find {
        let found = companion_client::find(COMPANION_FIND_WAIT).await;
        for f in &found {
            say(
                &format!("{} ({})", f.name, f.address),
                &format!("found {}\t{}", f.address, f.name),
            );
        }
        if found.is_empty() && !machine {
            println!("No Thrall on this network lets PCs join.");
        }
        return Ok(ExitCode::SUCCESS);
    }
    if let Some(joined) = Joined::load(&paths.home) {
        say(
            &format!(
                "This PC already joined Thrall on {} ({}). Run `thrall leave` first to join another.",
                joined.name, joined.address
            ),
            "joined",
        );
        return Ok(ExitCode::SUCCESS);
    }
    let (name, address) = match address {
        Some(a) => {
            let address = companion_client::with_port(&a);
            let name = a.rsplit_once(':').map_or(a.as_str(), |(h, _)| h).to_owned();
            (name, address)
        }
        None => {
            if !machine {
                println!("Looking for Thrall on this network…");
            }
            let found = companion_client::find(COMPANION_FIND_WAIT).await;
            match found.as_slice() {
                [] => {
                    say(
                        "No Thrall here lets PCs join. On the Thrall PC: Settings → PCs → Let PCs \
                         on this network connect. Over Tailscale, give its name: thrall join <name>",
                        "error No Thrall on this network lets PCs join.",
                    );
                    return Ok(ExitCode::FAILURE);
                }
                [one] => (one.name.clone(), one.address.clone()),
                many => {
                    if !machine {
                        println!("Found more than one; pick with `thrall join <address>`:");
                    }
                    for f in many {
                        say(
                            &format!("  {} ({})", f.name, f.address),
                            &format!("found {}\t{}", f.address, f.name),
                        );
                    }
                    return Ok(ExitCode::FAILURE);
                }
            }
        }
    };
    if !machine {
        println!("Asking Thrall on {name} to let {} join…", this_pc());
    }
    let answer = companion_client::ask(&address, &this_pc(), |code| {
        if machine {
            println!("code {code}");
        } else {
            println!();
            println!("  Code: {code}");
            println!();
            println!("Check it's the code on the Thrall PC, where the owner lets this PC in.");
        }
    })
    .await;
    match answer {
        Ok(Some(mut joined)) => {
            joined.name = name;
            joined.save(&paths.home)?;
            say(
                "Joined. Start the companion with `thrall runner` (the desktop app does it by itself).",
                "joined",
            );
            Ok(ExitCode::SUCCESS)
        }
        Ok(None) => {
            say("The owner said no.", "declined");
            Ok(ExitCode::FAILURE)
        }
        Err(why) => {
            say(&why, &format!("error {why}"));
            Ok(ExitCode::FAILURE)
        }
    }
}
/// Stop this PC's companion runner if it's running. False when what's running is a real engine
/// (it isn't stopped) or the runner didn't stop.
pub async fn stop_runner(paths: &Paths) -> bool {
    let Some(found) = discover::running(paths).await else {
        return true;
    };
    if !discover::is_companion(&found).await {
        return false;
    }
    if let Ok(token) = attach::token(paths)
        && let Ok(c) = thrall_tui::client::connect_as(
            &found.ws_url,
            &token,
            ClientKind::Headless,
            OPEN_CLIENT_NAME,
        )
        .await
    {
        let _ = c.client.request(Request::EngineStop {}).await;
    }
    for _ in 0..COMPANION_STOP_TRIES {
        tokio::time::sleep(COMPANION_STOP_POLL).await;
        if discover::running(paths).await.is_none() {
            return true;
        }
    }
    false
}

/// `thrall leave`: stop being a companion: the runner stops and the joined Thrall is forgotten
/// (the owner can remove this PC there too).
pub async fn leave(paths: &Paths) -> anyhow::Result<ExitCode> {
    if !stop_runner(paths).await {
        anyhow::bail!(
            "Thrall is running here and didn't stop. Quit it (tray → Quit), then try again."
        );
    }
    Joined::forget(&paths.home)?;
    println!("This PC isn't a companion any more.");
    Ok(ExitCode::SUCCESS)
}

/// `thrall runner`: stay connected to the joined Thrall and run what its sessions do on this PC,
/// reconnecting when the link drops.
pub async fn runner(paths: &Paths, work_folder: &str) -> anyhow::Result<ExitCode> {
    let Some(joined) = Joined::load(&paths.home) else {
        println!("This PC hasn't joined a Thrall. Run `thrall join` first.");
        return Ok(ExitCode::FAILURE);
    };
    let home = Paths::work_folder(work_folder)
        .ok_or_else(|| anyhow::anyhow!("couldn't work out this PC's Thrall folder"))?;
    std::fs::create_dir_all(&home)?;
    let base = Base {
        tools: thrall_tools::tools::core(),
        store: Arc::new(thrall_store::SqliteStore::open_in_memory().await?),
        http: thrall_tools::http_client()?,
        data_dir: Some(paths.home.clone()),
        browser: Arc::new(thrall_tools::browser::Browsers::default()),
        home: Some(home.clone()),
    };
    let runner = Arc::new(Runner::new(base));
    let home = home.to_string_lossy().into_owned();
    let version = env!("CARGO_PKG_VERSION");
    // The desktop app (and the terminal) find it like an engine, and use Thrall through it.
    let bridge = companion_bridge::start(joined.clone(), paths, version).await?;
    let work = async {
        let mut wait = COMPANION_RETRY_MIN;
        loop {
            match companion_client::run_once(&joined, runner.clone(), &home, version).await {
                Ok(()) => {
                    tracing::info!("the link to Thrall closed; reconnecting");
                    wait = COMPANION_RETRY_MIN;
                }
                // Not let in any more: nothing to gain by trying often.
                Err(e) if companion_client::is_not_joined(&e) => {
                    tracing::warn!(%e, "the Thrall this PC joined doesn't let it in");
                    wait = COMPANION_RETRY_MAX;
                }
                Err(e) => {
                    tracing::warn!(%e, "couldn't stay connected to Thrall");
                    wait = (wait * 2).min(COMPANION_RETRY_MAX);
                }
            }
            tokio::time::sleep(wait).await;
        }
    };
    tokio::select! {
        () = work => {}
        () = bridge.stop.notified() => tracing::info!("asked to stop"),
        _ = tokio::signal::ctrl_c() => {}
    }
    companion_bridge::stopped(paths);
    Ok(ExitCode::SUCCESS)
}
