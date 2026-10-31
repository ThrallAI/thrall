//! Starting Thrall when the person signs in. One entry is shared by the desktop app's tray menu
//! and Settings, so both always agree. It starts the desktop app in the tray, which starts the
//! engine and keeps it running.
//!
//! - Windows: `Thrall` in the per-user Run key (and Task Manager's switch for it).
//! - Linux: `~/.config/autostart/thrall.desktop` (XDG autostart, which GNOME, KDE and the systemd
//!   session managers Hyprland setups use all read).
//! - macOS: a launch agent, `~/Library/LaunchAgents/dev.thrall.desktop.plist`.
//!
//! Only with the desktop app: the engine on its own is a console program, so Windows would open a
//! console window for it at sign-in, and closing that window would stop Thrall.

use std::path::Path;

use crate::constants::{DESKTOP_HIDDEN_ARG, DESKTOP_PROGRAM, LAUNCH_AGENT_LABEL};
use crate::error::CoreError;

/// What this system runs at sign-in for the desktop app in `folder` (beside the engine), or
/// `None` when there's no desktop app there. On Windows it's the whole command line; elsewhere
/// it's the desktop app's path (the autostart file or launch agent adds the argument).
pub fn command_line(folder: &Path) -> Option<String> {
    let desktop = folder.join(format!("{DESKTOP_PROGRAM}{}", std::env::consts::EXE_SUFFIX));
    if !desktop.is_file() {
        return None;
    }
    Some(if cfg!(windows) {
        line_for(&desktop)
    } else {
        desktop.display().to_string()
    })
}

/// The quoted program and its argument: a path with spaces ("Coding Folder") must be quoted.
fn line_for(desktop: &Path) -> String {
    format!("\"{}\" {DESKTOP_HIDDEN_ARG}", desktop.display())
}

/// Linux: the autostart entry that starts `program` hidden in the tray.
pub fn desktop_entry(program: &str) -> String {
    format!(
        "[Desktop Entry]\nType=Application\nName=Thrall\nComment=Start Thrall in the tray\n\
         Exec={} {DESKTOP_HIDDEN_ARG}\nTerminal=false\nX-GNOME-Autostart-enabled=true\n",
        exec_quote(program)
    )
}

