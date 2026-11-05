//! The five safety gates and the permission modes.
//!
//! Every message, tool call, reply and memory write passes one of these before it goes anywhere.
//! The gates only decide; the agent carries out the decision and writes it to the audit log.
//!
//! 1. **input** — what the user sends: control characters removed, size limited.
//! 2. **context** — what enters the model's context: secrets in tool results removed.
//! 3. **tool** — whether a tool call runs, asks first, or is refused (by permission mode).
//! 4. **output** — what Thrall says: secrets removed from replies.
//! 5. **memory** — what reaches long-term memory: facts containing secrets are refused.

pub mod constants;

use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};
use thrall_protocol::{Diff, Risk};

use crate::constants::{
    INPUT_MAX_CHARS, REDACTED, SECRET_PATTERNS, SHARED_USERS, SYSTEM_FOLDERS, USER_ROOTS,
};

// ---------- permission modes ----------

/// How much Thrall may do without asking (protocol §9).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// Ask before edits and commands.
    #[default]
    Ask,
    /// File edits inside the folder go ahead; commands still ask.
    AutoEdits,
    /// Read-only until a plan is approved.
    Plan,
    /// No questions, except for actions that always ask.
    FullAuto,
}

impl Mode {
    /// The stored and wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ask => "ask",
            Self::AutoEdits => "auto_edits",
            Self::Plan => "plan",
            Self::FullAuto => "full_auto",
        }
    }

    /// Parse a stored name or something a person typed.
    pub fn parse(text: &str) -> Option<Self> {
        match text
            .trim()
            .to_ascii_lowercase()
            .replace(['-', ' '], "_")
            .as_str()
        {
            "ask" | "default" | "normal" => Some(Self::Ask),
            "auto_edits" | "auto" | "edits" | "accept_edits" => Some(Self::AutoEdits),
            "plan" | "planning" => Some(Self::Plan),
            "full_auto" | "full" => Some(Self::FullAuto),
            _ => None,
        }
    }

    /// Plain-language label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Ask => "ask before changes",
            Self::AutoEdits => "auto-accept edits",
            Self::Plan => "plan mode (read-only)",
            Self::FullAuto => "full auto",
        }
    }

    /// What the mode lets Thrall do without asking, in one plain sentence (shown when it's picked).
    pub fn covers(self) -> &'static str {
        match self {
            Self::Ask => "Thrall asks before every file edit and every command.",
            Self::AutoEdits => {
                "File edits in the folder go ahead without asking; commands still ask."
            }
            Self::Plan => "Thrall only reads and plans; nothing changes until you approve a plan.",
            Self::FullAuto => {
                "Edits and commands go ahead without asking, risky ones too (restarts, sending \
                 and deleting outside the folder still ask)."
            }
        }
    }

    /// The next mode for the quick toggle (Shift+Tab). Full auto is never reached by toggling.
    pub fn next(self) -> Self {
        match self {
            Self::Ask => Self::AutoEdits,
            Self::AutoEdits => Self::Plan,
            Self::Plan | Self::FullAuto => Self::Ask,
        }
    }
}

// ---------- tool gate ----------

/// What a tool call would do, as the tool describes it. (Serialised when a companion PC works it
/// out for a tool that runs there.)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Effect {
    /// Only reads (files, the web, memory).
    Read,
    /// Changes a file inside the working folder.
    Edit {
        /// File, as shown to the user.
        path: String,
        /// The change, when it could be worked out in advance.
        diff: Option<Diff>,
    },
    /// Runs something.
    Run {
        /// Plain language: "Run `cargo test`".
        summary: String,
        /// How risky.
        risk: Risk,
        /// What an "always" answer covers (e.g. `shell:cargo`). `None` = never "always".
        grant: Option<String>,
    },
    /// Runs something that may only look (a tool on another app's MCP server that doesn't say
    /// it's read-only). Like [`Effect::Run`], except that plan mode asks instead of refusing:
    /// planning often needs a look at what an app offers.
    Probe {
        /// Plain language.
        summary: String,
        /// How risky.
        risk: Risk,
        /// What an "always" answer covers. `None` = never "always".
        grant: Option<String>,
    },
    /// Asks every time, whatever the mode (restarts, sending, deleting outside the folder).
    AlwaysAsk {
        /// Plain language.
        summary: String,
        /// How risky.
        risk: Risk,
        /// What it would write, when that helps the person decide (a proposed skill).
        diff: Option<Diff>,
    },
}

