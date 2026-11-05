//! The `thrall` program: the terminal app (`thrall`, `thrall -c`, `thrall -r`), plus `thrall run`,
//! `thrall engine`, `thrall open`, `thrall key`, `thrall sessions` and `thrall paths`.

mod attach;
mod cli;
mod companion;
mod constants;
mod discover;
mod engine;
mod key;
mod logging;
mod open;
mod remove;
mod run;
mod serve;
mod sessions;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use thrall_core::constants::HOME_ENV_VAR;
use thrall_core::{Config, Paths};
use thrall_tui::Launch;

use crate::cli::{Cli, Command, KeyCommand, RunArgs};
use crate::constants::{EXIT_FAILURE, EXIT_SETUP_NEEDED};

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    match dispatch(cli).await {
        Ok(code) => code,
        Err(e) => {
            tracing::error!(error = %e, "command failed");
            eprintln!("{e}");
            ExitCode::from(EXIT_FAILURE)
        }
    }
}

async fn dispatch(cli: Cli) -> anyhow::Result<ExitCode> {
    let paths = Paths::resolve(std::env::var_os(HOME_ENV_VAR).map(PathBuf::from))?;
    paths.ensure_dirs()?;
    let config = Config::load_or_create(&paths.config_file)?;
    let _log_guard = logging::init(&paths, &config)?;
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "thrall started");

    if let Some(message) = cli.print.clone() {
        let args = RunArgs {
            prompt: vec![message],
            continue_last: cli.continue_last,
            session: None,
            json: cli.json,
            mode: cli.mode.clone(),
        };
        return run::execute(&paths, config, args).await;
    }

    match cli.command {
        None => {
            // The terminal is a client of the one engine: attach to it, starting it if needed.
            let Some(found) = attach::engine(&paths, &config).await? else {
                return Ok(ExitCode::from(EXIT_SETUP_NEEDED));
            };
            let connected =
                thrall_tui::client::connect(&found.ws_url, &attach::token(&paths)?).await?;
            let mine = env!("CARGO_PKG_VERSION");
            let notice = (connected.engine.version != mine).then(|| {
                format!(
                    "Thrall's engine is version {} and this terminal is {mine}. Stop the engine \
                     (close the desktop app, or Ctrl+C in `thrall engine`) and start Thrall \
                     again to finish updating.",
                    connected.engine.version
                )
            });
            let launch = Launch {
                resume: cli.resume.clone().filter(|id| !id.is_empty()),
                continue_last: cli.continue_last,
                pick: cli.resume.as_deref() == Some(""),
                workspace: std::env::current_dir()
                    .ok()
                    .map(|p| p.to_string_lossy().into_owned()),
                notice,
            };
            thrall_tui::run(connected, launch).await?;
            Ok(ExitCode::SUCCESS)
        }
        Some(Command::Run(args)) => run::execute(&paths, config, args).await,
        Some(Command::Engine(args)) => serve::run(&paths, config, args).await,
        Some(Command::Key(KeyCommand::Set { name })) => key::set(&name),
        Some(Command::Key(KeyCommand::Remove { name })) => key::remove(&name),
        Some(Command::Key(KeyCommand::Status)) => key::status(&config),
        Some(Command::Sessions { limit }) => sessions::list(&paths, limit).await,
        Some(Command::Open) => open::run(&paths, &config).await,
        Some(Command::Join { address }) => companion::join(&paths, address).await,
        Some(Command::Runner) => companion::runner(&paths, &config.folders.new_in).await,
        Some(Command::Leave) => companion::leave(&paths),
        Some(Command::RemoveEverything) => remove::run(&paths, &config).await,
        Some(Command::Paths) => {
            println!("{} {}", constants::APP_NAME, env!("CARGO_PKG_VERSION"));
            println!("Data folder: {}", paths.home.display());
            println!("Config:      {}", paths.config_file.display());
            println!("Database:    {}", paths.db_file.display());
            println!("Logs:        {}", paths.logs_dir.display());
            Ok(ExitCode::SUCCESS)
        }
    }
}
