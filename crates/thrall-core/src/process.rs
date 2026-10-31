//! Starting other programs. On Windows, a console program started by the engine (which has no
//! console of its own) gets a new console window: one per MCP server, shell command or hook. Those
//! windows flash up in front of the person, and closing one kills the program inside it. Every
//! program the engine starts goes through [`no_window`] first.

/// Start `command` without a console window (Windows); nothing to do elsewhere.
pub fn no_window(command: &mut std::process::Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(crate::constants::CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    let _ = command;
}
