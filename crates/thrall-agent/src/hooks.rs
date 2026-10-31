//! Hooks: the user's own commands from `[[hooks]]` in config.toml, run at points in a turn. Each
//! gets the details as JSON on its input. A `pre_tool` hook that exits with code 2 blocks the tool,
//! and what it printed tells Thrall why. A `user_message` hook's output is added for Thrall to read.

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use serde_json::Value;
use thrall_core::config::HookEvent;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use crate::constants::{HOOK_BLOCK_EXIT_CODE, HOOK_DEFAULT_TIMEOUT_SECS, HOOK_OUTPUT_MAX_CHARS};
use crate::state::AppState;

/// What the hooks for one event came to.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct HookOutcome {
    /// A `pre_tool` hook blocked the tool; the reason.
    pub blocked: Option<String>,
    /// What the hooks printed (for `user_message`, added to the message).
    pub output: String,
}

/// Run every hook for `event` (and `tool`, for tool events).
pub async fn fire(
    state: &AppState,
    event: HookEvent,
    tool: Option<&str>,
    payload: Value,
    workspace: &Path,
) -> HookOutcome {
    let mut outcome = HookOutcome::default();
    let input = serde_json::json!({ "event": event.as_str(), "details": payload }).to_string();
    for hook in state.config.hooks.iter().filter(|h| h.event == event) {
        if let (Some(want), Some(tool)) = (&hook.tool, tool)
            && !want.eq_ignore_ascii_case(tool)
        {
            continue;
        }
        let secs = hook.timeout_secs.unwrap_or(HOOK_DEFAULT_TIMEOUT_SECS);
        match run(&hook.command, &input, workspace, Duration::from_secs(secs)).await {
            Ok((code, stdout, stderr)) => {
                if code == Some(HOOK_BLOCK_EXIT_CODE) && event == HookEvent::PreTool {
                    let why = if stderr.trim().is_empty() {
                        stdout
                    } else {
                        stderr
                    };
                    outcome.blocked = Some(cap(why.trim()));
                    return outcome;
                }
                if code != Some(0) {
                    tracing::warn!(command = hook.command, ?code, %stderr, "hook failed");
                }
                if !stdout.trim().is_empty() {
                    if !outcome.output.is_empty() {
                        outcome.output.push('\n');
                    }
                    outcome.output.push_str(&cap(stdout.trim()));
                }
            }
            Err(e) => tracing::warn!(command = hook.command, %e, "hook couldn't run"),
        }
    }
    outcome
}

fn cap(text: &str) -> String {
    text.chars().take(HOOK_OUTPUT_MAX_CHARS).collect()
}

async fn run(
    command: &str,
    input: &str,
    cwd: &Path,
    timeout: Duration,
) -> std::io::Result<(Option<i32>, String, String)> {
    let (program, args) = thrall_tools::tools::shell_command(command);
    let mut command = Command::new(program);
    command
        .args(&args)
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    thrall_core::process::no_window(command.as_std_mut());
    let mut child = command.spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        // A hook that ignores its input closes the pipe early; that's fine.
        let _ = stdin.write_all(input.as_bytes()).await;
    }
    match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Ok(out) => {
            let out = out?;
            Ok((
                out.status.code(),
                String::from_utf8_lossy(&out.stdout).into_owned(),
                String::from_utf8_lossy(&out.stderr).into_owned(),
            ))
        }
        Err(_) => Err(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            format!("stopped after {}s", timeout.as_secs()),
        )),
    }
}
