//! Command-line arguments.

use clap::{Args, Parser, Subcommand};
use thrall_core::ProviderKind;

use crate::constants::DEFAULT_SESSION_LIST_LIMIT;

/// Thrall: an AI agent that works on your computer. Run it on its own to open the terminal app.
#[derive(Debug, Parser)]
#[command(name = "thrall", version, about)]
pub struct Cli {
    /// What to do (leave out to open the terminal app).
    #[command(subcommand)]
    pub command: Option<Command>,
    /// Open the terminal app on your most recent session.
    #[arg(short = 'c', long = "continue", conflicts_with = "resume")]
    pub continue_last: bool,
    /// Open the terminal app on a session; without an id, pick from a list.
    #[arg(short = 'r', long, num_args = 0..=1, default_missing_value = "")]
    pub resume: Option<String>,
    /// Headless: send one message, print the reply, exit (like `thrall run`).
    #[arg(
        short = 'p',
        long = "print",
        value_name = "MESSAGE",
        conflicts_with = "resume"
    )]
    pub print: Option<String>,
    /// With -p: print one JSON object (reply, status, steps, usage) instead of text.
    #[arg(long, requires = "print")]
    pub json: bool,
    /// With -p: permission mode for this run (ask, auto, plan, full).
    #[arg(long, requires = "print")]
    pub mode: Option<String>,
}

/// Top-level commands.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Send Thrall one message and stream the reply (for scripts and quick questions).
    Run(RunArgs),
    /// Manage AI provider keys (kept in your system keychain, never in a file).
    #[command(subcommand)]
    Key(KeyCommand),
    /// List your sessions, most recent first.
    Sessions {
        /// How many to show.
        #[arg(long, default_value_t = DEFAULT_SESSION_LIST_LIMIT)]
        limit: u32,
    },
    /// Show where Thrall keeps its files.
    Paths,
    /// Open Thrall in your browser, signed in (starts the engine if it isn't running).
    Open,
    /// Remove Thrall's data folder, keys and start-at-sign-in (asks first; your working folder
    /// stays). Uninstall the app itself afterwards.
    RemoveEverything,
    /// Make this PC a companion of Thrall on another PC: find it on this network (or give its
    /// name or address), ask to join, and wait for its owner to let this PC in.
    Join {
        /// The Thrall PC's name or address (`kids-pc`, `192.168.1.20:7420`); found on this
        /// network when left out.
        address: Option<String>,
        /// Only list the Thralls on this network that let PCs join.
        #[arg(long)]
        find: bool,
        /// Lines for the desktop app to read (`found`, `code`, `joined`, `declined`, `error`).
        #[arg(long, hide = true)]
        machine: bool,
    },
    /// Stay connected to the Thrall this PC joined, and do this PC's share of the work.
    Runner,
    /// Stop being a companion: forget the Thrall this PC joined.
    Leave,
    /// Run the engine that the web app, desktop app and terminal connect to.
    #[command(alias = "serve")]
    Engine(EngineArgs),
}

/// `thrall engine`.
#[derive(Debug, Args)]
pub struct EngineArgs {
    /// Port to listen on (on 127.0.0.1 only).
    #[arg(long)]
    pub port: Option<u16>,
    /// Also accept browser connections from this page origin (for developing the web app,
    /// e.g. http://localhost:5173).
    #[arg(long, value_name = "ORIGIN")]
    pub allow_origin: Vec<String>,
    /// Stop after a while with no clients and nothing running (how `thrall` starts the engine
    /// when none is running).
    #[arg(long, hide = true)]
    pub idle_exit: bool,
}

/// `thrall run`.
#[derive(Debug, Args)]
pub struct RunArgs {
    /// Your message. Leave it out to read it from piped input.
    pub prompt: Vec<String>,
    /// Continue your most recent session.
    #[arg(short = 'c', long = "continue", conflicts_with = "session")]
    pub continue_last: bool,
    /// Continue a specific session (see `thrall sessions`).
    #[arg(short, long)]
    pub session: Option<String>,
    /// Print one JSON object (reply, status, steps, usage) instead of text.
    #[arg(long)]
    pub json: bool,
    /// Permission mode for this run: ask, auto, plan or full. Without a terminal to answer on,
    /// anything that would ask is refused.
    #[arg(long)]
    pub mode: Option<String>,
}

/// `thrall key …`.
#[derive(Debug, Subcommand)]
pub enum KeyCommand {
    /// Save a key. You'll be asked to paste it; it won't be shown.
    Set {
        /// `openrouter`, or a name your config refers to as "keychain:<name>" (e.g. `youtube`).
        #[arg(value_parser = parse_key_name)]
        name: String,
    },
    /// Delete a saved key.
    Remove {
        /// The key's name.
        #[arg(value_parser = parse_key_name)]
        name: String,
    },
    /// Show which keys are set (only a short preview of each).
    Status,
}

fn parse_key_name(name: &str) -> Result<String, String> {
    let name = name.trim().to_lowercase();
    if ProviderKind::parse(&name).is_some() || thrall_core::secrets::valid_name(&name) {
        Ok(name)
    } else {
        Err(format!(
            "`{name}` can't be a key name: use letters, digits, - and _ (e.g. youtube)"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn arguments_are_well_formed() {
        Cli::command().debug_assert();
    }

    #[test]
    fn run_joins_words_and_flags() {
        let cli = Cli::try_parse_from(["thrall", "run", "-c", "hello", "there"]).unwrap();
        let Some(Command::Run(args)) = cli.command else {
            panic!("expected run");
        };
        assert!(args.continue_last);
        assert_eq!(args.prompt, vec!["hello", "there"]);
    }

    #[test]
    fn continue_and_session_conflict() {
        assert!(Cli::try_parse_from(["thrall", "run", "-c", "-s", "s_1", "hi"]).is_err());
    }

    #[test]
    fn key_names_are_checked() {
        assert!(Cli::try_parse_from(["thrall", "key", "set", "youtube"]).is_ok());
        assert!(Cli::try_parse_from(["thrall", "key", "set", "bad name!"]).is_err());
    }

    #[test]
    fn print_is_headless_run() {
        let cli = Cli::try_parse_from(["thrall", "-p", "hi", "--json", "--mode", "auto"]).unwrap();
        assert_eq!(cli.print.as_deref(), Some("hi"));
        assert!(cli.json);
        assert!(Cli::try_parse_from(["thrall", "--json"]).is_err());
    }

    #[test]
    fn engine_and_serve_are_the_same() {
        for name in ["engine", "serve"] {
            let cli = Cli::try_parse_from(["thrall", name, "--port", "9000"]).unwrap();
            let Some(Command::Engine(args)) = cli.command else {
                panic!("expected engine");
            };
            assert_eq!(args.port, Some(9000));
        }
    }

    #[test]
    fn bare_thrall_opens_the_app() {
        let cli = Cli::try_parse_from(["thrall"]).unwrap();
        assert!(cli.command.is_none() && !cli.continue_last && cli.resume.is_none());
        let cli = Cli::try_parse_from(["thrall", "-c"]).unwrap();
        assert!(cli.continue_last);
        let cli = Cli::try_parse_from(["thrall", "-r"]).unwrap();
        assert_eq!(cli.resume.as_deref(), Some(""));
        let cli = Cli::try_parse_from(["thrall", "--resume", "s_1"]).unwrap();
        assert_eq!(cli.resume.as_deref(), Some("s_1"));
    }
}
