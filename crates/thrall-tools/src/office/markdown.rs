//! Markdown read into a small document model (headings, paragraphs, lists, tables, code,
//! quotes), shared by the PDF and Word writers so both lay documents out the same way.

use pulldown_cmark::{Alignment, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

/// A run of text with its inline style.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Span {
    /// The text.
    pub text: String,
    /// Bold.
    pub bold: bool,
    /// Italic.
    pub italic: bool,
    /// Inline code.
    pub code: bool,
    /// Struck through.
    pub strike: bool,
    /// A link's address, when it's a link.
    pub link: Option<String>,
}

/// How a table column lines up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    /// Left (the default).
    Left,
    /// Centred.
    Center,
    /// Right.
    Right,
}

/// A block of the document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    /// A heading, level 1-6.
    Heading(u8, Vec<Span>),
    /// A paragraph.
    Paragraph(Vec<Span>),
    /// A list; each item is its own blocks (paragraphs, nested lists).
    List {
        /// Numbered (and from which number), or bulleted.
        start: Option<u64>,
        /// The items.
        items: Vec<Vec<Block>>,
    },
    /// A table.
    Table {
        /// Column alignment.
        align: Vec<Align>,
        /// Header cells.
        head: Vec<Vec<Span>>,
        /// Body rows.
        rows: Vec<Vec<Vec<Span>>>,
    },
    /// A code block.
    Code(String),
    /// A quotation.
    Quote(Vec<Block>),
    /// A horizontal rule.
    Rule,
}

fn options() -> Options {
    Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_SMART_PUNCTUATION
}

/// The document's HTML (body only), as the PDF writer styles it.
pub fn to_html(markdown: &str) -> String {
    let mut html = String::new();
    pulldown_cmark::html::push_html(&mut html, Parser::new_ext(markdown, options()));
    html
}

/// The text of the first level-1 heading, if any (a document's title).
pub fn title(markdown: &str) -> Option<String> {
    parse(markdown).into_iter().find_map(|b| match b {
        Block::Heading(1, spans) => Some(spans.iter().map(|s| s.text.as_str()).collect()),
        _ => None,
    })
}

/// A table being read: column alignment, header cells, body rows.
type TableParts = (Vec<Align>, Vec<Vec<Span>>, Vec<Vec<Vec<Span>>>);

/// Where parsing is: the blocks being built, innermost last.
enum Frame {
    Root(Vec<Block>),
    Quote(Vec<Block>),
    List(Option<u64>, Vec<Vec<Block>>),
    Item(Vec<Block>),
}

impl Frame {
    fn blocks(&mut self) -> &mut Vec<Block> {
        match self {
            Self::Root(b) | Self::Quote(b) | Self::Item(b) => b,
            // Text straight inside a list belongs to its last item.
            Self::List(_, items) => {
                if items.is_empty() {
                    items.push(Vec::new());
                }
                let last = items.len() - 1;
                &mut items[last]
            }
        }
    }
}

