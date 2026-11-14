//! Shell commands and git. Shell always asks first; git asks unless the command only reads.

use std::path::Path;
use std::time::Duration;

use futures::future::BoxFuture;
use serde::Deserialize;
use serde_json::{Value, json};
use thrall_gates::Effect;
use thrall_protocol::Risk;

use crate::constants::{
    BACKGROUND_MAX_PER_PERSON, BACKGROUND_READ_MAX_CHARS, COMMAND_SEPARATORS, GIT_READ_ONLY,
    GIT_RISKY, GIT_TIMEOUT_SECS, GIT_WRITE_FLAGS, INSTALL_CUSTOM_SOURCES, INSTALL_PIECES,
    INSTALL_SOURCE_FLAGS, INSTALL_VALUE_FLAGS, INSTALL_VERBS, KEEP_RUNNING_SETTLE_SECS,
    LEFT_RUNNING_NOTE, LEFT_TRACKED_NOTE, LOCAL_INSTALL_FLAGS, NONZERO_EXIT_NOTE, NPM_NAMES,
    OFFICIAL_SOURCES, PACKAGE_PROGRAMS, PIP_NAMES, PIP_REQUIREMENT_FLAGS, PIP_SOURCE_FLAGS,
    REG_EXE_WRITES, REGISTRY_NAMES, REGISTRY_WRITERS, REQUIREMENTS_DEPTH_MAX,
    REQUIREMENTS_MAX_BYTES, RESULT_MAX_CHARS, SHELL_BY_NAME, SHELL_CAREFUL, SHELL_CHAINING,
    SHELL_CUSTOM_SOURCE_NOTE, SHELL_DEFAULT_TIMEOUT_SECS, SHELL_DELETERS, SHELL_DELETES_NOTE,
    SHELL_MAX_TIMEOUT_SECS, SHELL_MOVERS, SHELL_RECURSIVE, SHELL_RISKY, SHELL_RISKY_NOTE,
    SHELL_STOPPERS, SHELL_SYSTEM_NOTE, SHELL_WRITERS, SHORT_SOURCE_FLAGS, SHORT_SOURCE_PROGRAMS,
    STOPPED_DETACHED_NOTE, STOPPED_NOTE, SYSTEM_CHANGE_NEEDS, UNTRACKED_PROGRAMS, UNTRACKED_STARTS,
};
use crate::paths::inside;
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

/// The command's words, lower case, wherever they sit in a chain or pipe, quotes taken off.
fn words(command: &str) -> Vec<String> {
    command
        .to_lowercase()
        .split(|c: char| c.is_whitespace() || SHELL_CHAINING.contains(&c) || "(){},=".contains(c))
        .map(|w| w.trim_matches(|c| c == '\'' || c == '"').to_owned())
        .filter(|w| !w.is_empty())
        .collect()
}

/// Whether `word` is a full path (`C:\…`, `\\server\…`, and on Linux and macOS `/…` or `~…`).
fn full_path(word: &str) -> bool {
    let bytes = word.as_bytes();
    let drive = bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'\\' || bytes[2] == b'/');
    drive || word.starts_with("\\\\") || (!cfg!(windows) && word.starts_with(['/', '~']))
}

/// Whether what the command works on may be outside `workspace`: a full path outside it, a `..`,
/// a variable, or a move to another folder.
fn reaches_outside(command: &str, words: &[String], workspace: &Path) -> bool {
    // A variable can name any folder (`$` splits words, so it's looked for in the command).
    command.contains('$')
        || words.iter().any(|w| {
            (full_path(w) && !inside(workspace, Path::new(w)))
                || w.contains("..")
                || (!cfg!(windows) && w.starts_with('~'))
                || SHELL_MOVERS.contains(&w.as_str())
        })
}

/// Whether the command changes the registry, or something in the system's folders (Windows,
/// programs, another person's user folder).
fn system_change(command: &str, workspace: &Path) -> bool {
    let lower = command.to_lowercase();
    let words = words(command);
    let has = |list: &[&str]| words.iter().any(|w| list.contains(&w.as_str()));
    let registry = REG_EXE_WRITES.iter().any(|r| lower.contains(r))
        || (REGISTRY_NAMES.iter().any(|r| lower.contains(r)) && has(REGISTRY_WRITERS));
    let system_files = (has(SHELL_WRITERS) || command.contains('>'))
        && words
            .iter()
            .any(|w| full_path(w) && thrall_gates::system_folder(w, &workspace.to_string_lossy()));
    registry || system_files
}

