//! Trimming old tool results: a big result from several exchanges ago is sent to the model
//! shortened, with a note to run the tool again if it's needed. Only what the model is sent
//! changes; the stored message keeps every character. Off unless `memory.trim_old_tool_results`
//! is on. One small, tested function decides, like compaction.

use crate::constants::{TRIM_AFTER_USER_MESSAGES, TRIM_KEEP_CHARS, TRIM_MIN_CHARS};

/// What trimming needs to know about one context message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Item {
    /// A user message (each one starts a new exchange).
    pub is_user: bool,
    /// A tool result, with its length in characters.
    pub tool_chars: Option<usize>,
}

/// Which messages to send trimmed: tool results of at least [`TRIM_MIN_CHARS`] characters with
/// at least [`TRIM_AFTER_USER_MESSAGES`] user messages after them. The current exchange and the
/// few before it are never touched.
pub fn plan(items: &[Item]) -> Vec<usize> {
    let mut users_after = 0;
    let mut trim = Vec::new();
    for (i, item) in items.iter().enumerate().rev() {
        if item.is_user {
            users_after += 1;
        } else if let Some(chars) = item.tool_chars
            && chars >= TRIM_MIN_CHARS
            && users_after >= TRIM_AFTER_USER_MESSAGES
        {
            trim.push(i);
        }
    }
    trim.reverse();
    trim
}

/// The shortened text sent in place of a long tool result: its start and a note.
pub fn trimmed(text: &str) -> String {
    let total = text.chars().count();
    let start: String = text.chars().take(TRIM_KEEP_CHARS).collect();
    format!(
        "{start}\n\n[Trimmed by Thrall: this older tool result was {total} characters and you've \
         already used it. Only the start is shown. If you need the rest, run the tool again.]"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user() -> Item {
        Item {
            is_user: true,
            tool_chars: None,
        }
    }
    fn reply() -> Item {
        Item {
            is_user: false,
            tool_chars: None,
        }
    }
    fn tool(chars: usize) -> Item {
        Item {
            is_user: false,
            tool_chars: Some(chars),
        }
    }

    #[test]
    fn old_big_results_are_trimmed() {
        // A big result, then three more exchanges: it's old enough.
        let items = [
            user(),
            reply(),
            tool(50_000),
            reply(),
            user(),
            reply(),
            user(),
            reply(),
            user(),
        ];
        assert_eq!(plan(&items), vec![2]);
    }

    #[test]
    fn recent_results_are_left_alone() {
        // Only two user messages after it: still in use.
        let items = [user(), tool(50_000), reply(), user(), reply(), user()];
        assert!(plan(&items).is_empty());
        // The current exchange's results are never trimmed, however big.
        let items = [user(), tool(500_000), tool(500_000)];
        assert!(plan(&items).is_empty());
    }

    #[test]
    fn small_results_are_left_alone() {
        let items = [user(), tool(TRIM_MIN_CHARS - 1), user(), user(), user()];
        assert!(plan(&items).is_empty());
        let items = [user(), tool(TRIM_MIN_CHARS), user(), user(), user()];
        assert_eq!(plan(&items), vec![1]);
    }

    #[test]
    fn the_note_keeps_the_start_and_says_what_happened() {
        let text = "a".repeat(TRIM_KEEP_CHARS + 500);
        let short = trimmed(&text);
        assert!(short.starts_with(&"a".repeat(TRIM_KEEP_CHARS)));
        assert!(!short.contains(&"a".repeat(TRIM_KEEP_CHARS + 1)));
        assert!(short.contains(&format!("{} characters", TRIM_KEEP_CHARS + 500)));
        assert!(short.contains("run the tool again"));
    }
}