/// The tool gate's answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// Go ahead.
    Allow,
    /// Ask the user; `always` says whether "always" may be offered.
    Ask {
        /// Offer "always".
        always: bool,
    },
    /// Refuse, with a reason for the model.
    Deny(String),
}

/// The grant key edits share: one "always" covers every edit in the session.
pub const EDIT_GRANT: &str = "edit";

/// Decide a tool call. `granted` says whether an earlier "always" covers it.
pub fn tool_gate(mode: Mode, effect: &Effect, granted: bool) -> Decision {
    match effect {
        Effect::Read => Decision::Allow,
        Effect::Edit { path, .. } => match mode {
            Mode::Plan => Decision::Deny(plan_refusal(&format!("edit {path}"))),
            Mode::AutoEdits | Mode::FullAuto => Decision::Allow,
            Mode::Ask if granted => Decision::Allow,
            Mode::Ask => Decision::Ask { always: true },
        },
        Effect::Run {
            summary,
            risk,
            grant,
        } => match mode {
            Mode::Plan => Decision::Deny(plan_refusal(summary)),
            Mode::FullAuto => Decision::Allow,
            // Risky actions are never covered by "always" (a Python Thrall lesson).
            Mode::Ask | Mode::AutoEdits if granted && *risk != Risk::High => Decision::Allow,
            Mode::Ask | Mode::AutoEdits => Decision::Ask {
                always: grant.is_some() && *risk != Risk::High,
            },
        },
        Effect::Probe {
            summary,
            risk,
            grant,
        } => match mode {
            Mode::Plan if granted && *risk != Risk::High => Decision::Allow,
            Mode::Plan => Decision::Ask {
                always: grant.is_some() && *risk != Risk::High,
            },
            _ => tool_gate(
                mode,
                &Effect::Run {
                    summary: summary.clone(),
                    risk: *risk,
                    grant: grant.clone(),
                },
                granted,
            ),
        },
        Effect::AlwaysAsk { .. } => Decision::Ask { always: false },
    }
}

fn plan_refusal(what: &str) -> String {
    format!(
        "plan mode is on, so nothing can be changed yet ({what}). Finish looking around, then \
         present your plan with exit_plan_mode and wait for approval."
    )
}

// ---------- secrets (context, output, memory gates) ----------

static SECRETS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    SECRET_PATTERNS
        .iter()
        .filter_map(|p| Regex::new(p).ok())
        .collect()
});

/// Whether `text` contains something shaped like a key or token.
pub fn has_secret(text: &str) -> bool {
    SECRETS.iter().any(|re| re.is_match(text))
}

/// `text` with keys and tokens replaced, and how many were found.
pub fn redact(text: &str) -> (String, usize) {
    let mut out = text.to_owned();
    let mut found = 0;
    for re in SECRETS.iter() {
        let hits = re.find_iter(&out).count();
        if hits > 0 {
            found += hits;
            out = re.replace_all(&out, REDACTED).into_owned();
        }
    }
    (out, found)
}

// ---------- input and memory gates ----------

/// What the input gate did with a message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Input {
    /// Use this text (cleaned of control characters).
    Ok(String),
    /// Refuse, with a reason for the user.
    Refused(String),
}

/// Clean what the user sent: no control characters except newlines and tabs, not too long.
pub fn input_gate(text: &str) -> Input {
    if text.chars().count() > INPUT_MAX_CHARS {
        return Input::Refused(format!(
            "That message is over {INPUT_MAX_CHARS} characters. Put long content in a file and \
             ask Thrall to read it."
        ));
    }
    let cleaned: String = text
        .replace("\r\n", "\n")
        .chars()
        .filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
        .collect();
    if cleaned.trim().is_empty() {
        return Input::Refused("That message is empty.".to_owned());
    }
    Input::Ok(cleaned)
}

