//! Shell commands and git. Shell always asks first; git asks unless the command only reads.

use std::time::Duration;

use futures::future::BoxFuture;
use serde::Deserialize;
use serde_json::{Value, json};
use thrall_gates::Effect;
use thrall_protocol::Risk;

use crate::constants::{
    GIT_READ_ONLY, GIT_RISKY, GIT_TIMEOUT_SECS, GIT_WRITE_FLAGS, NONZERO_EXIT_NOTE,
    RESULT_MAX_CHARS, SHELL_CHAINING, SHELL_DEFAULT_TIMEOUT_SECS, SHELL_MAX_TIMEOUT_SECS,
    SHELL_RISKY,
};
use crate::process::{self, Finished};
use crate::tool::{Tool, ToolContext, ToolOutput, arg_str, cap, parse_args};

/// The shell and the arguments that run one command string in it (also used for hooks).
pub fn shell_command(command: &str) -> (&'static str, Vec<String>) {
    if cfg!(windows) {
        // UTF-8 output, so non-ASCII text survives the pipe.
        let script = format!(
            "[Console]::OutputEncoding = [System.Text.Encoding]::UTF8; $ProgressPreference = \
             'SilentlyContinue'; {command}"
        );
        (
            "powershell.exe",
            vec![
                "-NoLogo".into(),
                "-NoProfile".into(),
                "-NonInteractive".into(),
                "-Command".into(),
                script,
            ],
        )
    } else if cfg!(target_os = "macos") {
        // zsh is macOS's shell. A login shell reads ~/.zprofile, where Homebrew puts itself on
        // PATH: an app started from the Dock or at login otherwise has only /usr/bin and /bin.
        (
            "/bin/zsh",
            vec!["-l".into(), "-c".into(), command.to_owned()],
        )
    } else if std::path::Path::new("/bin/bash").exists()
        || std::path::Path::new("/usr/bin/bash").exists()
    {
        // A login shell reads ~/.profile, where rustup, nvm and the like put themselves on PATH.
        ("bash", vec!["-l".into(), "-c".into(), command.to_owned()])
    } else {
        ("sh", vec!["-c".into(), command.to_owned()])
    }
}

/// How risky a shell command looks. High-risk commands are never covered by "always".
fn shell_risk(command: &str) -> Risk {
    let lower = command.to_lowercase();
    if SHELL_RISKY.iter().any(|r| lower.contains(r)) {
        Risk::High
    } else {
        Risk::Medium
    }
}

/// What an "always" for this command covers: its program (`shell:cargo`). Commands that chain or
/// pipe several programs, or that look risky, can't be covered.
fn shell_grant(command: &str) -> Option<String> {
    if command.contains(SHELL_CHAINING) || shell_risk(command) == Risk::High {
        return None;
    }
    let program = command.split_whitespace().next()?.to_lowercase();
    Some(format!("shell:{program}"))
}

fn describe(finished: Finished, timeout_secs: u64, what: &str) -> ToolOutput {
    let body = if finished.output.trim().is_empty() {
        "(no output)".to_owned()
    } else {
        finished.output
    };
    if finished.cancelled {
        return ToolOutput::error(format!(
            "{what} was stopped because the user interrupted (it and everything it started \
             were killed).\n{}",
            cap(body, RESULT_MAX_CHARS)
        ));
    }
    if finished.timed_out {
        return ToolOutput::error(format!(
            "{what} was stopped after {timeout_secs}s (it and everything it started were \
             killed).\n{}",
            cap(body, RESULT_MAX_CHARS)
        ));
    }
    match finished.code {
        Some(0) => ToolOutput::ok(cap(body, RESULT_MAX_CHARS), "Exit code 0"),
        Some(code) => {
            let mut out = ToolOutput::ok(
                format!(
                    "Exit code {code}{NONZERO_EXIT_NOTE}\n{}",
                    cap(body, RESULT_MAX_CHARS)
                ),
                format!("Exit code {code}"),
            );
            out.ok = false;
            out.warn = true;
            out
        }
        None => ToolOutput::error(format!("{what} ended without an exit code\n{body}")),
    }
}

// ---------- shell ----------

/// Run a shell command in the session folder. Always asks first.
pub struct Shell;

