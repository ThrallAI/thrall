//! The conversation on screen: entries built from protocol events, each rendered once and cached
//! (re-rendered only when it changes or the width changes), and only visible entries drawn.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};
use thrall_protocol::{Diff, StepStatus, Usage};

use crate::constants::{STEP_EXPANDED_LINES, STEP_TAIL_LINES};
use crate::{markdown, plain, theme};

/// One thing in the transcript.
#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    /// What the user typed.
    User(String),
    /// Thrall's reply (markdown).
    Assistant {
        /// Message id.
        id: String,
        /// Reply so far.
        text: String,
        /// Whether thinking text arrived.
        thinking: bool,
        /// Still arriving.
        streaming: bool,
    },
    /// A tool call.
    Step {
        /// Step id.
        id: String,
        /// "Reading src/main.rs".
        title: String,
        /// `None` while running.
        status: Option<StepStatus>,
        /// One-line result.
        summary: String,
        /// Live output (shell).
        output: String,
        /// The change, for edits.
        diff: Option<Diff>,
    },
    /// Information or a warning.
    Notice {
        /// Warning rather than information.
        warning: bool,
        /// Text.
        text: String,
    },
    /// Something failed.
    Error(String),
    /// A slash command's answer (markdown).
    Command(String),
    /// Totals after a turn.
    TurnEnd(Usage),
}

struct Cached {
    version: u64,
    width: u16,
    expanded: bool,
    lines: Vec<Line<'static>>,
    height: usize,
}

struct Entry {
    kind: Kind,
    version: u64,
    blank_before: bool,
    cache: Option<Cached>,
}

/// The whole transcript.
#[derive(Default)]
pub struct Transcript {
    entries: Vec<Entry>,
    /// Show step output and diffs in full.
    pub expanded: bool,
    /// Draw with ASCII only (consoles that can't show symbols).
    pub ascii: bool,
}

impl Transcript {
    /// An empty transcript; `ascii` for consoles that can't show symbols.
    pub fn new(ascii: bool) -> Self {
        Self {
            ascii,
            ..Self::default()
        }
    }

    /// Remove everything.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Add an entry at the end.
    pub fn push(&mut self, kind: Kind) {
        let is_step = matches!(kind, Kind::Step { .. });
        let after_step = matches!(
            self.entries.last().map(|e| &e.kind),
            Some(Kind::Step { .. })
        );
        let blank_before = !self.entries.is_empty()
            && !(is_step && after_step)
            && !matches!(kind, Kind::TurnEnd(_));
        self.entries.push(Entry {
            kind,
            version: 0,
            blank_before,
            cache: None,
        });
    }

    /// Change the entry matching `find`, newest first. False if there's none.
    pub fn update(&mut self, find: impl Fn(&Kind) -> bool, change: impl FnOnce(&mut Kind)) -> bool {
        match self.entries.iter_mut().rev().find(|e| find(&e.kind)) {
            Some(entry) => {
                change(&mut entry.kind);
                entry.version += 1;
                true
            }
            None => false,
        }
    }

    /// How many entries there are.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Drop every entry from `len` on (a turn that was taken back).
    pub fn truncate(&mut self, len: usize) {
        self.entries.truncate(len);
    }

    /// Whether there's nothing yet.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Render whatever changed for this width; returns the total height in rows.
    pub fn prepare(&mut self, width: u16) -> usize {
        let expanded = self.expanded;
        let ascii = self.ascii;
        let mut total = 0;
        for entry in &mut self.entries {
            let fresh = entry.cache.as_ref().is_some_and(|c| {
                c.version == entry.version && c.width == width && c.expanded == expanded
            });
            if !fresh {
                let body = render(&entry.kind, expanded);
                let mut lines = Vec::new();
                // Entries with nothing to show take no room at all.
                if entry.blank_before && !body.is_empty() {
                    lines.push(Line::default());
                }
                if ascii {
                    lines.extend(body.into_iter().map(plain::line));
                } else {
                    lines.extend(body);
                }
                let height = if lines.is_empty() {
                    0
                } else {
                    Paragraph::new(lines.clone())
                        .wrap(Wrap { trim: false })
                        .line_count(width)
                };
                entry.cache = Some(Cached {
                    version: entry.version,
                    width,
                    expanded,
                    lines,
                    height,
                });
            }
            total += entry.cache.as_ref().map_or(0, |c| c.height);
        }
        total
    }

    /// The lines covering rows `top..top + height` (after `prepare`), and how many wrapped rows of
    /// the first returned line to skip.
    pub fn visible(&self, top: usize, height: usize) -> (Vec<Line<'static>>, u16) {
        let mut y = 0;
        let mut out = Vec::new();
        let mut skip = 0;
        for entry in &self.entries {
            let Some(cache) = &entry.cache else { continue };
            let end = y + cache.height;
            if end > top && y < top + height {
                if out.is_empty() {
                    skip = top.saturating_sub(y);
                }
                out.extend(cache.lines.iter().cloned());
            }
            if y >= top + height {
                break;
            }
            y = end;
        }
        (out, u16::try_from(skip).unwrap_or(u16::MAX))
    }
}