/// Whether the command installs something from a place that isn't the official one: a web or git
/// address, another index or registry, a package file, a source flag naming an unofficial
/// source, a local folder, an npm `user/repo` shorthand, or a requirements file naming any of
/// these (or that can't be read to check).
fn custom_source_install(command: &str, workspace: &Path) -> bool {
    let lower = command.to_lowercase();
    if !INSTALL_PIECES.iter().any(|r| lower.contains(r)) {
        return false;
    }
    let words = words(command);
    let has = |list: &[&str]| words.iter().any(|w| list.contains(&w.as_str()));
    let pip = has(PIP_NAMES);
    let short = has(SHORT_SOURCE_PROGRAMS);
    let source_flag = |w: &str| {
        INSTALL_SOURCE_FLAGS.contains(&w)
            || (pip && PIP_SOURCE_FLAGS.contains(&w))
            || (short && SHORT_SOURCE_FLAGS.contains(&w))
    };
    INSTALL_CUSTOM_SOURCES.iter().any(|r| lower.contains(r))
        || words
            .windows(2)
            .any(|pair| source_flag(&pair[0]) && !OFFICIAL_SOURCES.contains(&pair[1].as_str()))
        || command
            .split(COMMAND_SEPARATORS)
            .any(|one| unofficial_packages(one, workspace))
}

/// A command's words with their case kept (file names on Linux and macOS), quotes taken off.
fn cased_words(command: &str) -> Vec<&str> {
    command
        .split(|c: char| c.is_whitespace() || c == '=')
        .map(|w| w.trim_matches(|c| c == '\'' || c == '"'))
        .filter(|w| !w.is_empty())
        .collect()
}

/// Whether `word` names a folder or file on this computer rather than a package.
fn local_place(word: &str) -> bool {
    word == "."
        || word == ".."
        || word.starts_with("./")
        || word.starts_with(".\\")
        || word.starts_with("../")
        || word.starts_with("..\\")
        || full_path(word)
}

/// Whether one command of a chain installs with pip, npm's family or cargo from a local folder,
/// a `user/repo` shorthand, or a requirements file that isn't all official.
fn unofficial_packages(one: &str, workspace: &Path) -> bool {
    let words = cased_words(one);
    let lower: Vec<String> = words.iter().map(|w| w.to_lowercase()).collect();
    let Some(verb) = lower
        .iter()
        .position(|w| INSTALL_VERBS.contains(&w.as_str()))
    else {
        return false;
    };
    let before = |list: &[&str]| lower[..verb].iter().any(|w| list.contains(&w.as_str()));
    if !before(PACKAGE_PROGRAMS) {
        return false;
    }
    let (pip, npm) = (before(PIP_NAMES), before(NPM_NAMES));
    let mut skip = false;
    for (i, word) in lower.iter().enumerate().skip(verb + 1) {
        let previous = lower[i - 1].as_str();
        if pip && PIP_REQUIREMENT_FLAGS.contains(&previous) {
            if requirements_unofficial(&workspace.join(words[i]), 0) {
                return true;
            }
        } else if LOCAL_INSTALL_FLAGS.contains(&words[i - 1]) {
            // Case kept: npm's -E is --save-exact, pip's -e is a folder.
            return true;
        } else if skip || word.starts_with('-') {
            // A flag, or the folder after one like --target.
        } else if local_place(word) || (npm && word.contains('/') && !word.starts_with('@')) {
            return true;
        }
        skip = INSTALL_VALUE_FLAGS.contains(&word.as_str());
    }
    false
}

/// Whether a pip requirements file names an unofficial place: another index, a web or git
/// address, a package file, a local folder, or another such file. One that can't be read (not
/// there yet, too big, too deep) counts as unofficial, as what it installs can't be seen.
fn requirements_unofficial(file: &Path, depth: usize) -> bool {
    if depth >= REQUIREMENTS_DEPTH_MAX {
        return true;
    }
    let readable =
        std::fs::metadata(file).is_ok_and(|m| m.is_file() && m.len() <= REQUIREMENTS_MAX_BYTES);
    let text = if readable {
        std::fs::read_to_string(file).ok()
    } else {
        None
    };
    let Some(text) = text else {
        return true;
    };
    let folder = file.parent().unwrap_or(Path::new("."));
    text.lines().any(|line| {
        let line = line.split(" #").next().unwrap_or("").trim();
        if line.starts_with('#') {
            return false;
        }
        let lower = line.to_lowercase();
        let words = cased_words(line);
        let first = words.first().map(|w| w.to_lowercase()).unwrap_or_default();
        INSTALL_CUSTOM_SOURCES.iter().any(|r| lower.contains(r))
            || PIP_SOURCE_FLAGS.contains(&first.as_str())
            || LOCAL_INSTALL_FLAGS.contains(&first.as_str())
            || local_place(&first)
            || (PIP_REQUIREMENT_FLAGS.contains(&first.as_str())
                && words
                    .get(1)
                    .is_none_or(|next| requirements_unofficial(&folder.join(next), depth + 1)))
    })
}

