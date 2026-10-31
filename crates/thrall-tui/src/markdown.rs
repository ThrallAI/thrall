//! Markdown to terminal lines: headings, emphasis, inline code, code blocks, lists, quotes, rules
//! and simple tables. Wrapping is left to the paragraph that shows the lines.

use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::theme;

struct Renderer {
    lines: Vec<Line<'static>>,
    current: Vec<Span<'static>>,
    styles: Vec<Style>,
    /// Open lists: `None` = bullets, `Some(n)` = next number.
    lists: Vec<Option<u64>>,
    quote_depth: usize,
    in_code_block: bool,
    pending_item_marker: Option<String>,
    link_url: Option<String>,
    table_row: Vec<String>,
    in_table: bool,
}

impl Renderer {
    fn style(&self) -> Style {
        self.styles.last().copied().unwrap_or_default()
    }

    fn push_style(&mut self, f: impl Fn(Style) -> Style) {
        let next = f(self.style());
        self.styles.push(next);
    }

    fn indent(&self) -> String {
        let mut s = "│ ".repeat(self.quote_depth);
        s.push_str(&"  ".repeat(self.lists.len().saturating_sub(1)));
        s
    }

    fn text(&mut self, text: &str) {
        if self.current.is_empty() {
            let mut prefix = self.indent();
            if let Some(marker) = self.pending_item_marker.take() {
                prefix.push_str(&marker);
            } else if !self.lists.is_empty() {
                prefix.push_str("  ");
            }
            if !prefix.is_empty() {
                self.current.push(Span::styled(prefix, theme::dim()));
            }
        }
        self.current
            .push(Span::styled(text.to_owned(), self.style()));
    }

    fn end_line(&mut self) {
        if !self.current.is_empty() || self.pending_item_marker.is_some() {
            if self.current.is_empty() {
                self.text("");
            }
            self.lines
                .push(Line::from(std::mem::take(&mut self.current)));
        }
    }

    fn blank(&mut self) {
        self.end_line();
        if self.lines.last().is_some_and(|l| !l.spans.is_empty()) {
            self.lines.push(Line::default());
        }
    }
}

