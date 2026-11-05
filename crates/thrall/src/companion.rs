//! This PC as a companion of another Thrall: `thrall join` finds it and asks to join (the owner
//! lets it in there), `thrall runner` stays connected and does this PC's share of the work, and
//! `thrall leave` forgets the joined Thrall.

use std::process::ExitCode;
use std::sync::Arc;

use thrall_core::Paths;
use thrall_server::companion_client::{self, Joined};
use thrall_tools::companion::{Base, Runner};

use crate::constants::{COMPANION_FIND_WAIT, COMPANION_RETRY_MAX, COMPANION_RETRY_MIN};

/// This computer's name, as the owner sees it when it asks to join.
fn this_pc() -> String {
    thrall_server::remote::computer_name()
}

/// `thrall join [address]`: find Thrall on the network (or use `address`), ask to join, show the
/// code, and wait for the owner.
pub async fn join(paths: &Paths, address: Option<String>) -> anyhow::Result<ExitCode> {
    if let Some(joined) = Joined::load(&paths.home) {
        println!(
            "This PC already joined Thrall on {} ({}). Run `thrall leave` first to join another.",
            joined.name, joined.address
        );
        return Ok(ExitCode::FAILURE);
    }
    let (name, address) = match address {
        Some(a) => (a.clone(), companion_client::with_port(&a)),
        None => {
            println!("Looking for Thrall on this network…");
            let found = companion_client::find(COMPANION_FIND_WAIT).await;
            match found.as_slice() {
                [] => {
                    println!(
                        "No Thrall here lets PCs join. On the Thrall PC: Settings → PCs → Let PCs \
                         on this network connect. Over Tailscale, give its name: thrall join <name>"
                    );
                    return Ok(ExitCode::FAILURE);
                }
                [one] => (one.name.clone(), one.address.clone()),
                many => {
                    println!("Found more than one; pick with `thrall join <address>`:");
                    for f in many {
                        println!("  {} ({})", f.name, f.address);
                    }
                    return Ok(ExitCode::FAILURE);
                }
            }
        }
    };
    println!("Asking Thrall on {name} to let {} join…", this_pc());
    let answer = companion_client::ask(&address, &this_pc(), |code| {
        println!();
        println!("  Code: {code}");
        println!();
        println!("Check it's the code on the Thrall PC, where the owner lets this PC in.");
    })
    .await
    .map_err(anyhow::Error::msg)?;
    match answer {
        Some(mut joined) => {
            joined.name = name;
            joined.save(&paths.home)?;
            println!(
                "Joined. Start the companion with `thrall runner` (the desktop app does it by itself)."
            );
            Ok(ExitCode::SUCCESS)
        }
        None => {
            println!("The owner said no.");
            Ok(ExitCode::FAILURE)
        }
    }
}

/// `thrall leave`: forget the joined Thrall (the owner can remove this PC there too).
pub fn leave(paths: &Paths) -> anyhow::Result<ExitCode> {
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
    };
    let runner = Arc::new(Runner::new(base));
    let home = home.to_string_lossy().into_owned();
    let version = env!("CARGO_PKG_VERSION");
    let mut wait = COMPANION_RETRY_MIN;
    loop {
        match companion_client::run_once(&joined, runner.clone(), &home, version).await {
            Ok(()) => {
                tracing::info!("the link to Thrall closed; reconnecting");
                wait = COMPANION_RETRY_MIN;
            }
            Err(e) => {
                tracing::warn!(%e, "couldn't stay connected to Thrall");
                wait = (wait * 2).min(COMPANION_RETRY_MAX);
            }
        }
        tokio::time::sleep(wait).await;
    }
}