/// How risky a shell command looks, for a session working in `workspace`:
/// - High asks in every mode and is never covered by "always": the `SHELL_RISKY` list, changing
///   the registry or system files, stopping programs by name, deleting whole folders outside the
///   session's, and installing from an unofficial source.
/// - Medium asks unless the mode is Full auto: deleting, installing, stopping a program.
/// - Low is an everyday command (building, testing, running a script): Auto edits runs it
///   without asking.
fn shell_risk(command: &str, workspace: &Path) -> Risk {
    let lower = command.to_lowercase();
    let words = words(command);
    let has = |list: &[&str]| words.iter().any(|w| list.contains(&w.as_str()));
    let deletes = has(SHELL_DELETERS);
    let stops = has(SHELL_STOPPERS);
    if SHELL_RISKY.iter().any(|r| lower.contains(r))
        || system_change(command, workspace)
        || (stops && has(SHELL_BY_NAME))
        || (deletes && has(SHELL_RECURSIVE) && reaches_outside(command, &words, workspace))
        || custom_source_install(command, workspace)
    {
        Risk::High
    } else if deletes || stops || SHELL_CAREFUL.iter().any(|r| lower.contains(r)) {
        Risk::Medium
    } else {
        Risk::Low
    }
}

/// Whether any program in the command deletes files (`rm`, `Remove-Item`, `del`…), wherever it
/// sits in a chain or pipe.
fn shell_deletes(command: &str) -> bool {
    words(command)
        .iter()
        .any(|word| SHELL_DELETERS.contains(&word.as_str()))
}

/// The permission card's line: what the command does to files first, when it deletes, is risky
/// or changes the system (then with what it does and how to undo it).
fn shell_summary(args: &Value, workspace: &Path) -> String {
    let command = arg_str(args, "command");
    if system_change(command, workspace) {
        return format!(
            "{SHELL_SYSTEM_NOTE}Run `{command}`\nOutcome: {}\nUndo: {}",
            arg_str(args, "outcome").trim(),
            arg_str(args, "undo").trim()
        );
    }
    let note = if shell_deletes(command) {
        SHELL_DELETES_NOTE
    } else if custom_source_install(command, workspace) {
        SHELL_CUSTOM_SOURCE_NOTE
    } else if shell_risk(command, workspace) == Risk::High {
        SHELL_RISKY_NOTE
    } else {
        ""
    };
    format!("{note}Run `{command}`")
}

/// Why a change to the registry or system files can't go ahead: it needs `outcome`, and `undo`
/// naming a backup that's there.
fn system_change_blocked(args: &Value, workspace: &Path) -> Option<String> {
    if !system_change(arg_str(args, "command"), workspace) {
        return None;
    }
    let undo = arg_str(args, "undo");
    // Not lower-cased: file names are case-sensitive on Linux.
    let backed_up = undo
        .split(|c: char| c.is_whitespace() || SHELL_CHAINING.contains(&c) || "(){},=".contains(c))
        .map(|w| w.trim_matches(|c| c == '\'' || c == '"'))
        .any(|w| !w.is_empty() && crate::paths::resolve(workspace, w).is_file());
    (arg_str(args, "outcome").trim().is_empty() || !backed_up)
        .then(|| SYSTEM_CHANGE_NEEDS.to_owned())
}

/// What an "always" for this command covers: its program (`shell:cargo`). Commands that chain or
/// pipe several programs, delete, or look risky can't be covered.
fn shell_grant(command: &str, workspace: &Path) -> Option<String> {
    if command.contains(SHELL_CHAINING)
        || shell_deletes(command)
        || shell_risk(command, workspace) == Risk::High
    {
        return None;
    }
    let program = command.split_whitespace().next()?.to_lowercase();
    Some(format!("shell:{program}"))
}

/// Whether a shell command starts a program that outlives it: one of `UNTRACKED_STARTS` or
/// `UNTRACKED_PROGRAMS` where a command begins, not just named (as in `Get-Process pythonw`).
pub fn starts_untracked(command: &str) -> bool {
    let command = command.to_lowercase();
    command.split(COMMAND_SEPARATORS).any(|part| {
        let mut part = part.trim();
        // `$p = Start-Process …`
        if part.starts_with('$')
            && let Some((_, rest)) = part.split_once('=')
        {
            part = rest.trim();
        }
        let program = match part.strip_prefix(['"', '\'']) {
            Some(quoted) => quoted.split(['"', '\'']).next().unwrap_or_default(),
            None => part.split_whitespace().next().unwrap_or_default(),
        };
        let name = program.rsplit(['/', '\\']).next().unwrap_or_default();
        let name = name.strip_suffix(".exe").unwrap_or(name);
        UNTRACKED_STARTS.iter().any(|s| part.starts_with(s)) || UNTRACKED_PROGRAMS.contains(&name)
    })
}