/// Render `source` as styled lines.
pub fn render(source: &str) -> Vec<Line<'static>> {
    let mut r = Renderer {
        lines: Vec::new(),
        current: Vec::new(),
        styles: vec![theme::text()],
        lists: Vec::new(),
        quote_depth: 0,
        in_code_block: false,
        pending_item_marker: None,
        link_url: None,
        table_row: Vec::new(),
        in_table: false,
    };
    let options =
        Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    for event in Parser::new_ext(source, options) {
        match event {
            Event::Start(tag) => match tag {
                Tag::Heading { level, .. } => {
                    r.blank();
                    r.push_style(|_| {
                        let s = theme::strong_accent();
                        if level == HeadingLevel::H1 {
                            s.add_modifier(Modifier::UNDERLINED)
                        } else {
                            s
                        }
                    });
                }
                Tag::Paragraph => {}
                Tag::Strong => r.push_style(|s| s.add_modifier(Modifier::BOLD)),
                Tag::Emphasis => r.push_style(|s| s.add_modifier(Modifier::ITALIC)),
                Tag::Strikethrough => r.push_style(|s| s.add_modifier(Modifier::CROSSED_OUT)),
                Tag::Link { dest_url, .. } => {
                    r.link_url = Some(dest_url.to_string());
                    r.push_style(|s| s.fg(theme::ACCENT).add_modifier(Modifier::UNDERLINED));
                }
                Tag::BlockQuote(_) => {
                    r.end_line();
                    r.quote_depth += 1;
                    r.push_style(|s| s.fg(theme::DIM).add_modifier(Modifier::ITALIC));
                }
                Tag::CodeBlock(kind) => {
                    r.end_line();
                    r.in_code_block = true;
                    if let CodeBlockKind::Fenced(lang) = kind
                        && !lang.is_empty()
                    {
                        r.lines.push(Line::from(Span::styled(
                            format!("{}  {lang}", r.indent()),
                            theme::dim(),
                        )));
                    }
                }
                Tag::List(start) => {
                    r.end_line();
                    r.lists.push(start);
                }
                Tag::Item => {
                    r.end_line();
                    let marker = match r.lists.last_mut() {
                        Some(Some(n)) => {
                            let m = format!("{n}. ");
                            *n += 1;
                            m
                        }
                        _ => "• ".to_owned(),
                    };
                    r.pending_item_marker = Some(marker);
                }
                Tag::Table(_) => {
                    r.end_line();
                    r.in_table = true;
                }
                Tag::TableHead | Tag::TableRow => r.table_row.clear(),
                Tag::TableCell => r.table_row.push(String::new()),
                _ => {}
            },
            Event::End(tag) => match tag {
                TagEnd::Heading(_) => {
                    r.styles.pop();
                    r.end_line();
                }
                TagEnd::Paragraph => {
                    if r.lists.is_empty() {
                        r.blank();
                    } else {
                        r.end_line();
                    }
                }
                TagEnd::Strong | TagEnd::Emphasis | TagEnd::Strikethrough => {
                    r.styles.pop();
                }
                TagEnd::Link => {
                    r.styles.pop();
                    if let Some(url) = r.link_url.take() {
                        let shown = r.current.iter().any(|s| s.content.contains(url.as_str()));
                        if !shown {
                            r.current
                                .push(Span::styled(format!(" ({url})"), theme::dim()));
                        }
                    }
                }
                TagEnd::BlockQuote(_) => {
                    r.styles.pop();
                    r.end_line();
                    r.quote_depth = r.quote_depth.saturating_sub(1);
                }
                TagEnd::CodeBlock => {
                    r.in_code_block = false;
                    r.blank();
                }
                TagEnd::List(_) => {
                    r.end_line();
                    r.lists.pop();
                    if r.lists.is_empty() {
                        r.blank();
                    }
                }
                TagEnd::Item => r.end_line(),
                TagEnd::TableHead | TagEnd::TableRow => {
                    let row = std::mem::take(&mut r.table_row).join(" │ ");
                    let style = if matches!(tag, TagEnd::TableHead) {
                        theme::text().add_modifier(Modifier::BOLD)
                    } else {
                        theme::text()
                    };
                    r.lines.push(Line::from(Span::styled(
                        format!("{}{row}", r.indent()),
                        style,
                    )));
                }
                TagEnd::Table => {
                    r.in_table = false;
                    r.blank();
                }
                _ => {}
            },
            Event::Text(text) => {
                if r.in_table {
                    if let Some(cell) = r.table_row.last_mut() {
                        cell.push_str(&text);
                    }
                } else if r.in_code_block {
                    for line in text.lines() {
                        r.lines.push(Line::from(vec![
                            Span::styled(format!("{}  ", r.indent()), theme::dim()),
                            Span::styled(line.to_owned(), Style::default().fg(theme::CODE)),
                        ]));
                    }
                } else {
                    r.text(&text);
                }
            }
            Event::Code(code) => {
                if r.in_table {
                    if let Some(cell) = r.table_row.last_mut() {
                        cell.push_str(&code);
                    }
                } else {
                    r.push_style(|s| s.fg(theme::CODE));
                    r.text(&code);
                    r.styles.pop();
                }
            }
            Event::SoftBreak => r.text(" "),
            Event::HardBreak => r.end_line(),
            Event::Rule => {
                r.end_line();
                r.lines
                    .push(Line::from(Span::styled("───".to_owned(), theme::dim())));
                r.blank();
            }
            Event::TaskListMarker(done) => {
                r.pending_item_marker = None;
                r.text(if done { "[x] " } else { "[ ] " });
            }
            _ => {}
        }
    }
    r.end_line();
    while r.lines.last().is_some_and(|l| l.spans.is_empty()) {
        r.lines.pop();
    }
    r.lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(lines: &[Line<'_>]) -> Vec<String> {
        lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect()
    }

    #[test]
    fn paragraphs_lists_and_code() {
        let out = plain(&render(
            "Hello **world** and `code`.\n\n- one\n- two\n\n1. first\n2. second\n\n```rust\nfn x() {}\n```",
        ));
        assert_eq!(out[0], "Hello world and code.");
        assert!(out.contains(&"• one".to_owned()));
        assert!(out.contains(&"2. second".to_owned()));
        assert!(out.iter().any(|l| l.contains("fn x() {}")));
    }

    #[test]
    fn headings_and_links() {
        let out = plain(&render("# Title\n\nSee [docs](https://x.dev)."));
        assert_eq!(out[0], "Title");
        assert!(out.iter().any(|l| l.contains("docs (https://x.dev)")));
    }

    #[test]
    fn no_trailing_blank_lines() {
        let lines = render("text\n\n");
        assert!(!lines.last().unwrap().spans.is_empty());
    }
}
