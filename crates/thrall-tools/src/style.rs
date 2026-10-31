//! The style check behind the humaniser: finds the habits that make writing read as
//! machine-made (puffed-up phrases, chatbot talk, stock endings, overused words, flat sentence
//! rhythm). The patterns come from Wikipedia's "Signs of AI writing" guide and live in
//! [`STYLE_PATTERNS`]. Free and instant: no model call. It reports; it never rewrites.

use std::collections::HashMap;
use std::sync::LazyLock;

use regex::{Regex, RegexBuilder};

use crate::constants::{
    STYLE_BOLD_PER_100, STYLE_DETECTOR_MIN_SENTENCES, STYLE_DETECTOR_VARIATION,
    STYLE_EM_DASHES_PER_100, STYLE_LONG_SENTENCE_WORDS, STYLE_LOW_VARIATION, STYLE_MARKS_MIN,
    STYLE_MAX_HITS, STYLE_MIN_SENTENCES, STYLE_OPENER_MIN, STYLE_OPENER_SHARE, STYLE_PATTERNS,
    STYLE_SHORT_SENTENCE_WORDS, STYLE_SMALL_WORD_CHARS, STYLE_TITLE_CASE_WORDS, STYLE_TRANSITIONS,
    STYLE_TRANSITIONS_MIN,
};

/// One habit to look for.
#[derive(Debug)]
pub struct Pattern {
    /// Its name in reports ("advert tone").
    pub label: &'static str,
    /// Why readers notice it.
    pub why: &'static str,
    /// What to do instead.
    pub fix: &'static str,
    /// Hits only count when there are at least this many in the text.
    pub min_hits: usize,
    /// What it looks like (regular expressions, case ignored).
    pub regexes: &'static [&'static str],
}

/// One place the text shows a habit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    /// The habit.
    pub label: &'static str,
    /// Why it's a problem.
    pub why: &'static str,
    /// What to do instead.
    pub fix: &'static str,
    /// Line it's on (from 1).
    pub line: usize,
    /// The words found.
    pub found: String,
}

/// What the check found.
#[derive(Debug, Clone, PartialEq)]
pub struct Report {
    /// Words in the text (code left out).
    pub words: usize,
    /// Sentences in its paragraphs (headings, lists and tables left out).
    pub sentences: usize,
    /// How much sentence lengths vary (standard deviation ÷ mean), with enough sentences to say.
    pub variation: Option<f64>,
    /// Each sentence's length in words, in order.
    pub lengths: Vec<usize>,
    /// Phrases to fix, in order.
    pub hits: Vec<Hit>,
    /// Whole-text notes: rhythm, em dashes, bold, emoji, Title Case.
    pub notes: Vec<String>,
}

/// Each pattern's regexes as one, in [`STYLE_PATTERNS`] order (`None` if one didn't build, which
/// the tests rule out).
static COMPILED: LazyLock<Vec<Option<Regex>>> = LazyLock::new(|| {
    STYLE_PATTERNS
        .iter()
        .map(|p| {
            let joined = p
                .regexes
                .iter()
                .map(|r| format!("(?:{r})"))
                .collect::<Vec<_>>()
                .join("|");
            RegexBuilder::new(&joined)
                .case_insensitive(true)
                .build()
                .ok()
        })
        .collect()
});

static BOLD: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"\*\*[^*\n]+\*\*|__[^_\n]+__").ok());
static SENTENCE_END: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r#"[.!?]+["'”’)\]]*(?:\s+|$)"#).ok());
static LIST_MARKER: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"^\s*(?:[-*+]|\d+[.)])\s+").ok());

/// The text with code (fenced blocks and `inline` spans) blanked out, line count kept, so code is
/// never judged as prose.
fn prose(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut fenced = false;
    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            fenced = !fenced;
            out.push('\n');
            continue;
        }
        if !fenced {
            let mut in_code = false;
            for c in line.chars() {
                if c == '`' {
                    in_code = !in_code;
                    out.push(' ');
                } else {
                    out.push(if in_code { ' ' } else { c });
                }
            }
        }
        out.push('\n');
    }
    out
}

