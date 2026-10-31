//! Named constants for `thrall-memory`. Every limit, timeout and repeated string lives here.

/// Rough characters per token, for estimating context size without a tokenizer.
pub const CHARS_PER_TOKEN: usize = 4;

/// The character estimate runs low for some models (code, JSON and transcripts count nearer 2
/// characters per token), so it's corrected by what the provider really counted: real tokens per
/// 100 estimated. Before the first count the estimate is used as it is.
pub const TOKEN_SCALE_DEFAULT_PERCENT: usize = 100;

/// Lowest correction: one odd count can't make compaction wait more than twice as long.
pub const TOKEN_SCALE_MIN_PERCENT: usize = 50;

/// Highest correction: one odd count can't make compaction run more than four times as early.
pub const TOKEN_SCALE_MAX_PERCENT: usize = 400;

/// Trimming (when on): a tool result is "old" once this many user messages have come after it.
pub const TRIM_AFTER_USER_MESSAGES: usize = 3;

/// Trimming: only tool results at least this long (characters) are trimmed.
pub const TRIM_MIN_CHARS: usize = 8000;

/// Trimming: characters of a trimmed result still sent (its start).
pub const TRIM_KEEP_CHARS: usize = 1000;

/// Compaction starts when the context reaches this share of the limit (percent).
pub const COMPACT_AT_PERCENT: usize = 75;

/// Recent messages always kept as they are when compacting.
pub const KEEP_RECENT_MESSAGES: usize = 8;

/// Compaction only runs when the part it would summarise is at least this share of the context
/// (percent). When the recent messages alone are over the limit, the only thing left to cut is
/// the last summary: summarising it again every call gains nothing and costs a call each time
/// (seen 2026-10-29: 7 compactions in 7 minutes, mostly of 1 message).
pub const COMPACT_MIN_GAIN_PERCENT: usize = 25;

/// Reciprocal-rank-fusion constant: how much lower-ranked hits still count.
pub const RRF_K: f32 = 60.0;

/// Below this similarity a vector match isn't worth showing.
pub const MIN_SIMILARITY: f32 = 0.25;

/// Candidates taken from each search (words, meaning) before merging.
pub const CANDIDATES_PER_SOURCE: u32 = 30;

/// Most characters of a conversation kept in an episode.
pub const EPISODE_MAX_CHARS: usize = 4000;

/// Most characters of one remembered thing sent for embedding.
pub const EMBED_MAX_CHARS: usize = 8000;

/// Words ignored when deciding whether two fact topics are the same ("favourite drink updated"
/// is still "favourite drink").
pub const TOPIC_FILLER_WORDS: &[&str] = &[
    "a",
    "an",
    "the",
    "my",
    "of",
    "user",
    "users",
    "updated",
    "update",
    "new",
    "current",
    "latest",
    "now",
    "changed",
    "change",
    "revised",
    "corrected",
    "correction",
];