fn render(kind: &Kind, expanded: bool) -> Vec<Line<'static>> {
    match kind {
        Kind::User(text) => {
            let mut lines = Vec::new();
            for (i, line) in text.lines().enumerate() {
                let marker = if i == 0 { "› " } else { "  " };
                lines.push(Line::from(vec![
                    Span::styled(marker, theme::strong_accent()),
                    Span::styled(
                        line.to_owned(),
                        Style::default().add_modifier(Modifier::BOLD),
                    ),
                ]));
            }
            lines
        }
        Kind::Assistant {
            text,
            thinking,
            streaming,
            ..
        } => {
            if text.is_empty() {
                // A finished message with no text was only thinking before tool calls: the steps
                // that follow say what happened, so it shows nothing.
                if !*streaming {
                    return Vec::new();
                }
                let label = if *thinking { "Thinking…" } else { "…" };
                return vec![Line::from(vec![
                    Span::styled("● ", theme::accent()),
                    Span::styled(label, theme::dim().add_modifier(Modifier::ITALIC)),
                ])];
            }
            markdown::render(text)
                .into_iter()
                .enumerate()
                .map(|(i, line)| {
                    let marker = if i == 0 {
                        Span::styled("● ", theme::accent())
                    } else {
                        Span::raw("  ")
                    };
                    let mut spans = vec![marker];
                    spans.extend(line.spans);
                    Line::from(spans)
                })
                .collect()
        }
        Kind::Step {
            title,
            status,
            summary,
            output,
            diff,
            ..
        } => render_step(title, *status, summary, output, diff.as_ref(), expanded),
        Kind::Notice { warning, text } => {
            let (mark, style) = if *warning {
                ("! ", Style::default().fg(theme::WARN))
            } else {
                ("· ", theme::dim())
            };
            vec![Line::from(Span::styled(format!("{mark}{text}"), style))]
        }
        Kind::Error(text) => vec![Line::from(Span::styled(
            format!("✗ {text}"),
            Style::default().fg(theme::ERROR),
        ))],
        Kind::Command(md) => markdown::render(md)
            .into_iter()
            .map(|line| {
                let mut spans = vec![Span::styled("│ ", theme::dim())];
                spans.extend(line.spans);
                Line::from(spans)
            })
            .collect(),
        Kind::TurnEnd(usage) => {
            let mut text = format!("  {} in · {} out", usage.input_tokens, usage.output_tokens);
            if let Some(cost) = usage.cost_usd {
                text.push_str(&format!(" · ${cost:.4}"));
            }
            vec![Line::from(Span::styled(text, theme::dim()))]
        }
    }
}

fn render_step(
    title: &str,
    status: Option<StepStatus>,
    summary: &str,
    output: &str,
    diff: Option<&Diff>,
    expanded: bool,
) -> Vec<Line<'static>> {
    let (icon, colour) = match status {
        None => ("○", theme::ACCENT),
        Some(StepStatus::Ok) => ("✓", theme::OK),
        Some(StepStatus::Error) => ("✗", theme::ERROR),
        Some(StepStatus::Warn) => ("!", theme::WARN),
        Some(StepStatus::Denied | StepStatus::Cancelled) => ("⊘", theme::WARN),
    };
    let mut head = vec![
        Span::styled(format!("  {icon} "), Style::default().fg(colour)),
        Span::raw(title.to_owned()),
    ];
    if !summary.is_empty() {
        head.push(Span::styled(format!(" · {summary}"), theme::dim()));
    }
    let mut lines = vec![Line::from(head)];

    let show = if expanded {
        STEP_EXPANDED_LINES
    } else if status.is_none() {
        STEP_TAIL_LINES
    } else {
        0
    };
    if show > 0 {
        if let Some(diff) = diff.filter(|_| expanded) {
            for line in diff.unified.lines().take(show) {
                let style = if line.starts_with('+') && !line.starts_with("+++") {
                    Style::default().fg(theme::OK)
                } else if line.starts_with('-') && !line.starts_with("---") {
                    Style::default().fg(theme::ERROR)
                } else if line.starts_with("@@") {
                    theme::accent()
                } else {
                    theme::dim()
                };
                lines.push(Line::from(vec![
                    Span::styled("    │ ", theme::dim()),
                    Span::styled(line.to_owned(), style),
                ]));
            }
        }
        let out: Vec<&str> = output.lines().collect();
        let start = out.len().saturating_sub(show);
        for line in &out[start..] {
            lines.push(Line::from(vec![
                Span::styled("    │ ", theme::dim()),
                Span::styled((*line).to_owned(), theme::dim()),
            ]));
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(lines: &[Line<'_>]) -> String {
        lines
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn steps_collapse_and_expand() {
        let step = Kind::Step {
            id: "p".into(),
            title: "Running cargo test".into(),
            status: Some(StepStatus::Ok),
            summary: "Exit code 0".into(),
            output: "a\nb\nc\nd\n".into(),
            diff: None,
        };
        assert_eq!(render(&step, false).len(), 1);
        assert_eq!(render(&step, true).len(), 5);
    }

    #[test]
    fn running_steps_show_their_tail() {
        let step = Kind::Step {
            id: "p".into(),
            title: "Running".into(),
            status: None,
            summary: String::new(),
            output: "1\n2\n3\n4\n5\n".into(),
            diff: None,
        };
        let shown = text(&render(&step, false));
        assert!(shown.contains('5') && !shown.contains('1'));
    }

    #[test]
    fn only_visible_entries_are_returned() {
        let mut t = Transcript::default();
        for i in 0..50 {
            t.push(Kind::User(format!("message {i}")));
        }
        let total = t.prepare(80);
        assert_eq!(total, 50 + 49, "one line each plus blank lines between");
        let (lines, _) = t.visible(total - 4, 4);
        assert!(text(&lines).contains("message 49"));
        assert!(!text(&lines).contains("message 10"));
    }
}
