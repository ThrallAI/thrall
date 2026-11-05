//! Getting the terminal onto the one engine: use the running engine, or start one — the desktop
//! app in the tray when it's installed, else a background engine that stops when unused.

use std::process::Stdio;
use std::time::Instant;

use thrall_core::{Config, Paths};
use thrall_server::EngineFile;

use crate::constants::{
    DESKTOP_EXE, DESKTOP_HIDDEN_ARG, ENGINE_START_POLL, ENGINE_START_TIMEOUT, POWERSHELL_EXE,
};
use crate::{discover, key};

/// The running engine, starting one first if needed. `None` means setup is needed (no key yet);
/// the user has been told what to do.
pub async fn engine(paths: &Paths, config: &Config) -> anyhow::Result<Option<EngineFile>> {
    if let Some(found) = discover::running(paths).await {
        return Ok(Some(found));
    }
    let provider = config.provider.kind;
    // A local model (`local/…`) needs no key.
    let local = config
        .provider
        .model
        .starts_with(thrall_core::constants::LOCAL_MODEL_PREFIX);
    // A companion PC uses the AI of the Thrall it joined: no key needed here.
    let companion = paths
        .home
        .join(thrall_core::constants::COMPANION_FILE)
        .exists();
    if !companion && key::resolve(provider)?.is_none() && !local {
        eprintln!(
            "Thrall needs your {name} key first. Run `thrall key set {name}` and paste it in.",
            name = provider.as_str()
        );
        return Ok(None);
    }
    start_in_background(companion)?;
    let started = Instant::now();
    while started.elapsed() < ENGINE_START_TIMEOUT {
        tokio::time::sleep(ENGINE_START_POLL).await;
        if let Some(found) = discover::running(paths).await {
            return Ok(Some(found));
        }
    }
    anyhow::bail!(
        "Thrall's engine didn't start. Run `thrall engine` in a terminal to see what's wrong."
    )
}

/// Start the engine detached from this terminal: the desktop app in the tray (it runs the engine
/// and keeps it going) when it's installed next to `thrall`, otherwise a background
/// `thrall engine --idle-exit` (a server, or a terminal-only install), which stops by itself once
/// nobody is using it.
fn start_in_background(companion: bool) -> anyhow::Result<()> {
    let (program, args) = launcher(&std::env::current_exe()?, companion);
    #[cfg(windows)]
    let program_is_powershell = program == std::path::Path::new(POWERSHELL_EXE);
    let mut command = std::process::Command::new(program);
    command.args(args);
    if let Some(home) = Paths::user_home() {
        command.current_dir(home);
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // PowerShell needs a console to run in (a hidden one); the engine needs none.
        command.creation_flags(if program_is_powershell {
            crate::constants::HIDDEN_CONSOLE_FLAGS
        } else {
            crate::constants::DETACHED_FLAGS
        });
    }
    command.spawn()?;
    Ok(())
}

/// What starts the engine for a `thrall` at `exe`: the desktop app beside it (in the tray), or
/// `thrall engine --idle-exit`.
///
/// On Windows the desktop app is started through `Start-Process` (shell execute): a child made
/// directly would inherit this terminal's handles, and anything reading `thrall -p` through a
/// pipe would then wait for as long as the tray app runs. The PowerShell in between exits at once.
/// On a companion PC (`companion`) without the desktop app, it's `thrall runner`.
fn launcher(exe: &std::path::Path, companion: bool) -> (std::path::PathBuf, Vec<String>) {
    let desktop = exe.with_file_name(DESKTOP_EXE);
    if !desktop.is_file() {
        let args = if companion {
            vec!["runner".to_owned()]
        } else {
            vec!["engine".to_owned(), "--idle-exit".to_owned()]
        };
        return (exe.to_path_buf(), args);
    }
    if cfg!(windows) {
        let quoted = desktop.to_string_lossy().replace('\'', "''");
        (
            std::path::PathBuf::from(POWERSHELL_EXE),
            vec![
                "-NoProfile".to_owned(),
                "-NonInteractive".to_owned(),
                "-Command".to_owned(),
                format!("Start-Process -FilePath '{quoted}' -ArgumentList '{DESKTOP_HIDDEN_ARG}'"),
            ],
        )
    } else {
        (desktop, vec![DESKTOP_HIDDEN_ARG.to_owned()])
    }
}

/// The access token clients present.
pub fn token(paths: &Paths) -> anyhow::Result<String> {
    let text = std::fs::read_to_string(&paths.token_file).map_err(|e| {
        anyhow::anyhow!(
            "Couldn't read Thrall's access token ({}): {e}",
            paths.token_file.display()
        )
    })?;
    Ok(text.trim().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_desktop_app_runs_the_engine_when_it_is_installed() {
        let dir = std::env::temp_dir().join(format!("thrall-launcher-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("thrall.exe");
        // Terminal only: a background engine that stops when unused.
        let (program, args) = launcher(&exe, false);
        assert_eq!(program, exe);
        assert_eq!(args, vec!["engine", "--idle-exit"]);
        // With the desktop app beside it: the app, in the tray (on Windows via Start-Process,
        // so it doesn't hold this terminal's handles).
        std::fs::write(dir.join(DESKTOP_EXE), b"").unwrap();
        let (program, args) = launcher(&exe, false);
        let all = args.join(" ");
        if cfg!(windows) {
            assert_eq!(program, std::path::PathBuf::from(POWERSHELL_EXE));
            assert!(all.contains("Start-Process"), "{all}");
            assert!(all.contains(&dir.join(DESKTOP_EXE).to_string_lossy().into_owned()));
        } else {
            assert_eq!(program, dir.join(DESKTOP_EXE));
        }
        assert!(all.contains(DESKTOP_HIDDEN_ARG), "{all}");
    }
}