#[derive(Deserialize)]
struct ShellArgs {
    command: String,
    #[serde(default)]
    timeout_secs: Option<u64>,
}

impl Tool for Shell {
    fn name(&self) -> &'static str {
        "shell"
    }
    fn description(&self) -> &'static str {
        if cfg!(windows) {
            "Run Windows PowerShell code in the session folder. `command` is PowerShell itself: \
             write it exactly as you would type it at a PowerShell prompt. Never wrap it in \
             `powershell -Command` or `cmd /c` (a nested shell mangles $variables and quotes). \
             Several statements can go on one line separated by `;`. Prefer the file tools \
             (glob, grep, read_file) over shell commands for finding and reading files, and \
             don't leave helper scripts behind. The user approves every command. Output (stdout \
             and stderr) is returned. Default timeout 120s, max 600s."
        } else if cfg!(target_os = "macos") {
            "Run zsh code on macOS in the session folder. `command` is the shell code itself: \
             write it exactly as you would type it in Terminal. Never wrap it in `zsh -c` or \
             `bash -c`. Chain steps with `&&`. Nothing can be typed in while it runs, so never \
             use sudo, editors or pagers: pass `--no-pager`, `-y` or `--yes` where a program \
             would ask. macOS tools are the BSD ones (`sed -i ''`, no `--long` options on \
             `ls`/`stat`); Homebrew installs with `brew`. Prefer the file tools (glob, grep, \
             read_file) over shell commands for finding and reading files, and don't leave \
             helper scripts behind. The user approves every command. Output (stdout and stderr) \
             is returned. Default timeout 120s, max 600s."
        } else {
            "Run bash code on Linux in the session folder (sh where bash isn't installed). \
             `command` is the shell code itself: write it exactly as you would type it in a \
             terminal. Never wrap it in `bash -c` or `sh -c`. Chain steps with `&&`. Nothing can \
             be typed in while it runs, so never use sudo, editors or pagers: pass `--no-pager`, \
             `-y` or `--noconfirm` where a program would ask, and tell the user the command to \
             run themselves when it needs sudo. Prefer the file tools (glob, grep, read_file) \
             over shell commands for finding and reading files, and don't leave helper scripts \
             behind. The user approves every command. Output (stdout and stderr) is returned. \
             Default timeout 120s, max 600s."
        }
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": { "type": "string" },
                "timeout_secs": { "type": "integer", "description": "Default 120, max 600" }
            },
            "required": ["command"]
        })
    }
    fn title(&self, args: &Value) -> String {
        format!("Running {}", arg_str(args, "command"))
    }
    fn effect(&self, _ctx: &ToolContext, args: &Value) -> Effect {
        let command = arg_str(args, "command");
        Effect::Run {
            summary: format!("Run `{command}`"),
            risk: shell_risk(command),
            grant: shell_grant(command),
        }
    }
    fn run<'a>(&'a self, ctx: &'a ToolContext, args: Value) -> BoxFuture<'a, ToolOutput> {
        Box::pin(async move {
            let a: ShellArgs = match parse_args(self.name(), args) {
                Ok(a) => a,
                Err(e) => return ToolOutput::error(e),
            };
            let secs = a
                .timeout_secs
                .unwrap_or(SHELL_DEFAULT_TIMEOUT_SECS)
                .clamp(1, SHELL_MAX_TIMEOUT_SECS);
            let (program, argv) = shell_command(&a.command);
            match process::run(
                program,
                &argv,
                &ctx.workspace,
                Duration::from_secs(secs),
                &ctx.output,
                ctx.cancel.clone(),
            )
            .await
            {
                Ok(done) => describe(done, secs, "The command"),
                Err(e) => ToolOutput::error(format!("couldn't start {program}: {e}")),
            }
        })
    }
}

// ---------- git ----------

/// Run git. Read-only commands run straight away; anything else asks.
pub struct Git;

#[derive(Deserialize)]
struct GitArgs {
    args: Vec<String>,
}

fn read_only(args: &[String]) -> bool {
    let Some(sub) = args.iter().find(|a| !a.starts_with('-')) else {
        return false;
    };
    GIT_READ_ONLY.contains(&sub.as_str())
        && !args.iter().any(|a| {
            GIT_WRITE_FLAGS
                .iter()
                .any(|f| a == f || a.starts_with(&format!("{f}=")))
        })
}