/// One argument of a desktop entry's `Exec` line (Desktop Entry spec: quoted when it needs to
/// be, with `"`, `` ` ``, `$` and `\` escaped inside the quotes, and `%` doubled).
fn exec_quote(arg: &str) -> String {
    let arg = arg.replace('%', "%%");
    if !arg.contains([
        ' ', '\t', '"', '\'', '\\', '`', '$', ';', '&', '|', '<', '>', '(', ')',
    ]) {
        return arg;
    }
    let mut out = String::from('"');
    for c in arg.chars() {
        if matches!(c, '"' | '`' | '$' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    out
}

/// macOS: the launch agent that starts `program` hidden at log-in.
pub fn launch_agent(program: &str) -> String {
    let xml = |s: &str| {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    };
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \
         \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\">\n<dict>\n\
         \t<key>Label</key>\n\t<string>{LAUNCH_AGENT_LABEL}</string>\n\
         \t<key>ProgramArguments</key>\n\t<array>\n\
         \t\t<string>{}</string>\n\t\t<string>{DESKTOP_HIDDEN_ARG}</string>\n\t</array>\n\
         \t<key>RunAtLoad</key>\n\t<true/>\n\
         </dict>\n</plist>\n",
        xml(program)
    )
}

/// Whether Thrall starts at sign-in: its entry is there and not switched off in Task Manager.
pub fn is_on() -> Result<bool, CoreError> {
    imp::is_on()
}

/// Start Thrall at sign-in with `line` (from [`command_line`]), or stop doing so (`None`).
pub fn set(line: Option<&str>) -> Result<(), CoreError> {
    imp::set(line)
}

#[cfg(windows)]
mod imp {
    use windows_registry::{CURRENT_USER, Type};

    use crate::constants::{
        STARTUP_APPROVED_KEY, STARTUP_APPROVED_ON, STARTUP_ENTRY, STARTUP_RUN_KEY,
    };
    use crate::error::CoreError;

    /// The error code for a key or value that isn't there.
    const NOT_FOUND: u32 = 0x8007_0002;

    /// Ok(None) when the key or value isn't there.
    fn found<T>(r: windows_registry::Result<T>) -> Result<Option<T>, CoreError> {
        match r {
            Ok(v) => Ok(Some(v)),
            Err(e) if e.code().0 as u32 == NOT_FOUND => Ok(None),
            Err(e) => Err(CoreError::Startup(e.message())),
        }
    }

    pub fn is_on() -> Result<bool, CoreError> {
        let run = CURRENT_USER
            .open(STARTUP_RUN_KEY)
            .and_then(|k| k.get_string(STARTUP_ENTRY));
        if found(run)?.is_none() {
            return Ok(false);
        }
        let approved = CURRENT_USER
            .open(STARTUP_APPROVED_KEY)
            .and_then(|k| k.get_value(STARTUP_ENTRY));
        // Switched off in Task Manager: the time it was switched off fills the last 8 bytes.
        Ok(found(approved)?.is_none_or(|v| v.len() < 8 || v.iter().rev().take(8).all(|b| *b == 0)))
    }

    pub fn set(line: Option<&str>) -> Result<(), CoreError> {
        match line {
            Some(line) => {
                found(
                    CURRENT_USER
                        .create(STARTUP_RUN_KEY)
                        .and_then(|k| k.set_string(STARTUP_ENTRY, line)),
                )?;
                // Switched off in Task Manager earlier: switch it back on, or it still won't run.
                found(
                    CURRENT_USER.create(STARTUP_APPROVED_KEY).and_then(|k| {
                        k.set_bytes(STARTUP_ENTRY, Type::Bytes, &STARTUP_APPROVED_ON)
                    }),
                )?;
            }
            None => {
                found(
                    CURRENT_USER
                        .options()
                        .write()
                        .open(STARTUP_RUN_KEY)
                        .and_then(|k| k.remove_value(STARTUP_ENTRY)),
                )?;
            }
        }
        Ok(())
    }
}

/// Linux and macOS keep the entry in a file; these handle it the same way.
#[cfg(not(windows))]
mod file {
    use std::io::ErrorKind;
    use std::path::{Path, PathBuf};

    use crate::error::CoreError;

    /// The person's home folder.
    pub fn home() -> Result<PathBuf, CoreError> {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .ok_or_else(|| {
                CoreError::Startup("Couldn't find your home folder (HOME isn't set).".into())
            })
    }

    /// The entry's text, or `None` when there isn't one.
    pub fn read(path: &Path) -> Result<Option<String>, CoreError> {
        match std::fs::read_to_string(path) {
            Ok(text) => Ok(Some(text)),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
            Err(e) => Err(CoreError::Startup(format!(
                "Couldn't read {}: {e}",
                path.display()
            ))),
        }
    }

    /// Write the entry (`Some`) or remove it (`None`; gone already is fine).
    pub fn write(path: &Path, text: Option<String>) -> Result<(), CoreError> {
        let failed = |e: std::io::Error| {
            CoreError::Startup(format!("Couldn't change {}: {e}", path.display()))
        };
        match text {
            Some(text) => {
                if let Some(dir) = path.parent() {
                    std::fs::create_dir_all(dir).map_err(failed)?;
                }
                std::fs::write(path, text).map_err(failed)
            }
            None => match std::fs::remove_file(path) {
                Err(e) if e.kind() != ErrorKind::NotFound => Err(failed(e)),
                _ => Ok(()),
            },
        }
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
mod imp {
    use std::path::PathBuf;

    use crate::constants::{AUTOSTART_DIR, AUTOSTART_FILE};
    use crate::error::CoreError;

    use super::file;

    /// `$XDG_CONFIG_HOME/autostart/thrall.desktop`, else `~/.config/autostart/thrall.desktop`.
    fn entry() -> Result<PathBuf, CoreError> {
        let settings = match std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from) {
            Some(dir) if dir.is_absolute() => dir,
            _ => file::home()?.join(".config"),
        };
        Ok(settings.join(AUTOSTART_DIR).join(AUTOSTART_FILE))
    }

    pub fn is_on() -> Result<bool, CoreError> {
        Ok(file::read(&entry()?)?.is_some_and(|text| super::switched_on(&text)))
    }

    pub fn set(program: Option<&str>) -> Result<(), CoreError> {
        file::write(&entry()?, program.map(super::desktop_entry))
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use std::path::PathBuf;

    use crate::constants::{LAUNCH_AGENT_LABEL, LAUNCH_AGENTS_DIR};
    use crate::error::CoreError;

    use super::file;

    /// `~/Library/LaunchAgents/dev.thrall.desktop.plist`; launchd reads it at the next log-in.
    fn agent() -> Result<PathBuf, CoreError> {
        Ok(file::home()?
            .join(LAUNCH_AGENTS_DIR)
            .join(format!("{LAUNCH_AGENT_LABEL}.plist")))
    }

    pub fn is_on() -> Result<bool, CoreError> {
        Ok(file::read(&agent()?)?.is_some())
    }

    pub fn set(program: Option<&str>) -> Result<(), CoreError> {
        file::write(&agent()?, program.map(super::launch_agent))
    }
}

/// Linux: whether an autostart entry is switched on (desktops switch one off with `Hidden=true`
/// or GNOME's `X-GNOME-Autostart-enabled=false`).
#[cfg_attr(any(windows, target_os = "macos"), allow(dead_code))]
fn switched_on(entry: &str) -> bool {
    !entry.lines().any(|l| {
        let l = l.trim().replace(' ', "");
        l.eq_ignore_ascii_case("Hidden=true")
            || l.eq_ignore_ascii_case("X-GNOME-Autostart-enabled=false")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_desktop_app_starts_hidden_and_quoted() {
        let line = line_for(Path::new(
            r"C:\Users\me\Coding Folder\thrall\thrall-desktop.exe",
        ));
        assert_eq!(
            line,
            r#""C:\Users\me\Coding Folder\thrall\thrall-desktop.exe" --hidden"#
        );
    }

    #[test]
    fn the_linux_entry_quotes_a_path_with_spaces_and_starts_hidden() {
        let entry = desktop_entry("/home/me/My Apps/thrall-desktop");
        assert!(entry.starts_with("[Desktop Entry]\n"));
        assert!(entry.contains("\nExec=\"/home/me/My Apps/thrall-desktop\" --hidden\n"));
        assert!(
            desktop_entry("/opt/thrall/thrall-desktop")
                .contains("\nExec=/opt/thrall/thrall-desktop --hidden\n")
        );
        assert_eq!(exec_quote(r#"/a "b"/$c%"#), r#""/a \"b\"/\$c%%""#);
        assert!(switched_on(&entry));
        assert!(!switched_on(&format!("{entry}Hidden=true\n")));
        assert!(!switched_on(
            &entry.replace("enabled=true", "enabled=false")
        ));
    }

    #[test]
    fn the_mac_agent_runs_the_app_hidden_at_login() {
        let plist = launch_agent("/Applications/Thrall & Co.app/Contents/MacOS/thrall-desktop");
        assert!(plist.contains("<string>dev.thrall.desktop</string>"));
        assert!(plist.contains(
            "<string>/Applications/Thrall &amp; Co.app/Contents/MacOS/thrall-desktop</string>"
        ));
        assert!(plist.contains("<string>--hidden</string>"));
        assert!(plist.contains("<key>RunAtLoad</key>\n\t<true/>"));
    }

    #[test]
    fn without_the_desktop_app_there_is_nothing_to_start() {
        let empty = std::env::temp_dir().join("thrall-startup-test-empty");
        let _ = std::fs::create_dir_all(&empty);
        assert_eq!(command_line(&empty), None);
    }

    #[cfg(windows)]
    #[test]
    fn beside_the_desktop_app_it_starts_that() {
        let dir = std::env::temp_dir().join("thrall-startup-test-desktop");
        let _ = std::fs::create_dir_all(&dir);
        let desktop = dir.join("thrall-desktop.exe");
        let _ = std::fs::write(&desktop, b"");
        assert_eq!(command_line(&dir), Some(line_for(&desktop)));
    }
}