/// `command` is what ran, for the warning that something it started on its own may have
/// escaped a stop (empty: no warning).
fn describe(finished: Finished, timeout_secs: u64, what: &str, command: &str) -> ToolOutput {
    let mut body = if finished.output.trim().is_empty() {
        "(no output)".to_owned()
    } else {
        finished.output
    };
    let detached = if starts_untracked(command) {
        STOPPED_DETACHED_NOTE
    } else {
        ""
    };
    if finished.cancelled {
        return ToolOutput::error(format!(
            "{what} was stopped because the user interrupted ({STOPPED_NOTE}{detached}).\n{}",
            cap(body, RESULT_MAX_CHARS)
        ));
    }
    if finished.timed_out {
        return ToolOutput::error(format!(
            "{what} was stopped after {timeout_secs}s ({STOPPED_NOTE}{detached}).\n{}",
            cap(body, RESULT_MAX_CHARS)
        ));
    }
    if finished.left.is_some() {
        body = format!("{}\n\n{LEFT_RUNNING_NOTE}", cap(body, RESULT_MAX_CHARS));
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

/// [`describe`], keeping track of what the command left running as the person's (so
/// shell_output and shell_stop work on it), and saying so.
fn describe_tracked(ctx: &ToolContext, command: &str, mut done: Finished, secs: u64) -> ToolOutput {
    let note = done.left.take_if(|l| !l.programs.is_empty()).map(|left| {
        let programs = left
            .programs
            .iter()
            .map(|(pid, name)| format!("{name} (process ID {pid})"))
            .collect::<Vec<_>>()
            .join(", ");
        let id =
            crate::background::adopt(&ctx.user_id, command, left.stop, left.lines, String::new());
        LEFT_TRACKED_NOTE
            .replace("{programs}", &programs)
            .replace("{id}", &id)
    });
    let mut out = describe(done, secs, "The command", command);
    if let Some(note) = note {
        out.content.push_str("\n\n");
        out.content.push_str(&note);
    }
    out
}

// ---------- shell ----------

/// Run a shell command in the session folder. Always asks first.
pub struct Shell;

#[derive(Deserialize)]
struct ShellArgs {
    command: String,
    #[serde(default)]
    timeout_secs: Option<u64>,
    #[serde(default)]
    wait_for: Option<String>,
    #[serde(default)]
    keep_running: bool,
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
             don't leave helper scripts behind. Risky commands need the user's approval. Output (stdout \
             and stderr) is returned. Default timeout 120s, max 600s."
        } else if cfg!(target_os = "macos") {
            "Run zsh code on macOS in the session folder. `command` is the shell code itself: \
             write it exactly as you would type it in Terminal. Never wrap it in `zsh -c` or \
             `bash -c`. Chain steps with `&&`. Nothing can be typed in while it runs, so never \
             use sudo, editors or pagers: pass `--no-pager`, `-y` or `--yes` where a program \
             would ask. macOS tools are the BSD ones (`sed -i ''`, no `--long` options on \
             `ls`/`stat`); Homebrew installs with `brew`. Prefer the file tools (glob, grep, \
             read_file) over shell commands for finding and reading files, and don't leave \
             helper scripts behind. Risky commands need the user's approval. Output (stdout and stderr) \
             is returned. Default timeout 120s, max 600s."
        } else {
            "Run bash code on Linux in the session folder (sh where bash isn't installed). \
             `command` is the shell code itself: write it exactly as you would type it in a \
             terminal. Never wrap it in `bash -c` or `sh -c`. Chain steps with `&&`. Nothing can \
             be typed in while it runs, so never use sudo, editors or pagers: pass `--no-pager`, \
             `-y` or `--noconfirm` where a program would ask, and tell the user the command to \
             run themselves when it needs sudo. Prefer the file tools (glob, grep, read_file) \
             over shell commands for finding and reading files, and don't leave helper scripts \
             behind. Risky commands need the user's approval. Output (stdout and stderr) is returned. \
             Default timeout 120s, max 600s."
        }
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": { "type": "string" },
                "timeout_secs": { "type": "integer", "description": "Default 120, max 600" },
                "wait_for": { "type": "string", "description": "Return as soon as a line of output contains this (text, or a regular expression), e.g. 'Listening on' for a server starting. Without keep_running the program is then stopped. Not for a windowed app, which prints nothing: start it with a plain command, and if it keeps running Thrall says so and keeps track of it." },
                "keep_running": { "type": "boolean", "description": "Leave the program running in the background (a dev server, a watcher): once the wait_for line appears, or without wait_for once it's been going a few seconds. You get an id for shell_output and shell_stop, and Thrall keeps track of it. Prefer this to Start-Process or & for anything that keeps running. Stop it when you're done." },
                "outcome": { "type": "string", "description": "Only for a change to the registry or system files: what will change and what that does." },
                "undo": { "type": "string", "description": "Only for a change to the registry or system files: how to put it back, naming the backup you made first (reg export <key> <file>.reg, or a copy of the file)." }
            },
            "required": ["command"]
        })
    }
    fn title(&self, args: &Value) -> String {
        format!("Running {}", arg_str(args, "command"))
    }
    fn precheck(&self, ctx: &ToolContext, args: &Value) -> Option<String> {
        system_change_blocked(args, &ctx.workspace)
    }
    fn effect(&self, ctx: &ToolContext, args: &Value) -> Effect {
        let command = arg_str(args, "command");
        let risk = shell_risk(command, &ctx.workspace);
        Effect::Run {
            summary: shell_summary(args, &ctx.workspace),
            risk,
            grant: shell_grant(command, &ctx.workspace),
            everyday: risk == Risk::Low,
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
            let pattern = a.wait_for.as_deref().filter(|p| !p.trim().is_empty());
            if pattern.is_some() || a.keep_running {
                return watch(ctx, &a, pattern, secs, program, &argv).await;
            }
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
                Ok(done) => describe_tracked(ctx, &a.command, done, secs),
                Err(e) => ToolOutput::error(format!("couldn't start {program}: {e}")),
            }
        })
    }
}

