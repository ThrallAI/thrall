//! Thrall checks its own work before a turn ends (Python Thrall lessons: models claim work they
//! never did, and say "done" without looking).
//!
//! - **Verify step**: files were changed but nothing was read or run afterwards → Thrall is asked
//!   to check its work once before finishing.
//! - **Claim check**: the final reply says a file was created or changed, but no change to that
//!   file happened this turn → Thrall is asked to correct the reply, once.

use std::sync::LazyLock;

use regex::Regex;
use thrall_core::config::AgentConfig;

use crate::constants::{CHECK_PREFIX, CLAIM_PATTERN};

/// What happened this turn, for the checks.
#[derive(Debug, Default)]
pub struct Track {
    /// Files changed this turn (as shown to the user).
    pub edited: Vec<String>,
    /// Something was read or run since the last change.
    pub checked_since_edit: bool,
    verify_asked: bool,
    claim_asked: bool,
    /// The user approved a restart.
    pub restart: bool,
    /// These steps are a helper's (skills run inline there, not in another helper).
    pub in_helper: bool,
    /// These steps are split mode's Thinker's: it may only look.
    pub look_only: bool,
    /// Thrall already put a question card to the person this task.
    pub asked: bool,
}

impl Track {
    /// A file was changed.
    pub fn edited(&mut self, path: &str) {
        if !self.edited.iter().any(|p| p == path) {
            self.edited.push(path.to_owned());
        }
        self.checked_since_edit = false;
    }

    /// Something was read or run.
    pub fn checked(&mut self) {
        self.checked_since_edit = true;
    }
}

static CLAIMS: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new(CLAIM_PATTERN).ok());

/// Files the reply says were created or changed.
pub fn claimed_files(reply: &str) -> Vec<String> {
    let Some(re) = CLAIMS.as_ref() else {
        return Vec::new();
    };
    let mut out: Vec<String> = Vec::new();
    for caps in re.captures_iter(reply) {
        let Some(path) = caps.name("path").or_else(|| caps.name("path2")) else {
            continue;
        };
        let path = path
            .as_str()
            .trim_matches(|c| c == '`' || c == '\'' || c == '"');
        // Needs a letter in the extension, so "1.5" or "v2.0" don't count as files.
        let has_ext = path
            .rsplit_once('.')
            .is_some_and(|(_, ext)| ext.chars().any(|c| c.is_ascii_alphabetic()));
        if has_ext && !out.iter().any(|p| p == path) {
            out.push(path.to_owned());
        }
    }
    out
}

fn same_file(claimed: &str, edited: &str) -> bool {
    let norm = |p: &str| p.replace('\\', "/").trim_start_matches("./").to_lowercase();
    let (c, e) = (norm(claimed), norm(edited));
    c == e || e.ends_with(&format!("/{c}")) || c.ends_with(&format!("/{e}"))
}

/// After a final reply: a message asking Thrall to check or correct itself, or `None` to finish.
pub fn after_reply(config: &AgentConfig, track: &mut Track, reply: &str) -> Option<String> {
    if config.verify && !track.edited.is_empty() && !track.checked_since_edit && !track.verify_asked
    {
        track.verify_asked = true;
        return Some(format!(
            "{CHECK_PREFIX} You changed {} but haven't looked at the result since. Check your \
             work now (re-read what you changed, or run the relevant test or build). Fix anything \
             wrong, then give your final answer.",
            track.edited.join(", ")
        ));
    }
    if config.claim_check && !track.claim_asked {
        let unbacked: Vec<String> = claimed_files(reply)
            .into_iter()
            .filter(|c| !track.edited.iter().any(|e| same_file(c, e)))
            .collect();
        if !unbacked.is_empty() {
            track.claim_asked = true;
            return Some(format!(
                "{CHECK_PREFIX} Your reply says {} was created or changed, but no change to it \
                 happened in this turn. If the change happened in an earlier turn, say so; if it \
                 didn't happen, correct your reply (or make the change). Reply again with an \
                 accurate final answer.",
                unbacked.join(", ")
            ));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> AgentConfig {
        AgentConfig::default()
    }

    #[test]
    fn claims_are_found() {
        let found = claimed_files(
            "I've created `src/new.rs` and updated README.md. Version 1.5 is out; \
             config.toml has been updated.",
        );
        assert_eq!(found, vec!["src/new.rs", "README.md", "config.toml"]);
        assert!(claimed_files("The file README.md describes the project.").is_empty());
    }

    #[test]
    fn verify_asks_once_after_unchecked_edits() {
        let mut t = Track::default();
        t.edited("a.rs");
        assert!(after_reply(&config(), &mut t, "Done.").is_some());
        assert!(after_reply(&config(), &mut t, "Done.").is_none());

        let mut checked = Track::default();
        checked.edited("a.rs");
        checked.checked();
        assert!(after_reply(&config(), &mut checked, "Done.").is_none());
    }

    #[test]
    fn false_claims_are_caught_once() {
        let mut t = Track::default();
        let nudge = after_reply(&config(), &mut t, "I created answer.txt for you.");
        assert!(nudge.is_some_and(|n| n.contains("answer.txt")));
        assert!(after_reply(&config(), &mut t, "I created answer.txt for you.").is_none());
    }

    #[test]
    fn true_claims_pass() {
        let mut t = Track::default();
        t.edited("notes/answer.txt");
        t.checked();
        assert!(after_reply(&config(), &mut t, "I wrote `answer.txt`.").is_none());
    }

    #[test]
    fn checks_can_be_switched_off() {
        let off = AgentConfig {
            verify: false,
            claim_check: false,
            ..AgentConfig::default()
        };
        let mut t = Track::default();
        t.edited("a.rs");
        assert!(after_reply(&off, &mut t, "I created b.rs").is_none());
    }
}
