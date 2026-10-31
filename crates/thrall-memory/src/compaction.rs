//! Deciding when and where to compact a long conversation. One small, tested function decides —
//! a Python Thrall lesson: a pruning bug there once wiped every tool result mid-task.

use crate::constants::{
    CHARS_PER_TOKEN, COMPACT_AT_PERCENT, COMPACT_MIN_GAIN_PERCENT, KEEP_RECENT_MESSAGES,
    TOKEN_SCALE_MAX_PERCENT, TOKEN_SCALE_MIN_PERCENT,
};

/// What compaction needs to know about one context message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sized {
    /// A user message (a safe place to cut: it never sits between a tool call and its result).
    pub is_user: bool,
    /// Estimated tokens.
    pub tokens: usize,
}

/// Rough token count of some text.
pub fn estimate_tokens(text: &str) -> usize {
    text.chars().count().div_ceil(CHARS_PER_TOKEN)
}

/// Real tokens per 100 estimated, from one call: the provider `reported` this many input tokens
/// for a context estimated at `estimated`. `None` when either is unknown (zero). Kept between
/// [`TOKEN_SCALE_MIN_PERCENT`] and [`TOKEN_SCALE_MAX_PERCENT`] so one odd count can't run wild.
pub fn token_scale(estimated: usize, reported: usize) -> Option<usize> {
    if estimated == 0 || reported == 0 {
        return None;
    }
    Some((reported * 100 / estimated).clamp(TOKEN_SCALE_MIN_PERCENT, TOKEN_SCALE_MAX_PERCENT))
}

/// An estimate corrected by a scale from [`token_scale`].
pub fn scaled(tokens: usize, scale_percent: usize) -> usize {
    (tokens * scale_percent).div_ceil(100)
}

/// How many of the oldest messages to summarise, or `None` if the context fits.
///
/// Compacts once the context reaches [`COMPACT_AT_PERCENT`] of `limit` tokens. Always keeps the
/// last [`KEEP_RECENT_MESSAGES`] messages, and cuts just before a user message so a tool call is
/// never separated from its result. Returns `None` rather than a cut that would summarise nothing,
/// or (unless forced: `limit` 0) one that would summarise less than [`COMPACT_MIN_GAIN_PERCENT`]
/// of the context, such as only the previous summary when the recent messages alone are big.
pub fn plan(messages: &[Sized], extra_tokens: usize, limit: usize) -> Option<usize> {
    let total: usize = extra_tokens + messages.iter().map(|m| m.tokens).sum::<usize>();
    if total * 100 < limit * COMPACT_AT_PERCENT {
        return None;
    }
    let latest_cut = messages.len().checked_sub(KEEP_RECENT_MESSAGES)?;
    let cut = (1..=latest_cut).rev().find(|&i| messages[i].is_user)?;
    let summarised: usize = messages[..cut].iter().map(|m| m.tokens).sum();
    if limit > 0 && summarised * 100 < total * COMPACT_MIN_GAIN_PERCENT {
        return None;
    }
    Some(cut)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conv(pattern: &str, tokens: usize) -> Vec<Sized> {
        pattern
            .chars()
            .map(|c| Sized {
                is_user: c == 'u',
                tokens,
            })
            .collect()
    }

    #[test]
    fn small_contexts_are_left_alone() {
        assert_eq!(plan(&conv("uauauauauauauaua", 10), 0, 10_000), None);
    }

    #[test]
    fn big_contexts_cut_before_a_user_message_keeping_the_recent_ones() {
        // u a t t a u a u a u a u a u a u a  (17 messages, t = tool results)
        let msgs = conv("uattauauauauauaua", 1000);
        let cut = plan(&msgs, 0, 10_000).unwrap();
        assert!(msgs[cut].is_user, "cut lands on a user message");
        assert!(
            msgs.len() - cut >= KEEP_RECENT_MESSAGES,
            "recent messages kept"
        );
        assert!(cut > 0);
    }

    #[test]
    fn never_cuts_through_a_tool_run() {
        // One user message, then a long tool run: there's no safe cut, so nothing is compacted.
        let msgs = conv("uatatatatatatatatatat", 1000);
        assert_eq!(plan(&msgs, 0, 10_000), None);
    }

    #[test]
    fn the_system_prompt_counts() {
        let msgs = conv("uauauauauauauauaua", 40);
        assert_eq!(plan(&msgs, 0, 1000), None);
        assert!(plan(&msgs, 600, 1000).is_some());
    }

    #[test]
    fn the_scale_follows_what_the_provider_counted() {
        // The 2026-10-28 case: ~2 characters per token, so twice the estimate.
        assert_eq!(token_scale(95_000, 190_000), Some(200));
        assert_eq!(token_scale(1000, 1000), Some(100));
        // Nothing counted yet: no correction.
        assert_eq!(token_scale(0, 500), None);
        assert_eq!(token_scale(500, 0), None);
        // One odd count stays within bounds.
        assert_eq!(token_scale(100, 100_000), Some(TOKEN_SCALE_MAX_PERCENT));
        assert_eq!(token_scale(100_000, 1), Some(TOKEN_SCALE_MIN_PERCENT));
        assert_eq!(scaled(1000, 200), 2000);
        assert_eq!(scaled(3, 150), 5);
    }

    #[test]
    fn a_corrected_estimate_compacts_when_the_real_context_is_big() {
        // 18 messages estimated at 3,000 tokens each = 54,000: under 75% of 100,000 as is, but
        // the provider counts twice that, so corrected it compacts.
        let msgs = conv("uauauauauauauauaua", 3000);
        assert_eq!(plan(&msgs, 0, 100_000), None);
        let corrected: Vec<Sized> = msgs
            .iter()
            .map(|m| Sized {
                tokens: scaled(m.tokens, 200),
                ..*m
            })
            .collect();
        assert!(plan(&corrected, 0, 100_000).is_some());
    }

    #[test]
    fn a_big_recent_part_doesnt_resummarise_the_summary_every_call() {
        // The 2026-10-29 loop: a summary (2,000) then a user message and big recent tool results
        // (8 x 10,000) over 75% of 100,000. The only cut summarises the summary: not worth a call.
        let mut msgs = vec![Sized {
            is_user: true,
            tokens: 2000,
        }];
        msgs.extend(conv("uattttttt", 10_000));
        assert_eq!(plan(&msgs, 3000, 100_000), None);
        // A real backlog in front still compacts.
        let mut backlog = conv("uauauauaua", 4000);
        backlog.extend(conv("uattttttt", 10_000));
        let cut = plan(&backlog, 3000, 100_000).unwrap();
        assert_eq!(cut, 10);
        // Asked for (/compact), it still goes ahead.
        assert_eq!(plan(&msgs, 3000, 0), Some(1));
    }

    #[test]
    fn estimates_round_up() {
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("abcde"), 2);
    }
}