/// Text or a regular expression, matched without regard to case.
fn line_pattern(pattern: &str) -> Option<regex::Regex> {
    regex::RegexBuilder::new(pattern)
        .case_insensitive(true)
        .build()
        .or_else(|_| {
            regex::RegexBuilder::new(&regex::escape(pattern))
                .case_insensitive(true)
                .build()
        })
        .ok()
}

/// `shell` with `wait_for`: hand back when the line appears. With `keep_running` and no
/// `wait_for`, hand back after `KEEP_RUNNING_SETTLE_SECS`, leaving the program running.
async fn watch(
    ctx: &ToolContext,
    a: &ShellArgs,
    pattern: Option<&str>,
    secs: u64,
    program: &str,
    argv: &[String],
) -> ToolOutput {
    let wait_for = match pattern.map(line_pattern) {
        Some(None) => {
            return ToolOutput::error("wait_for isn't usable text or a regular expression");
        }
        Some(Some(w)) => Some(w),
        None => None,
    };
    let secs = if wait_for.is_some() {
        secs
    } else {
        KEEP_RUNNING_SETTLE_SECS
    };
    // What the result says the program was watched for.
    let watched = match pattern {
        Some(p) => format!("Saw \"{p}\""),
        None => format!("It was still going after {secs}s"),
    };
    let pattern = pattern.unwrap_or_default();
    if a.keep_running && crate::background::running_for(&ctx.user_id) >= BACKGROUND_MAX_PER_PERSON {
        return ToolOutput::error(format!(
            "{BACKGROUND_MAX_PER_PERSON} programs are already running in the background: stop one \
             with shell_stop first"
        ));
    }
    let keep = a
        .keep_running
        .then_some((ctx.user_id.as_str(), a.command.as_str()));
    match process::run_until(
        program,
        argv,
        &ctx.workspace,
        Duration::from_secs(secs),
        &ctx.output,
        ctx.cancel.clone(),
        wait_for.as_ref(),
        keep,
    )
    .await
    {
        Ok(process::Watched::Seen(output)) => ToolOutput::ok(
            format!(
                "Saw \"{pattern}\", then stopped the command.\n{}",
                cap(output, RESULT_MAX_CHARS)
            ),
            "Saw what it waited for",
        ),
        Ok(process::Watched::Kept { output, id }) => ToolOutput::ok(
            format!(
                "{watched}. It keeps running in the background as {id}: read its latest \
                 output with shell_output, and stop it with shell_stop when you're done.\n{}",
                cap(output, RESULT_MAX_CHARS)
            ),
            format!("Running in the background ({id})"),
        ),
        Ok(process::Watched::Ended(done)) if done.timed_out => ToolOutput::error(format!(
            "\"{pattern}\" didn't appear within {secs}s, so the command was stopped.\n{}",
            cap(done.output, RESULT_MAX_CHARS)
        )),
        Ok(process::Watched::Ended(done)) => {
            let left = done.left.is_some();
            let mut out = describe_tracked(ctx, &a.command, done, secs);
            out.content = if wait_for.is_some() {
                format!("It ended before \"{pattern}\" appeared.\n{}", out.content)
            } else if left {
                format!("It ended within {secs}s.\n{}", out.content)
            } else {
                format!(
                    "It ended within {secs}s, so nothing was left running.\n{}",
                    out.content
                )
            };
            out
        }
        Err(e) => ToolOutput::error(format!("couldn't start {program}: {e}")),
    }
}