/// Read markdown into blocks.
#[allow(clippy::too_many_lines)]
pub fn parse(markdown: &str) -> Vec<Block> {
    let mut stack = vec![Frame::Root(Vec::new())];
    // Inline content being gathered (a paragraph, heading, or table cell).
    let mut spans: Vec<Span> = Vec::new();
    let (mut bold, mut italic, mut strike) = (0u32, 0u32, 0u32);
    let mut link: Option<String> = None;
    let mut in_code_block: Option<String> = None;
    // Tables.
    let mut table: Option<TableParts> = None;
    let mut in_head = false;
    let mut row: Vec<Vec<Span>> = Vec::new();
    // A list item's loose text (tight lists have no Paragraph events).
    let flush_loose = |stack: &mut Vec<Frame>, spans: &mut Vec<Span>| {
        if !spans.is_empty()
            && let Some(Frame::Item(b)) = stack.last_mut()
        {
            b.push(Block::Paragraph(std::mem::take(spans)));
        }
    };

    for event in Parser::new_ext(markdown, options()) {
        match event {
            Event::Start(tag) => match tag {
                Tag::Heading { .. } | Tag::Paragraph => spans.clear(),
                Tag::BlockQuote(_) => stack.push(Frame::Quote(Vec::new())),
                Tag::List(start) => {
                    flush_loose(&mut stack, &mut spans);
                    stack.push(Frame::List(start, Vec::new()));
                }
                Tag::Item => stack.push(Frame::Item(Vec::new())),
                Tag::CodeBlock(_) => in_code_block = Some(String::new()),
                Tag::Table(aligns) => {
                    let align = aligns
                        .iter()
                        .map(|a| match a {
                            Alignment::Center => Align::Center,
                            Alignment::Right => Align::Right,
                            _ => Align::Left,
                        })
                        .collect();
                    table = Some((align, Vec::new(), Vec::new()));
                }
                Tag::TableHead => {
                    in_head = true;
                    row.clear();
                }
                Tag::TableRow => row.clear(),
                Tag::TableCell => spans.clear(),
                Tag::Strong => bold += 1,
                Tag::Emphasis => italic += 1,
                Tag::Strikethrough => strike += 1,
                Tag::Link { dest_url, .. } => link = Some(dest_url.into_string()),
                _ => {}
            },
            Event::End(end) => match end {
                TagEnd::Heading(level) => {
                    let n = match level {
                        HeadingLevel::H1 => 1,
                        HeadingLevel::H2 => 2,
                        HeadingLevel::H3 => 3,
                        HeadingLevel::H4 => 4,
                        HeadingLevel::H5 => 5,
                        HeadingLevel::H6 => 6,
                    };
                    let done = std::mem::take(&mut spans);
                    if let Some(top) = stack.last_mut() {
                        top.blocks().push(Block::Heading(n, done));
                    }
                }
                TagEnd::Paragraph => {
                    let done = std::mem::take(&mut spans);
                    if let Some(top) = stack.last_mut() {
                        top.blocks().push(Block::Paragraph(done));
                    }
                }
                TagEnd::BlockQuote(_) => {
                    if let Some(Frame::Quote(blocks)) = stack.pop()
                        && let Some(top) = stack.last_mut()
                    {
                        top.blocks().push(Block::Quote(blocks));
                    }
                }
                TagEnd::Item => {
                    flush_loose(&mut stack, &mut spans);
                    if let Some(Frame::Item(blocks)) = stack.pop()
                        && let Some(Frame::List(_, items)) = stack.last_mut()
                    {
                        items.push(blocks);
                    }
                }
                TagEnd::List(_) => {
                    if let Some(Frame::List(start, items)) = stack.pop()
                        && let Some(top) = stack.last_mut()
                    {
                        top.blocks().push(Block::List { start, items });
                    }
                }
                TagEnd::CodeBlock => {
                    if let Some(code) = in_code_block.take()
                        && let Some(top) = stack.last_mut()
                    {
                        top.blocks().push(Block::Code(code.trim_end().to_owned()));
                    }
                }
                TagEnd::TableCell => row.push(std::mem::take(&mut spans)),
                TagEnd::TableHead => {
                    if let Some((_, head, _)) = table.as_mut() {
                        *head = std::mem::take(&mut row);
                    }
                    in_head = false;
                }
                TagEnd::TableRow => {
                    if !in_head && let Some((_, _, rows)) = table.as_mut() {
                        rows.push(std::mem::take(&mut row));
                    }
                }
                TagEnd::Table => {
                    if let Some((align, head, rows)) = table.take()
                        && let Some(top) = stack.last_mut()
                    {
                        top.blocks().push(Block::Table { align, head, rows });
                    }
                }
                TagEnd::Strong => bold = bold.saturating_sub(1),
                TagEnd::Emphasis => italic = italic.saturating_sub(1),
                TagEnd::Strikethrough => strike = strike.saturating_sub(1),
                TagEnd::Link => link = None,
                _ => {}
            },
            Event::Text(text) => {
                if let Some(code) = in_code_block.as_mut() {
                    code.push_str(&text);
                } else {
                    spans.push(Span {
                        text: text.into_string(),
                        bold: bold > 0,
                        italic: italic > 0,
                        strike: strike > 0,
                        link: link.clone(),
                        code: false,
                    });
                }
            }
            Event::Code(text) => spans.push(Span {
                text: text.into_string(),
                code: true,
                bold: bold > 0,
                italic: italic > 0,
                strike: strike > 0,
                link: link.clone(),
            }),
            Event::SoftBreak => spans.push(Span {
                text: " ".to_owned(),
                ..Span::default()
            }),
            Event::HardBreak => spans.push(Span {
                text: "\n".to_owned(),
                ..Span::default()
            }),
            Event::TaskListMarker(done) => spans.push(Span {
                text: if done { "☑ " } else { "☐ " }.to_owned(),
                ..Span::default()
            }),
            Event::Rule => {
                if let Some(top) = stack.last_mut() {
                    top.blocks().push(Block::Rule);
                }
            }
            _ => {}
        }
    }
    match stack.into_iter().next() {
        Some(Frame::Root(blocks)) => blocks,
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(spans: &[Span]) -> String {
        spans.iter().map(|s| s.text.as_str()).collect()
    }

    #[test]
    fn headings_paragraphs_and_inline_styles() {
        let doc = parse("# Title\n\nSome **bold** and *italic* and `code`.\n");
        assert_eq!(doc.len(), 2);
        assert!(matches!(&doc[0], Block::Heading(1, s) if text(s) == "Title"));
        let Block::Paragraph(spans) = &doc[1] else {
            panic!("{doc:?}")
        };
        assert!(spans.iter().any(|s| s.bold && s.text == "bold"));
        assert!(spans.iter().any(|s| s.italic && s.text == "italic"));
        assert!(spans.iter().any(|s| s.code && s.text == "code"));
    }

    #[test]
    fn tables_keep_head_rows_and_alignment() {
        let doc = parse("| Item | Cost |\n|:-----|-----:|\n| Rent | 1200 |\n| **Food** | 450 |\n");
        let Block::Table { align, head, rows } = &doc[0] else {
            panic!("{doc:?}")
        };
        assert_eq!(align, &vec![Align::Left, Align::Right]);
        assert_eq!(text(&head[1]), "Cost");
        assert_eq!(rows.len(), 2);
        assert!(rows[1][0][0].bold);
    }

    #[test]
    fn lists_nest_and_number() {
        let doc = parse("1. One\n2. Two\n   - inner\n- after\n");
        let Block::List { start, items } = &doc[0] else {
            panic!("{doc:?}")
        };
        assert_eq!(*start, Some(1));
        assert_eq!(items.len(), 2);
        assert!(
            items[1]
                .iter()
                .any(|b| matches!(b, Block::List { start: None, .. }))
        );
        assert!(matches!(&doc[1], Block::List { start: None, .. }));
    }

    #[test]
    fn code_quotes_rules_and_titles() {
        let doc = parse("> quoted\n\n---\n\n```rust\nfn x() {}\n```\n");
        assert!(matches!(&doc[0], Block::Quote(_)));
        assert!(matches!(&doc[1], Block::Rule));
        assert!(matches!(&doc[2], Block::Code(c) if c == "fn x() {}"));
        assert_eq!(
            title("intro\n\n# Report 1\n\n# Other"),
            Some("Report 1".to_owned())
        );
    }
}