/// The memory gate: facts containing secrets are refused.
pub fn memory_gate(content: &str) -> Result<(), String> {
    if has_secret(content) {
        Err(
            "that looks like it contains a key or token; secrets are never saved to memory"
                .to_owned(),
        )
    } else {
        Ok(())
    }
}

// ---------- system folders ----------

/// `path` (relative to `workspace`, or absolute) with `.` and `..` worked out, `\` for `/`, lower
/// case: the same on Windows, Linux and macOS for comparing.
fn plain_path(path: &str, workspace: &str) -> String {
    let path = path.trim().replace('/', "\\");
    let absolute = path.starts_with('\\') || path.get(1..2) == Some(":");
    let full = if absolute {
        path
    } else {
        format!(
            "{}\\{path}",
            workspace.trim().replace('/', "\\").trim_end_matches('\\')
        )
    };
    let mut parts: Vec<&str> = Vec::new();
    for part in full.split('\\') {
        match part {
            "" | "." if !parts.is_empty() => {}
            ".." => {
                if parts.len() > 1 {
                    parts.pop();
                }
            }
            p => parts.push(p),
        }
    }
    parts.join("\\").to_lowercase()
}

/// The person's own user folder, from their working folder (`c:\users\you\documents\thrall` →
/// `you`).
fn user_of(path: &str) -> Option<&str> {
    let rest = USER_ROOTS.iter().find_map(|root| path.strip_prefix(root))?;
    rest.split('\\').next().filter(|u| !u.is_empty())
}