/// The latest output of a program left running with `keep_running`.
pub struct ShellOutput;

impl Tool for ShellOutput {
    fn name(&self) -> &'static str {
        "shell_output"
    }
    fn description(&self) -> &'static str {
        "Read the latest output of a program you left running with shell keep_running (its id \
         looks like bg_1), and whether it's still running."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": { "id": { "type": "string" } },
            "required": ["id"]
        })
    }
    fn title(&self, args: &Value) -> String {
        format!("Reading {}'s output", arg_str(args, "id"))
    }
    fn effect(&self, _ctx: &ToolContext, _args: &Value) -> Effect {
        Effect::Read
    }
    fn run<'a>(&'a self, ctx: &'a ToolContext, args: Value) -> BoxFuture<'a, ToolOutput> {
        Box::pin(async move {
            let id = arg_str(&args, "id");
            match crate::background::output(&ctx.user_id, id, BACKGROUND_READ_MAX_CHARS) {
                None => ToolOutput::error(format!("there's no background program {id}")),
                Some((command, output, ended)) => {
                    let state = match ended {
                        None => "still running".to_owned(),
                        Some(Some(code)) => format!("ended with exit code {code}"),
                        Some(None) => "ended".to_owned(),
                    };
                    let body = if output.trim().is_empty() {
                        "(no output yet)".to_owned()
                    } else {
                        output
                    };
                    ToolOutput::ok(
                        format!("`{command}` is {state}. Latest output:\n{body}"),
                        state,
                    )
                }
            }
        })
    }
}

/// Stop a program left running with `keep_running`.
pub struct ShellStop;