fn git_args(args: &Value) -> Vec<String> {
    args.get("args")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

impl Tool for Git {
    fn name(&self) -> &'static str {
        "git"
    }
    fn description(&self) -> &'static str {
        "Run git in the session folder with the given arguments, e.g. [\"status\"] or [\"diff\", \
         \"--stat\"]. Read-only commands (status, diff, log, show, blame…) run straight away; \
         anything that changes the repository asks the user first."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "args": { "type": "array", "items": { "type": "string" } }
            },
            "required": ["args"]
        })
    }
    fn title(&self, args: &Value) -> String {
        format!("git {}", git_args(args).join(" "))
    }
    fn effect(&self, _ctx: &ToolContext, args: &Value) -> Effect {
        let argv = git_args(args);
        if read_only(&argv) {
            return Effect::Read;
        }
        let sub = argv
            .iter()
            .find(|a| !a.starts_with('-'))
            .cloned()
            .unwrap_or_default();
        let joined = argv.join(" ");
        let risky = GIT_RISKY.iter().any(|r| joined.contains(r));
        Effect::Run {
            summary: format!("Run `git {joined}`"),
            risk: if risky { Risk::High } else { Risk::Medium },
            grant: (!sub.is_empty()).then(|| format!("git:{sub}")),
        }
    }
    fn run<'a>(&'a self, ctx: &'a ToolContext, args: Value) -> BoxFuture<'a, ToolOutput> {
        Box::pin(async move {
            let a: GitArgs = match parse_args(self.name(), args) {
                Ok(a) => a,
                Err(e) => return ToolOutput::error(e),
            };
            // No pager, no colour codes, no prompts that would hang.
            let mut argv = vec![
                "--no-pager".to_owned(),
                "-c".into(),
                "color.ui=never".into(),
            ];
            argv.extend(a.args);
            match process::run(
                "git",
                &argv,
                &ctx.workspace,
                Duration::from_secs(GIT_TIMEOUT_SECS),
                &ctx.output,
                ctx.cancel.clone(),
            )
            .await
            {
                Ok(done) => describe(done, GIT_TIMEOUT_SECS, "git"),
                Err(e) => ToolOutput::error(format!("couldn't run git (is it installed?): {e}")),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn read_only_git_is_recognised() {
        assert!(read_only(&v(&["status"])));
        assert!(read_only(&v(&["diff", "--stat"])));
        assert!(read_only(&v(&["--no-pager", "log", "-5"])));
        assert!(!read_only(&v(&["commit", "-m", "x"])));
        assert!(!read_only(&v(&["push"])));
        assert!(!read_only(&v(&["diff", "--output=patch.txt"])));
        assert!(!read_only(&v(&[])));
    }

    #[test]
    fn shell_grants_and_risk() {
        assert_eq!(shell_grant("cargo test -p x"), Some("shell:cargo".into()));
        assert_eq!(shell_grant("cargo test; rm -rf /"), None);
        assert_eq!(shell_grant("ls | sort"), None);
        assert_eq!(shell_risk("Remove-Item -Recurse -Force build"), Risk::High);
        assert_eq!(shell_risk("rm -rf target"), Risk::High);
        assert_eq!(shell_risk("cargo build"), Risk::Medium);
        assert_eq!(shell_grant("rm -rf target"), None);
    }

    fn finished(code: Option<i32>, timed_out: bool) -> Finished {
        Finished {
            code,
            output: "done\n".into(),
            timed_out,
            cancelled: false,
        }
    }

    #[test]
    fn a_nonzero_exit_is_a_warning_and_a_timeout_a_failure() {
        let ok = describe(finished(Some(0), false), 60, "The command");
        assert!(ok.ok && !ok.warn);
        // It ran to the end: its output may still be what was asked.
        let exit1 = describe(finished(Some(1), false), 60, "The command");
        assert!(!exit1.ok && exit1.warn);
        assert_eq!(exit1.summary, "Exit code 1");
        assert!(exit1.content.contains("done"));
        let late = describe(finished(None, true), 60, "The command");
        assert!(!late.ok && !late.warn);
        let killed = describe(finished(None, false), 60, "The command");
        assert!(!killed.ok && !killed.warn);
    }
}