/// Whether `path` is in the system's folders (Windows, programs, the boot and system folders,
/// someone else's user folder) for a person working in `workspace`.
pub fn system_folder(path: &str, workspace: &str) -> bool {
    let path = plain_path(path, workspace);
    let starts = |prefix: &&str| path == *prefix || path.starts_with(&format!("{prefix}\\"));
    if SYSTEM_FOLDERS.iter().any(starts) {
        return true;
    }
    let mine = user_of(&plain_path("", workspace)).map(str::to_owned);
    match user_of(&path) {
        Some(theirs) => !SHARED_USERS.contains(&theirs) && mine.as_deref() != Some(theirs),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(risk: Risk, grant: Option<&str>) -> Effect {
        Effect::Run {
            summary: "Run `x`".into(),
            risk,
            grant: grant.map(str::to_owned),
        }
    }

    #[test]
    fn reads_always_go_ahead() {
        for mode in [Mode::Ask, Mode::AutoEdits, Mode::Plan, Mode::FullAuto] {
            assert_eq!(tool_gate(mode, &Effect::Read, false), Decision::Allow);
        }
    }

    #[test]
    fn edits_follow_the_mode() {
        let edit = Effect::Edit {
            path: "a.rs".into(),
            diff: None,
        };
        assert_eq!(
            tool_gate(Mode::Ask, &edit, false),
            Decision::Ask { always: true }
        );
        assert_eq!(tool_gate(Mode::Ask, &edit, true), Decision::Allow);
        assert_eq!(tool_gate(Mode::AutoEdits, &edit, false), Decision::Allow);
        assert!(matches!(
            tool_gate(Mode::Plan, &edit, true),
            Decision::Deny(_)
        ));
    }

    #[test]
    fn plan_mode_asks_about_a_probe_instead_of_refusing() {
        let probe = Effect::Probe {
            summary: "Use list_commands on the cadcraft MCP server".into(),
            risk: Risk::Medium,
            grant: Some("mcp:cadcraft:list_commands".into()),
        };
        assert_eq!(
            tool_gate(Mode::Plan, &probe, false),
            Decision::Ask { always: true }
        );
        assert_eq!(tool_gate(Mode::Plan, &probe, true), Decision::Allow);
        // Everywhere else it's an ordinary command.
        assert_eq!(
            tool_gate(Mode::Ask, &probe, false),
            Decision::Ask { always: true }
        );
        assert_eq!(tool_gate(Mode::FullAuto, &probe, false), Decision::Allow);
        // A plain command is still refused while planning.
        assert!(matches!(
            tool_gate(Mode::Plan, &run(Risk::Medium, Some("x")), false),
            Decision::Deny(_)
        ));
    }

    #[test]
    fn commands_ask_and_risky_ones_never_offer_always() {
        assert_eq!(
            tool_gate(
                Mode::AutoEdits,
                &run(Risk::Medium, Some("shell:cargo")),
                false
            ),
            Decision::Ask { always: true }
        );
        assert_eq!(
            tool_gate(Mode::Ask, &run(Risk::High, Some("shell:rm")), true),
            Decision::Ask { always: false }
        );
        assert_eq!(
            tool_gate(Mode::FullAuto, &run(Risk::High, None), false),
            Decision::Allow
        );
    }

    #[test]
    fn always_ask_asks_even_in_full_auto() {
        let restart = Effect::AlwaysAsk {
            summary: "Restart".into(),
            risk: Risk::Medium,
            diff: None,
        };
        assert_eq!(
            tool_gate(Mode::FullAuto, &restart, true),
            Decision::Ask { always: false }
        );
    }

    #[test]
    fn modes_parse_and_cycle() {
        assert_eq!(Mode::parse("auto"), Some(Mode::AutoEdits));
        assert_eq!(Mode::parse("full-auto"), Some(Mode::FullAuto));
        assert_eq!(Mode::Plan.next(), Mode::Ask);
        assert_eq!(Mode::parse(Mode::AutoEdits.as_str()), Some(Mode::AutoEdits));
    }

    #[test]
    fn secrets_are_found_and_redacted() {
        let text = "key=sk-or-v1-abcdefghijklmnopqrstuvwxyz0123456789 and \
                    ghp_abcdefghijklmnopqrstuvwxyz0123456789";
        assert!(has_secret(text));
        let (clean, n) = redact(text);
        assert_eq!(n, 2);
        assert!(!clean.contains("sk-or"));
        assert!(!has_secret("a normal sentence about sketching tasks"));
    }

    #[test]
    fn input_is_cleaned_and_limited() {
        assert_eq!(
            input_gate("hi\u{7}\r\nthere"),
            Input::Ok("hi\nthere".into())
        );
        assert!(matches!(input_gate("   "), Input::Refused(_)));
        assert!(matches!(
            input_gate(&"x".repeat(INPUT_MAX_CHARS + 1)),
            Input::Refused(_)
        ));
    }

    #[test]
    fn memory_refuses_secrets() {
        assert!(memory_gate("prefers dark mode").is_ok());
        assert!(memory_gate("my key is sk-ant-abcdefghijklmnopqrstuvwxyz123456").is_err());
    }
}

#[cfg(test)]
mod system_folder_tests {
    use super::system_folder;

    #[test]
    fn system_and_other_peoples_folders_are_out_of_bounds() {
        let ws = r"C:\Users\you\Documents\Thrall";
        assert!(!system_folder("notes.txt", ws));
        assert!(!system_folder(r"C:\Users\you\Desktop\a.txt", ws));
        assert!(!system_folder(r"C:\Users\Public\a.txt", ws));
        assert!(system_folder(r"C:\Windows\System32\drivers\etc\hosts", ws));
        assert!(system_folder(r"c:/program files/app/x.dll", ws));
        assert!(system_folder(r"C:\Users\me\Documents\secret.txt", ws));
        assert!(
            system_folder(r"..\..\..\me\x.txt", ws),
            "climbing out with .."
        );
        assert!(
            !system_folder(r"C:\Windowsish\a.txt", ws),
            "only the folder itself"
        );
        assert!(system_folder("/etc/passwd", "/home/you/Thrall"));
        assert!(!system_folder("notes.txt", "/home/you/Thrall"));
    }
}