impl Tool for ShellStop {
    fn name(&self) -> &'static str {
        "shell_stop"
    }
    fn description(&self) -> &'static str {
        "Stop a program you left running with shell keep_running (and everything it started)."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": { "id": { "type": "string" } },
            "required": ["id"]
        })
    }
    fn title(&self, args: &Value) -> String {
        format!("Stopping {}", arg_str(args, "id"))
    }
    // Only ever a program Thrall itself started for this person.
    fn effect(&self, _ctx: &ToolContext, _args: &Value) -> Effect {
        Effect::Read
    }
    fn run<'a>(&'a self, ctx: &'a ToolContext, args: Value) -> BoxFuture<'a, ToolOutput> {
        Box::pin(async move {
            let id = arg_str(&args, "id");
            if crate::background::stop(&ctx.user_id, id).await {
                ToolOutput::ok(format!("Stopped {id}."), "Stopped")
            } else {
                ToolOutput::error(format!("there's no background program {id}"))
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
            everyday: false,
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
                Ok(done) => describe(done, GIT_TIMEOUT_SECS, "git", ""),
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

    /// A session folder in the person's own user folder, as on a real install.
    fn ws() -> std::path::PathBuf {
        if cfg!(windows) {
            Path::new(r"C:\Users\me\Documents\Thrall\me").to_path_buf()
        } else {
            Path::new("/home/me/Thrall/me").to_path_buf()
        }
    }

    fn card(command: &str) -> String {
        shell_summary(&json!({ "command": command }), &ws())
    }

    #[test]
    fn shell_grants_and_risk() {
        let ws = ws();
        assert_eq!(
            shell_grant("cargo test -p x", &ws),
            Some("shell:cargo".into())
        );
        assert_eq!(shell_grant("cargo test; rm -rf /", &ws), None);
        assert_eq!(shell_grant("ls | sort", &ws), None);
        // Deleting is never covered by "always".
        assert_eq!(shell_grant("rm -rf target", &ws), None);
        assert_eq!(shell_grant("rm notes.txt", &ws), None);
    }

    #[test]
    fn everyday_commands_are_low_risk() {
        let ws = ws();
        for command in [
            "cargo build",
            "py -V",
            "cd tiny-notes; py server.py",
            "curl.exe -s http://127.0.0.1:8766/health",
            "Get-ItemProperty HKCU:\\Software\\Thrall",
            "py format_rd.py --del-old",
        ] {
            assert_eq!(shell_risk(command, &ws), Risk::Low, "{command}");
        }
    }

    #[test]
    fn deleting_installing_and_stopping_a_program_need_a_look() {
        let ws = ws();
        for command in [
            "Remove-Item body1.json,body2.json",
            "Remove-Item -Recurse -Force build",
            "rm -rf target",
            "pip install requests",
            "npm i left-pad",
            "Stop-Process -Id 20852 -Force",
            "git push origin main",
        ] {
            assert_eq!(shell_risk(command, &ws), Risk::Medium, "{command}");
        }
    }

    #[test]
    fn installs_from_the_official_places_need_only_a_look() {
        let ws = ws();
        for command in [
            "pip install requests pyserial",
            "pip install --target ./lib requests",
            "npm install left-pad",
            "npm install -E left-pad",
            "npm i @types/node",
            "npm install left-pad && cd src/app",
            "cargo install ripgrep",
            "winget install Git.Git --source winget",
            "winget install 9NBLGGH4NNS1 -s msstore",
            "choco install git",
            "Install-Module PSReadLine -Repository PSGallery",
        ] {
            assert_eq!(shell_risk(command, &ws), Risk::Medium, "{command}");
            assert!(
                !shell_summary(&json!({ "command": command }), &ws)
                    .starts_with(SHELL_CUSTOM_SOURCE_NOTE)
            );
        }
    }

    #[test]
    fn installs_from_anywhere_else_always_ask() {
        let ws = ws();
        for command in [
            "pip install https://example.com/pkg-1.0.tar.gz",
            "pip install git+https://example.com/me/pkg",
            "pip install --index-url https://example.com/simple pkg",
            "pip install --extra-index-url=https://example.com/simple pkg",
            "pip install -i https://example.com/simple pkg",
            "pip install -f ./wheels pkg",
            "pip install ./pkg-1.0-py3-none-any.whl",
            "npm install github:me/pkg",
            "npm i ./pkg-1.0.0.tgz",
            "npm install pkg --registry https://example.com",
            "cargo install --git https://example.com/me/tool",
            "cargo install tool --index sparse+https://example.com/index/",
            "winget install pkg --source mysource",
            "choco install pkg -s https://example.com/feed",
            "Install-Module pkg -Repository MyFeed",
            "npm i me/pkg",
            "yarn add me/pkg#main",
            "pip install .",
            "pip install -e .",
            "python -m pip install ..\\tool",
            "npm install ./folder",
            "cargo install --path .",
            "pip install -r requirements-that-are-not-there.txt",
        ] {
            assert_eq!(shell_risk(command, &ws), Risk::High, "{command}");
            assert!(
                shell_summary(&json!({ "command": command }), &ws)
                    .starts_with(SHELL_CUSTOM_SOURCE_NOTE),
                "{command}"
            );
        }
    }

    #[test]
    fn a_requirements_file_is_read_for_where_it_installs_from() {
        let ws = std::env::temp_dir().join(format!("thrall-requirements-{}", std::process::id()));
        std::fs::create_dir_all(&ws).unwrap();
        let files = [
            (
                "plain.txt",
                "# tools\nrequests>=2.0  # http\npyserial==3.5\n\n-r more.txt\n",
            ),
            ("more.txt", "mido\n"),
            (
                "index.txt",
                "--index-url https://example.com/simple\nrequests\n",
            ),
            (
                "git.txt",
                "requests\npkg @ git+https://example.com/me/pkg\n",
            ),
            ("folder.txt", "-e .\n"),
            ("nested.txt", "-r git.txt\n"),
        ];
        for (name, text) in files {
            std::fs::write(ws.join(name), text).unwrap();
        }
        let risk = |file: &str| shell_risk(&format!("pip install -r {file}"), &ws);
        assert_eq!(risk("plain.txt"), Risk::Medium);
        assert_eq!(
            shell_risk("pip install -c plain.txt requests", &ws),
            Risk::Medium
        );
        for file in ["index.txt", "git.txt", "folder.txt", "nested.txt"] {
            assert_eq!(risk(file), Risk::High, "{file}");
        }
        std::fs::remove_dir_all(&ws).unwrap();
    }

    #[test]
    fn risky_commands_are_high_risk() {
        let ws = ws();
        let elsewhere = if cfg!(windows) {
            r"Remove-Item -Recurse -Force C:\Users\me\Projects\old"
        } else {
            "rm -rf /home/me/Projects/old"
        };
        for command in [
            elsewhere,
            "Remove-Item -Recurse ..\\other",
            "cd ..; rm -r build",
            "Remove-Item -Recurse $env:TEMP\\x",
            "Stop-Process -Name python",
            "taskkill /IM node.exe /F",
            "git push --force",
            "Restart-Computer",
            "Format-Volume -DriveLetter E",
            "reg add HKCU\\Software\\X /v Y /d 1",
            "Set-ItemProperty -Path HKLM:\\Software\\X -Name Y -Value 1",
        ] {
            assert_eq!(shell_risk(command, &ws), Risk::High, "{command}");
        }
    }

    #[test]
    fn a_delete_chained_onto_something_harmless_is_named_first() {
        let first = card("Remove-Item old.py -ErrorAction SilentlyContinue; py -m pytest -q");
        assert!(
            first.starts_with("Deletes files: Run `Remove-Item"),
            "{first}"
        );
        assert!(card("cargo test && rm notes.txt").starts_with("Deletes files: "));
        assert!(card("git reset --hard").starts_with("Risky: "));
        assert_eq!(card("cargo build"), "Run `cargo build`");
        // Words that only contain a deleter's name don't count.
        assert_eq!(
            card("py format_rd.py --del-old"),
            "Run `py format_rd.py --del-old`"
        );
    }

    #[test]
    fn a_system_change_needs_its_outcome_and_a_backup_to_undo_it() {
        let ws = std::env::temp_dir().join("thrall-system-change");
        std::fs::create_dir_all(&ws).unwrap();
        let backup = ws.join("before.reg");
        let _ = std::fs::remove_file(&backup);
        let command = "reg add HKCU\\Software\\ThrallTest /v Shown /d 1 /f";
        let with = |outcome: &str, undo: &str| json!({ "command": command, "outcome": outcome, "undo": undo });
        // Nothing said, or a backup that isn't there: refused before anyone is asked.
        assert!(system_change_blocked(&json!({ "command": command }), &ws).is_some());
        let args = with("Shows the test value", "reg import before.reg");
        assert!(system_change_blocked(&args, &ws).is_some());
        // With the backup made, it can be asked about, and the card says both.
        std::fs::write(&backup, "Windows Registry Editor Version 5.00").unwrap();
        assert_eq!(system_change_blocked(&args, &ws), None);
        let shown = shell_summary(&args, &ws);
        assert!(
            shown.starts_with("Changes the system: Run `reg add"),
            "{shown}"
        );
        assert!(shown.contains("Outcome: Shows the test value"), "{shown}");
        assert!(shown.contains("Undo: reg import before.reg"), "{shown}");
        // Reading the registry, or an ordinary command, needs none of it.
        let read = json!({ "command": "reg query HKCU\\Software\\ThrallTest" });
        assert_eq!(system_change_blocked(&read, &ws), None);
        let _ = std::fs::remove_dir_all(&ws);
    }

    #[cfg(windows)]
    #[test]
    fn changing_files_in_the_system_folders_is_a_system_change() {
        let ws = ws();
        assert!(system_change(
            r"Copy-Item hosts C:\Windows\System32\drivers\etc\hosts",
            &ws
        ));
        assert!(system_change(r"echo x > C:\Windows\x.txt", &ws));
        // Only looking isn't.
        assert!(!system_change(
            r"Get-Content C:\Windows\System32\drivers\etc\hosts",
            &ws
        ));
        assert!(!system_change(r"Copy-Item a.txt b.txt", &ws));
    }

    fn finished(code: Option<i32>, timed_out: bool) -> Finished {
        Finished {
            code,
            output: "done\n".into(),
            timed_out,
            cancelled: false,
            left: None,
        }
    }

    #[test]
    fn a_nonzero_exit_is_a_warning_and_a_timeout_a_failure() {
        let ok = describe(finished(Some(0), false), 60, "The command", "dir");
        assert!(ok.ok && !ok.warn);
        // It ran to the end: its output may still be what was asked.
        let exit1 = describe(finished(Some(1), false), 60, "The command", "dir");
        assert!(!exit1.ok && exit1.warn);
        assert_eq!(exit1.summary, "Exit code 1");
        assert!(exit1.content.contains("done"));
        let late = describe(finished(None, true), 60, "The command", "dir");
        assert!(!late.ok && !late.warn);
        let killed = describe(finished(None, false), 60, "The command", "dir");
        assert!(!killed.ok && !killed.warn);
    }

    #[test]
    fn a_stop_warns_of_escaped_programs_only_when_the_command_starts_one() {
        let search = describe(
            finished(None, true),
            180,
            "The command",
            "Get-ChildItem -Recurse",
        );
        assert!(search.content.contains(STOPPED_NOTE));
        assert!(!search.content.contains("may still be running"));
        let launch = describe(
            finished(None, true),
            180,
            "The command",
            "Start-Process app.exe",
        );
        assert!(launch.content.contains("may still be running"));
    }

    #[test]
    fn only_starting_a_program_counts_not_naming_it() {
        for started in [
            "Start-Process python -ArgumentList app.py",
            "$p = Start-Process python -PassThru; $p.Id",
            "(Start-Process notepad -PassThru).Id",
            "cd app; .venv\\Scripts\\pythonw.exe cast.py",
            "& \"C:\\My App\\.venv\\Scripts\\pythonw.exe\" cast.py",
            "start /b server.exe",
            "nohup ./server &",
        ] {
            assert!(starts_untracked(started), "{started}");
        }
        for named in [
            "Get-Process pythonw | Select-Object Id, StartTime",
            "Get-Help Start-Process",
            "Stop-Process -Name pythonw",
            "Start-Sleep 2",
            "Get-Content start.log",
        ] {
            assert!(!starts_untracked(named), "{named}");
        }
    }
}
