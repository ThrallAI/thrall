//! `thrall remove-everything`: take away what Thrall keeps on this computer, for someone who's
//! done with it. The data folder (settings, sessions, memory, logs), Thrall's keys in the system
//! keychain and starting at sign-in all go; the working folder, with the files Thrall made for
//! the person, stays. The app itself is uninstalled the usual way. Only with Thrall stopped, and
//! only once the person types the confirmation.

use std::io::BufRead;
use std::process::ExitCode;

use thrall_core::{Config, Paths, secrets, startup};

use crate::constants::REMOVE_CONFIRM;
use crate::discover;

/// `thrall remove-everything`.
pub async fn run(paths: &Paths, config: &Config) -> anyhow::Result<ExitCode> {
    if discover::running(paths).await.is_some() {
        anyhow::bail!(
            "Thrall is running. Quit it first (the tray icon → Quit Thrall → Quit and stop \
             Thrall), then run this again."
        );
    }
    let keys = secrets::stored_names();
    println!("This removes everything Thrall keeps on this computer:");
    println!(
        "  - the data folder {} (settings, sessions, memory, logs)",
        paths.home.display()
    );
    match &keys {
        Some(names) if names.is_empty() => println!("  - no keys are stored"),
        Some(names) => println!(
            "  - {} key(s) in the keychain: {}",
            names.len(),
            names.join(", ")
        ),
        None => println!(
            "  - (your keychain app lists Thrall's keys under \"Thrall\": remove them there)"
        ),
    }
    println!("  - starting Thrall when you sign in");
    if let Some(work) = Paths::work_folder(&config.folders.new_in) {
        println!(
            "It keeps your working folder, {}, and the files in it.",
            work.display()
        );
    }
    println!("To remove the app itself, uninstall it afterwards as usual.");
    println!();
    println!("Type {REMOVE_CONFIRM} to go ahead (anything else stops):");
    let mut answer = String::new();
    std::io::stdin().lock().read_line(&mut answer)?;
    if answer.trim() != REMOVE_CONFIRM {
        println!("Nothing was removed.");
        return Ok(ExitCode::SUCCESS);
    }
    if let Err(e) = startup::set(None) {
        println!("Couldn't stop Thrall starting at sign-in: {e}");
    }
    for name in keys.iter().flatten() {
        if let Err(e) = secrets::delete_named(name) {
            println!("Couldn't remove the {name} key: {e}");
        }
    }
    if paths.home.is_dir() {
        std::fs::remove_dir_all(&paths.home).map_err(|e| {
            anyhow::anyhow!(
                "Couldn't remove {}: {e}. Close anything using it and try again.",
                paths.home.display()
            )
        })?;
    }
    println!("Done. Thrall's data and keys are gone.");
    Ok(ExitCode::SUCCESS)
}