fn line_of(text: &str, at: usize) -> usize {
    text[..at].matches('\n').count() + 1
}

/// The phrase hits, earlier patterns first; a hit overlapping one already taken is dropped.
fn phrase_hits(text: &str) -> Vec<Hit> {
    let mut taken: Vec<(usize, usize)> = Vec::new();
    let mut hits: Vec<(usize, Hit)> = Vec::new();
    for (pattern, regex) in STYLE_PATTERNS.iter().zip(COMPILED.iter()) {
        let Some(regex) = regex else { continue };
        let found: Vec<(usize, usize)> = regex
            .find_iter(text)
            .map(|m| (m.start(), m.end()))
            .filter(|&(s, e)| !taken.iter().any(|&(ts, te)| s < te && ts < e))
            .collect();
        if found.len() < pattern.min_hits.max(1) {
            continue;
        }
        for (start, end) in found {
            taken.push((start, end));
            hits.push((
                start,
                Hit {
                    label: pattern.label,
                    why: pattern.why,
                    fix: pattern.fix,
                    line: line_of(text, start),
                    found: text[start..end].trim_start_matches([',', ' ']).to_owned(),
                },
            ));
        }
    }
    hits.sort_by_key(|(at, _)| *at);
    hits.into_iter().map(|(_, h)| h).collect()
}

/// The paragraphs' sentences (headings, list items and table rows left out).
fn sentences(text: &str) -> Vec<String> {
    let (Some(end), Some(marker)) = (SENTENCE_END.as_ref(), LIST_MARKER.as_ref()) else {
        return Vec::new();
    };
    let mut paragraphs: Vec<String> = Vec::new();
    let mut current = String::new();
    for line in text.lines() {
        let t = line.trim();
        let skip = t.is_empty()
            || t.starts_with('#')
            || t.starts_with('|')
            || t.starts_with('>')
            || marker.is_match(line);
        if skip {
            if !current.trim().is_empty() {
                paragraphs.push(std::mem::take(&mut current));
            }
            current.clear();
            continue;
        }
        current.push(' ');
        current.push_str(t);
    }
    if !current.trim().is_empty() {
        paragraphs.push(current);
    }
    let mut out = Vec::new();
    for p in paragraphs {
        let mut from = 0;
        for m in end.find_iter(&p) {
            out.push(p[from..m.end()].trim().to_owned());
            from = m.end();
        }
        if from < p.len() {
            out.push(p[from..].trim().to_owned());
        }
    }
    out.retain(|s| s.split_whitespace().next().is_some());
    out
}

fn first_word(sentence: &str) -> String {
    sentence
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .trim_matches(|c: char| !c.is_alphanumeric())
        .to_lowercase()
}

fn is_emoji(c: char) -> bool {
    matches!(c as u32, 0x1F000..=0x1FAFF | 0x2600..=0x27BF | 0x2B50 | 0x2B55)
}

fn is_title_case(heading: &str) -> bool {
    let long: Vec<&str> = heading
        .trim_start_matches('#')
        .split_whitespace()
        .filter(|w| w.chars().count() > STYLE_SMALL_WORD_CHARS)
        .collect();
    long.len() >= STYLE_TITLE_CASE_WORDS
        && long
            .iter()
            .all(|w| w.chars().next().is_some_and(char::is_uppercase))
}

fn per_100(count: usize, words: usize) -> f64 {
    if words == 0 {
        return 0.0;
    }
    count as f64 * 100.0 / words as f64
}

/// Check `text` (plain text or markdown).
pub fn check(text: &str) -> Report {
    let text = prose(text);
    let words = text.split_whitespace().count();
    let hits = phrase_hits(&text);
    let sentences = sentences(&text);
    let mut notes = Vec::new();

    let lengths: Vec<f64> = sentences
        .iter()
        .map(|s| s.split_whitespace().count() as f64)
        .collect();
    let mut variation = None;
    if lengths.len() >= STYLE_MIN_SENTENCES {
        let n = lengths.len() as f64;
        let mean = lengths.iter().sum::<f64>() / n;
        let spread = (lengths.iter().map(|l| (l - mean).powi(2)).sum::<f64>() / n).sqrt();
        let v = if mean > 0.0 { spread / mean } else { 0.0 };
        variation = Some(v);
        if v < STYLE_LOW_VARIATION {
            notes.push(format!(
                "Sentence lengths barely vary (about {mean:.0} words each, variation {v:.2}): \
                 mix short sentences with long ones."
            ));
        }
        let mut openers: HashMap<String, usize> = HashMap::new();
        for s in &sentences {
            *openers.entry(first_word(s)).or_default() += 1;
        }
        if let Some((word, count)) = openers
            .into_iter()
            .filter(|(w, _)| !w.is_empty())
            .max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(&a.0)))
            && count >= STYLE_OPENER_MIN
            && count as f64 / n >= STYLE_OPENER_SHARE
        {
            notes.push(format!(
                "{count} of {} sentences start with \"{word}\": vary how sentences begin.",
                sentences.len()
            ));
        }
    }
    let transitions = sentences
        .iter()
        .filter(|s| STYLE_TRANSITIONS.contains(&first_word(s).as_str()))
        .count();
    if transitions >= STYLE_TRANSITIONS_MIN {
        notes.push(format!(
            "{transitions} sentences open with a linking word (Additionally, Moreover, \
             Ultimately…): most can go."
        ));
    }
    let dashes = text.matches('—').count();
    if dashes >= STYLE_MARKS_MIN && per_100(dashes, words) > STYLE_EM_DASHES_PER_100 {
        notes.push(format!(
            "{dashes} em dashes in {words} words: use commas, full stops or brackets for most."
        ));
    }
    let bold = BOLD.as_ref().map_or(0, |b| b.find_iter(&text).count());
    if bold >= STYLE_MARKS_MIN && per_100(bold, words) > STYLE_BOLD_PER_100 {
        notes.push(format!(
            "{bold} bold phrases: keep bold for the one thing a reader mustn't miss."
        ));
    }
    let marker = LIST_MARKER.as_ref();
    let emoji = text
        .lines()
        .filter(|l| {
            let rest = marker.map_or(l.trim_start(), |m| match m.find(l) {
                Some(found) => &l[found.end()..],
                None => l.trim_start(),
            });
            let rest = rest.trim_start_matches('#').trim_start();
            rest.chars().next().is_some_and(is_emoji)
        })
        .count();
    if emoji >= STYLE_MARKS_MIN {
        notes.push(format!(
            "{emoji} lines start with an emoji: drop them as decoration."
        ));
    }
    let title_case = text
        .lines()
        .filter(|l| l.trim_start().starts_with('#') && is_title_case(l.trim()))
        .count();
    if title_case >= STYLE_MARKS_MIN {
        notes.push(format!(
            "{title_case} headings in Title Case: use sentence case (\"How it works\")."
        ));
    }

    Report {
        words,
        sentences: sentences.len(),
        variation,
        lengths: lengths.iter().map(|l| *l as usize).collect(),
        hits,
        notes,
    }
}

impl Report {
    /// Nothing to fix and nothing to note.
    pub fn is_clean(&self) -> bool {
        self.hits.is_empty() && self.notes.is_empty()
    }

    /// One line for the person: "3 to fix, 1 note".
    pub fn summary(&self) -> String {
        if self.is_clean() {
            return "Nothing to fix".to_owned();
        }
        let notes = match self.notes.len() {
            0 => String::new(),
            1 => ", 1 note".to_owned(),
            n => format!(", {n} notes"),
        };
        format!("{} to fix{notes}", self.hits.len())
    }

    /// What AI detectors measure that a rewrite can change, for detector-aware mode. Detectors
    /// score how predictable each word is (which only a model can measure) and how much sentence
    /// length varies ("burstiness"), which is counted here. No score and no promise of passing.
    pub fn detector_signals(&self) -> String {
        let n = self.lengths.len();
        if n < STYLE_DETECTOR_MIN_SENTENCES {
            return format!(
                "Detector signals: only {n} sentences, too few to judge rhythm. Word choice still \
                 matters: prefer the specific word over the most expected one."
            );
        }
        let mean = self.lengths.iter().sum::<usize>() as f64 / n as f64;
        let spread = (self
            .lengths
            .iter()
            .map(|l| (*l as f64 - mean).powi(2))
            .sum::<f64>()
            / n as f64)
            .sqrt();
        let v = if mean > 0.0 { spread / mean } else { 0.0 };
        let rough = if n < STYLE_MIN_SENTENCES {
            " (few sentences: a rough guide)"
        } else {
            ""
        };
        let short = self
            .lengths
            .iter()
            .filter(|l| **l <= STYLE_SHORT_SENTENCE_WORDS)
            .count();
        let long = self
            .lengths
            .iter()
            .filter(|l| **l >= STYLE_LONG_SENTENCE_WORDS)
            .count();
        let (min, max) = (
            self.lengths.iter().min().copied().unwrap_or_default(),
            self.lengths.iter().max().copied().unwrap_or_default(),
        );
        let verdict = if v >= STYLE_DETECTOR_VARIATION {
            "varied enough".to_owned()
        } else {
            format!(
                "too even: aim for {STYLE_DETECTOR_VARIATION:.2} or more with some sentences of \
                 {STYLE_SHORT_SENTENCE_WORDS} words or fewer and some of {STYLE_LONG_SENTENCE_WORDS} \
                 or more"
            )
        };
        format!(
            "Detector signals: sentence-length variation {v:.2}{rough} ({verdict}); {n} sentences, \
             {min} to {max} words; {short} short, {long} long. Predictability can't be measured \
             here: prefer the specific word over the most expected one."
        )
    }

    /// The full report, for the model.
    pub fn render(&self) -> String {
        let size = match self.variation {
            Some(v) => format!(
                "{} words, {} sentences, sentence-length variation {v:.2}",
                self.words, self.sentences
            ),
            None => format!("{} words, {} sentences", self.words, self.sentences),
        };
        if self.is_clean() {
            return format!("Style check: nothing to fix ({size}).");
        }
        let mut out = format!("Style check: {} ({size}).", self.summary());
        if !self.hits.is_empty() {
            out.push_str("\nPhrases:");
        }
        for hit in self.hits.iter().take(STYLE_MAX_HITS) {
            out.push_str(&format!(
                "\n- line {}, {}: \"{}\" ({}; fix: {})",
                hit.line, hit.label, hit.found, hit.why, hit.fix
            ));
        }
        if self.hits.len() > STYLE_MAX_HITS {
            out.push_str(&format!("\n… {} more", self.hits.len() - STYLE_MAX_HITS));
        }
        if !self.notes.is_empty() {
            out.push_str("\nWhole text:");
            for note in &self.notes {
                out.push_str(&format!("\n- {note}"));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MACHINE: &str = "Nestled in the heart of the valley, the town stands as a testament to \
        resilience. Its vibrant market plays a pivotal role in local life, highlighting its \
        enduring appeal. Experts say the region is not just a destination, it's a journey. \
        Additionally, visitors can delve into its rich history. Despite these challenges, the \
        future looks bright. I hope this helps!";

    const PLAIN: &str = "The town sits at the bottom of the valley, two hours from the coast. \
        Its Saturday market started in 1952 with six stalls. Now it has eighty. Most sell \
        fruit from the orchards on the eastern slope, and the apple growers set the prices \
        for everyone else. The council closed the old bridge in March. Traffic now goes the \
        long way round, which the café owners hate.";

    fn labels(r: &Report) -> Vec<&'static str> {
        r.hits.iter().map(|h| h.label).collect()
    }

    #[test]
    fn every_pattern_builds() {
        assert!(COMPILED.iter().all(Option::is_some));
        assert!(BOLD.is_some() && SENTENCE_END.is_some() && LIST_MARKER.is_some());
    }

    #[test]
    fn machine_writing_is_caught() {
        let r = check(MACHINE);
        let l = labels(&r);
        for want in [
            "advert tone",
            "inflated significance",
            "-ing tail",
            "vague attribution",
            "not-X-but-Y",
            "overused word",
            "stock ending",
            "chatbot talk",
        ] {
            assert!(l.contains(&want), "missing {want}: {l:?}");
        }
        // "stands as a testament" is one hit (inflated significance), not also a stand-in verb.
        assert!(!l.contains(&"stand-in verb"), "{l:?}");
        assert!(r.render().contains("line 1, advert tone: \"Nestled\""));
    }

    #[test]
    fn plain_writing_is_clean() {
        let r = check(PLAIN);
        assert!(r.is_clean(), "{}", r.render());
        assert!(r.variation.is_some_and(|v| v > STYLE_LOW_VARIATION));
        assert!(r.render().starts_with("Style check: nothing to fix"));
    }

    #[test]
    fn one_overused_word_is_normal_writing() {
        assert!(check("The bridge is robust and cheap.").is_clean());
        assert_eq!(
            labels(&check("A robust plan and a crucial step.")),
            vec!["overused word", "overused word"]
        );
    }

    #[test]
    fn code_is_not_judged() {
        let text = "Run this:\n\n```\n// a robust, crucial, vibrant helper\n```\n\nUse `delve()` \
                    and `leverage()` here.";
        assert!(check(text).is_clean(), "{}", check(text).render());
    }

    #[test]
    fn chatbot_leftovers_are_found() {
        let r = check("See the report.\n\nSource: https://x.org/a?utm_source=chatgpt.com");
        assert_eq!(labels(&r), vec!["chatbot leftover"]);
        assert_eq!(r.hits[0].line, 3);
    }

    #[test]
    fn flat_rhythm_and_padding_are_noted() {
        let text = "The team built the new system last year. Additionally the team tested \
                    it on real data. Moreover the team wrote the guide for new staff. Ultimately \
                    the team shipped the tool on time. The team then fixed the open bugs quickly. \
                    The team will review the results next week.";
        let r = check(text);
        let notes = r.notes.join(" | ");
        assert!(notes.contains("barely vary"), "{notes}");
        assert!(notes.contains("start with \"the\""), "{notes}");
        assert!(notes.contains("linking word"), "{notes}");
    }

    #[test]
    fn formatting_habits_are_noted() {
        let text = "## Building The Faster Engine\n\n## Measuring Every Speed Change\n\n## \
                    Planning Next Year's Work\n\n- 🚀 **Fast** — very\n- ✅ **Safe** — very\n- 🔒 **Small** — \
                    very\n";
        let notes = check(text).notes.join(" | ");
        for want in ["Title Case", "emoji", "bold", "em dashes"] {
            assert!(notes.contains(want), "missing {want}: {notes}");
        }
    }

    #[test]
    fn detector_signals_judge_rhythm_only_with_enough_sentences() {
        let flat = "The team built the new system last year. The team tested it on real data \
                    too. The team wrote the guide for new staff. The team shipped the tool on \
                    time again. The team then fixed the open bugs quickly. The team will review \
                    the results next week.";
        let signals = check(flat).detector_signals();
        assert!(signals.contains("too even"), "{signals}");
        let varied = "Rain again. The council met on Tuesday night in the old library to argue, \
                      once more, about whether the bridge over the river should be rebuilt in \
                      stone or in steel. Nobody agreed. Steel is cheaper. The engineers said a \
                      stone bridge would last two centuries if the footings went deep enough, \
                      which nobody wanted to pay for. They voted at eleven.";
        let signals = check(varied).detector_signals();
        assert!(signals.contains("varied enough"), "{signals}");
        assert!(
            check("One line. Two.")
                .detector_signals()
                .contains("too few")
        );
        let few = check("Short one. This one runs on for quite a lot longer. Done.");
        assert!(
            few.detector_signals().contains("rough guide"),
            "{}",
            few.detector_signals()
        );
        assert_eq!(check("One two. Three four five.").lengths, vec![2, 3]);
    }

    #[test]
    fn summaries_count_what_was_found() {
        assert_eq!(check(PLAIN).summary(), "Nothing to fix");
        assert!(check(MACHINE).summary().ends_with("to fix"));
    }
}
