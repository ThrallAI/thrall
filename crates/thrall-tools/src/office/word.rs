//! Word documents (.docx): make a new one from markdown, or replace text in an existing
//! one while keeping its layout.

use std::io::{Read, Write};
use std::path::Path;

use docx_rs::{
    AbstractNumbering, AlignmentType, BorderType, BreakType, Docx, Footer, Header, IndentLevel,
    Level, LevelJc, LevelOverride, LevelText, LineSpacing, NumPages, NumberFormat, Numbering,
    NumberingId, PageMargin, PageNum, PageOrientationType, Paragraph, ParagraphBorder,
    ParagraphBorderPosition, ParagraphBorders, Run, RunFonts, Shading, SpecialIndentType, Start,
    Style, StyleType, Table, TableBorders, TableCell, TableCellBorder, TableCellBorderPosition,
    TableCellBorders, TableCellMargins, TableLayoutType, TableRow, VAlignType, WidthType,
};

use crate::constants::{
    WORD_ACCENT, WORD_BODY_FONT, WORD_CODE_FONT, WORD_HEADER_GAP, WORD_INK, WORD_LINE, WORD_MUTED,
    WORD_SOFT, WORD_TINT,
};
use crate::office::layout::Layout;
use crate::office::markdown::{self, Align, Block, Span};
/// One find-and-replace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Replacement {
    /// Text to find (exactly).
    pub find: String,
    /// What to put instead.
    pub replace: String,
}

// ---------- new documents ----------

/// Bullets use this numbering; numbered lists each get their own so they count from their start.
const BULLETS: usize = 1;
/// The numbered-list levels every numbered list is built on.
const NUMBERED_ABSTRACT: usize = 2;

fn fonts(name: &str) -> RunFonts {
    RunFonts::new()
        .ascii(name)
        .hi_ansi(name)
        .cs(name)
        .east_asia(name)
}

/// A heading style: `size` in half-points, spacing before/after in twentieths of a point.
fn heading_style(id: &str, name: &str, size: usize, color: &str, before: u32, after: u32) -> Style {
    let mut style = Style::new(id, StyleType::Paragraph)
        .name(name)
        .based_on("Normal")
        .next("Normal")
        .size(size)
        .bold()
        .color(color)
        .line_spacing(LineSpacing::new().before(before).after(after).line(252));
    style.paragraph_property = style.paragraph_property.keep_next(true);
    style
}

fn bottom_rule(size: usize, space: usize, color: &str) -> ParagraphBorders {
    ParagraphBorders::with_empty().set(
        ParagraphBorder::new(ParagraphBorderPosition::Bottom)
            .val(BorderType::Single)
            .size(size)
            .space(space)
            .color(color),
    )
}

/// Half-points (Word's unit for text size) for a size in points.
fn half_points(pt: f64) -> usize {
    // Layout sizes are checked to be 5-72 pt, so this always fits.
    format!("{:.0}", pt * 2.0).parse().unwrap_or(22)
}

/// Twentieths of a point for `em` of the body text (Word's unit for spacing).
fn twips(l: &Layout, em: f64) -> u32 {
    format!("{:.0}", l.body_pt * 20.0 * em).parse().unwrap_or(0)
}

fn styles(doc: Docx, l: &Layout) -> Docx {
    let (before, after) = if l.compact { (0.8, 0.3) } else { (1.6, 0.6) };
    let heading = |id: &str, name: &str, pt: f64, color: &str| {
        heading_style(
            id,
            name,
            half_points(pt),
            color,
            twips(l, before),
            twips(l, after),
        )
    };
    let mut title = heading_style(
        "Title",
        "Title",
        half_points(l.title_pt),
        WORD_INK,
        0,
        twips(l, if l.compact { 0.6 } else { 1.0 }),
    );
    title.paragraph_property =
        title
            .paragraph_property
            .set_borders(bottom_rule(12, 6, WORD_ACCENT));
    let mut h1 = heading("Heading1", "heading 1", l.heading_pt, WORD_INK).outline_lvl(0);
    h1.paragraph_property = h1
        .paragraph_property
        .set_borders(bottom_rule(4, 4, WORD_LINE));
    let mut code = Style::new("Code", StyleType::Paragraph)
        .name("Code")
        .based_on("Normal")
        .fonts(fonts(WORD_CODE_FONT))
        .size(half_points(l.body_pt * 0.84))
        .line_spacing(LineSpacing::new().before(0).after(0).line(240));
    code.paragraph_property = code
        .paragraph_property
        .shading(Shading::new().fill(WORD_TINT));
    let quote = Style::new("Quote", StyleType::Paragraph)
        .name("Quote")
        .based_on("Normal")
        .italic()
        .color(WORD_MUTED)
        .indent(Some(360), None, None, None);
    doc.add_style(title)
        .add_style(h1)
        .add_style(heading("Heading2", "heading 2", l.heading_pt, WORD_INK).outline_lvl(1))
        .add_style(heading("Heading3", "heading 3", l.subheading_pt, WORD_ACCENT).outline_lvl(2))
        .add_style(heading("Heading4", "heading 4", l.subheading_pt, WORD_MUTED).outline_lvl(3))
        .add_style(code)
        .add_style(quote)
}

/// Three levels of bullets (• ◦ ▪) or numbers (1. a. i.).
fn list_levels(numbered: bool) -> AbstractNumbering {
    let id = if numbered { NUMBERED_ABSTRACT } else { BULLETS };
    let mut abs = AbstractNumbering::new(id);
    for (level, left) in [(0usize, 400i32), (1, 760), (2, 1120)] {
        let (format, text) = if numbered {
            let f = ["decimal", "lowerLetter", "lowerRoman"][level];
            (f, format!("%{}.", level + 1))
        } else {
            ("bullet", ["•", "◦", "▪"][level].to_owned())
        };
        abs = abs.add_level(
            Level::new(
                level,
                Start::new(1),
                NumberFormat::new(format),
                LevelText::new(text),
                LevelJc::new("left"),
            )
            .indent(
                Some(left),
                Some(SpecialIndentType::Hanging(280)),
                None,
                None,
            ),
        );
    }
    abs
}

fn styled(mut run: Run, span: &Span, code_size: usize) -> Run {
    if span.bold {
        run = run.bold();
    }
    if span.italic {
        run = run.italic();
    }
    if span.strike {
        run = run.strike();
    }
    if span.code {
        run = run
            .fonts(fonts(WORD_CODE_FONT))
            .size(code_size)
            .shading(Shading::new().fill(WORD_TINT));
    }
    if span.link.is_some() {
        run = run.color(WORD_ACCENT).underline("single");
    }
    run
}

/// Inline spans as runs, each started by `base` (for a table cell's size, say); inline code is
/// `code_size` half-points.
fn runs(
    mut p: Paragraph,
    spans: &[Span],
    base: impl Fn(Run) -> Run,
    code_size: usize,
) -> Paragraph {
    for span in spans {
        let mut text = span.text.as_str();
        // Hard line breaks inside the paragraph.
        while let Some(i) = text.find('\n') {
            let run = styled(base(Run::new().add_text(&text[..i])), span, code_size);
            p = p.add_run(run.add_break(BreakType::TextWrapping));
            text = &text[i + 1..];
        }
        if !text.is_empty() {
            p = p.add_run(styled(base(Run::new().add_text(text)), span, code_size));
        }
    }
    p
}

fn table_cell(
    l: &Layout,
    spans: &[Span],
    align: Option<&Align>,
    header: bool,
    fill: Option<&str>,
) -> TableCell {
    let jc = match align {
        Some(Align::Center) => AlignmentType::Center,
        Some(Align::Right) => AlignmentType::Right,
        _ => AlignmentType::Left,
    };
    let size = half_points(l.body_pt * 0.92);
    let p = runs(
        Paragraph::new()
            .align(jc)
            .line_spacing(LineSpacing::new().before(0).after(0).line(252)),
        spans,
        |r| {
            let r = r.size(size);
            if header { r.bold() } else { r }
        },
        half_points(l.body_pt * 0.8),
    );
    let (border, color) = if header {
        (12, WORD_ACCENT)
    } else {
        (4, WORD_LINE)
    };
    let mut cell = TableCell::new()
        .add_paragraph(p)
        .vertical_align(VAlignType::Top)
        .set_borders(
            TableCellBorders::with_empty().set(
                TableCellBorder::new(TableCellBorderPosition::Bottom)
                    .border_type(BorderType::Single)
                    .size(border)
                    .color(color),
            ),
        );
    if let Some(fill) = fill {
        cell = cell.shading(Shading::new().fill(fill));
    }
    cell
}

/// Writes the document's blocks, keeping track of list numbering.
struct Writer {
    doc: Docx,
    layout: Layout,
    next_numbering: usize,
    titled: bool,
}

impl Writer {
    fn push(&mut self, p: Paragraph) {
        self.doc = std::mem::take(&mut self.doc).add_paragraph(p);
    }

    fn code_size(&self) -> usize {
        half_points(self.layout.body_pt * 0.86)
    }

    /// One block; `list` is the numbering and depth when it's a list item's first paragraph.
    fn block(&mut self, block: &Block, list: Option<(usize, usize)>) {
        let code = self.code_size();
        match block {
            Block::Heading(level, spans) => {
                let style = match level {
                    1 if !self.titled => "Title",
                    1 => "Heading1",
                    2 => "Heading2",
                    3 => "Heading3",
                    _ => "Heading4",
                };
                self.titled = true;
                self.push(runs(Paragraph::new().style(style), spans, |r| r, code));
            }
            Block::Paragraph(spans) => {
                let mut p = Paragraph::new();
                if let Some((id, depth)) = list {
                    let after = twips(&self.layout, if self.layout.compact { 0.1 } else { 0.25 });
                    p = p
                        .numbering(NumberingId::new(id), IndentLevel::new(depth))
                        .line_spacing(LineSpacing::new().after(after));
                }
                self.push(runs(p, spans, |r| r, code));
            }
            Block::List { start, items } => self.list(*start, items, list),
            Block::Table { align, head, rows } => self.table(align, head, rows),
            Block::Code(text) => {
                let lines: Vec<&str> = text.lines().collect();
                let last = lines.len().saturating_sub(1);
                for (i, line) in lines.iter().enumerate() {
                    // Room above the first line and below the last.
                    let mut spacing = LineSpacing::new().line(240);
                    if i == 0 {
                        spacing = spacing.before(twips(&self.layout, 0.4));
                    }
                    if i == last {
                        spacing = spacing.after(twips(&self.layout, 0.9));
                    }
                    let text = if line.is_empty() { " " } else { line };
                    self.push(
                        Paragraph::new()
                            .style("Code")
                            .keep_next(i < last)
                            .line_spacing(spacing)
                            .add_run(Run::new().add_text(text)),
                    );
                }
            }
            Block::Quote(blocks) => {
                for b in blocks {
                    if let Block::Paragraph(spans) = b {
                        self.push(runs(Paragraph::new().style("Quote"), spans, |r| r, code));
                    } else {
                        self.block(b, None);
                    }
                }
            }
            Block::Rule => {
                let (before, after) = if self.layout.compact {
                    (0.2, 0.4)
                } else {
                    (0.4, 0.8)
                };
                let spacing = LineSpacing::new()
                    .before(twips(&self.layout, before))
                    .after(twips(&self.layout, after))
                    .line(120);
                let mut p = Paragraph::new().line_spacing(spacing);
                p.property = p.property.set_borders(bottom_rule(4, 1, WORD_LINE));
                self.push(p);
            }
        }
    }

    fn list(&mut self, start: Option<u64>, items: &[Vec<Block>], outer: Option<(usize, usize)>) {
        let depth = outer.map_or(0, |(_, d)| (d + 1).min(2));
        let id = match start {
            Some(n) => {
                self.next_numbering += 1;
                let id = self.next_numbering;
                let first = usize::try_from(n).unwrap_or(1);
                self.doc = std::mem::take(&mut self.doc).add_numbering(
                    Numbering::new(id, NUMBERED_ABSTRACT)
                        .add_override(LevelOverride::new(depth).start(first)),
                );
                id
            }
            None => BULLETS,
        };
        for item in items {
            // An empty item (a blank to fill in) still gets its number or bullet.
            if !item.iter().any(|b| matches!(b, Block::Paragraph(_))) {
                self.block(&Block::Paragraph(Vec::new()), Some((id, depth)));
            }
            // Only the item's first paragraph carries the number or bullet.
            let mut first = true;
            for b in item {
                match b {
                    Block::Paragraph(_) if first => {
                        self.block(b, Some((id, depth)));
                        first = false;
                    }
                    Block::List { .. } => self.block(b, Some((id, depth))),
                    other => self.block(other, None),
                }
            }
        }
    }

    fn table(&mut self, align: &[Align], head: &[Vec<Span>], rows: &[Vec<Vec<Span>>]) {
        let l = self.layout.clone();
        let mut table_rows = Vec::new();
        if !head.is_empty() {
            let cells = head
                .iter()
                .enumerate()
                .map(|(i, s)| table_cell(&l, s, align.get(i), true, Some(WORD_SOFT)))
                .collect();
            table_rows.push(TableRow::new(cells).cant_split());
        }
        for (r, row) in rows.iter().enumerate() {
            let fill = (r % 2 == 1).then_some(WORD_TINT);
            let cells = row
                .iter()
                .enumerate()
                .map(|(i, s)| table_cell(&l, s, align.get(i), false, fill))
                .collect();
            table_rows.push(TableRow::new(cells).cant_split());
        }
        let pad = if l.compact { 50 } else { 90 };
        let table = Table::new(table_rows)
            .width(5000, WidthType::Pct)
            .layout(TableLayoutType::Autofit)
            .set_borders(TableBorders::with_empty())
            .margins(TableCellMargins::new().margin(pad, 140, pad, 140));
        let after = twips(&l, if l.compact { 0.3 } else { 0.6 });
        self.doc = std::mem::take(&mut self.doc)
            .add_table(table)
            // Breathing room after the table.
            .add_paragraph(
                Paragraph::new().line_spacing(LineSpacing::new().before(0).after(after)),
            );
    }
}

/// The running header (the title) and footer (page X of Y).
fn header_footer(doc: Docx, title: &str) -> Docx {
    // A style rather than run formatting, so the page-number fields come out the same size.
    let small = Style::new("HeaderFooter", StyleType::Paragraph)
        .name("Header and footer")
        .based_on("Normal")
        .size(16)
        .color(WORD_MUTED)
        .align(AlignmentType::Right)
        .line_spacing(LineSpacing::new().before(0).after(0));
    let mut doc = doc.add_style(small).footer(
        Footer::new().add_paragraph(
            Paragraph::new()
                .style("HeaderFooter")
                .add_run(Run::new().add_text("Page "))
                .add_page_num(PageNum::new())
                .add_run(Run::new().add_text(" of "))
                .add_num_pages(NumPages::new()),
        ),
    );
    if !title.is_empty() {
        doc = doc.header(
            Header::new().add_paragraph(
                Paragraph::new()
                    .style("HeaderFooter")
                    .add_run(Run::new().add_text(title)),
            ),
        );
    }
    doc
}

/// Millimetres in Word's twentieths of a point.
fn mm_twips(mm: f64) -> i32 {
    format!("{:.0}", mm * 56.693).parse().unwrap_or(1134)
}

/// The finished document for `text` laid out by `l`.
fn build(text: &str, l: &Layout) -> Docx {
    let (w, h) = l.paper.mm();
    let (w, h) = if l.landscape { (h, w) } else { (w, h) };
    let margin = mm_twips(l.margin_mm);
    let line = if l.compact { 240 } else { 276 };
    let after = twips(l, if l.compact { 0.3 } else { 0.7 });
    let mut doc = Docx::new()
        .page_size(
            u32::try_from(mm_twips(w)).unwrap_or(11906),
            u32::try_from(mm_twips(h)).unwrap_or(16838),
        )
        .page_margin(
            PageMargin::new()
                .top(margin)
                .bottom(margin)
                .left(margin)
                .right(margin)
                .header(WORD_HEADER_GAP)
                .footer(WORD_HEADER_GAP),
        )
        .default_fonts(fonts(WORD_BODY_FONT))
        .default_size(half_points(l.body_pt))
        .default_line_spacing(LineSpacing::new().after(after).line(line))
        .add_abstract_numbering(list_levels(false))
        .add_abstract_numbering(list_levels(true))
        .add_numbering(Numbering::new(BULLETS, BULLETS));
    if l.landscape {
        doc = doc.page_orient(PageOrientationType::Landscape);
    }
    let mut doc = styles(doc, l);
    if l.header_footer {
        doc = header_footer(doc, &markdown::title(text).unwrap_or_default());
    }
    let mut writer = Writer {
        doc,
        layout: l.clone(),
        next_numbering: NUMBERED_ABSTRACT,
        titled: false,
    };
    for block in &markdown::parse(text) {
        writer.block(block, None);
    }
    writer.doc
}

/// A new document from markdown: headings (the first `#` is the title), paragraphs, bullet and
/// numbered lists, tables, code, quotes and rules; bold, italic, code and links inline. Sizes,
/// spacing, paper and the running header and footer come from `layout`.
pub fn create(path: &Path, text: &str, layout: &Layout) -> Result<(), String> {
    let doc = build(text, layout);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("couldn't make the folder: {e}"))?;
    }
    let file = std::fs::File::create(path).map_err(|e| format!("couldn't create it: {e}"))?;
    doc.build()
        .pack(file)
        .map_err(|e| format!("couldn't write the document: {e}"))
}

// ---------- replacing text ----------

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// A text node inside a paragraph: where its content sits in the XML, and its text.
struct TextNode {
    start: usize,
    end: usize,
    text: String,
}

/// The `<w:t>` nodes between `from` and `to`.
fn text_nodes(xml: &str, from: usize, to: usize) -> Vec<TextNode> {
    let mut nodes = Vec::new();
    let mut at = from;
    while let Some(i) = xml[at..to].find("<w:t") {
        let open = at + i;
        // `<w:t>` or `<w:t xml:space="preserve">`, not `<w:tab/>`, `<w:tbl>` etc.
        let after = xml.as_bytes().get(open + 4).copied();
        if !matches!(after, Some(b'>' | b' ')) {
            at = open + 4;
            continue;
        }
        let Some(gt) = xml[open..to].find('>') else {
            break;
        };
        let content_start = open + gt + 1;
        if xml[..content_start].ends_with("/>") {
            at = content_start;
            continue;
        }
        let Some(close) = xml[content_start..to].find("</w:t>") else {
            break;
        };
        let content_end = content_start + close;
        nodes.push(TextNode {
            start: content_start,
            end: content_end,
            text: unescape(&xml[content_start..content_end]),
        });
        at = content_end + 6;
    }
    nodes
}

/// Paragraph spans `(start, end)` in document.xml.
fn paragraphs(xml: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(i) = xml[at..].find("<w:p") {
        let start = at + i;
        let after = xml.as_bytes().get(start + 4).copied();
        if !matches!(after, Some(b'>' | b' ')) {
            at = start + 4;
            continue;
        }
        let Some(close) = xml[start..].find("</w:p>") else {
            break;
        };
        let end = start + close + 6;
        out.push((start, end));
        at = end;
    }
    out
}

/// What replacing would do: the new XML, each changed paragraph's old and new text, and how many
/// replacements were made. A match inside one run keeps that run's formatting; a match spread
/// over several runs puts the paragraph's new text in its first run.
pub fn replace_in_xml(
    xml: &str,
    replacements: &[Replacement],
) -> (String, Vec<(String, String)>, usize) {
    let mut out = String::with_capacity(xml.len());
    let mut changed = Vec::new();
    let mut count = 0;
    let mut copied = 0;
    for (p_start, p_end) in paragraphs(xml) {
        let nodes = text_nodes(xml, p_start, p_end);
        if nodes.is_empty() {
            continue;
        }
        let old_text: String = nodes.iter().map(|n| n.text.as_str()).collect();
        let mut texts: Vec<String> = nodes.iter().map(|n| n.text.clone()).collect();
        let mut merged = false;
        for r in replacements {
            if r.find.is_empty() {
                continue;
            }
            let whole: String = texts.concat();
            let found = whole.matches(r.find.as_str()).count();
            if found == 0 {
                continue;
            }
            count += found;
            // Every match within single runs: replace in place.
            let in_runs: usize = texts
                .iter()
                .map(|t| t.matches(r.find.as_str()).count())
                .sum();
            if in_runs == found {
                for t in &mut texts {
                    *t = t.replace(r.find.as_str(), &r.replace);
                }
            } else {
                let new_whole = whole.replace(r.find.as_str(), &r.replace);
                texts = std::iter::once(new_whole)
                    .chain(std::iter::repeat_n(String::new(), texts.len() - 1))
                    .collect();
                merged = true;
            }
        }
        let new_text: String = texts.concat();
        if new_text == old_text && !merged {
            continue;
        }
        changed.push((old_text, new_text));
        for (node, text) in nodes.iter().zip(&texts) {
            out.push_str(&xml[copied..node.start]);
            // Keep leading and trailing spaces: the node must preserve them.
            if !xml[..node.start].ends_with("xml:space=\"preserve\">") && text.trim() != text {
                let tag_start = xml[..node.start].rfind("<w:t").unwrap_or(node.start);
                // Rewrite `<w:t>` as `<w:t xml:space="preserve">`.
                out.truncate(out.len() - (node.start - tag_start));
                out.push_str("<w:t xml:space=\"preserve\">");
            }
            out.push_str(&escape(text));
            copied = node.end;
        }
    }
    out.push_str(&xml[copied..]);
    (out, changed, count)
}

fn document_xml(path: &Path) -> Result<String, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("couldn't open it: {e}"))?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|e| format!("it isn't a valid Word file: {e}"))?;
    let mut entry = archive
        .by_name("word/document.xml")
        .map_err(|_| "it has no document text (is it really a .docx?)".to_owned())?;
    let mut xml = String::new();
    entry
        .read_to_string(&mut xml)
        .map_err(|e| format!("couldn't read it: {e}"))?;
    Ok(xml)
}

/// The preview: the changed paragraphs before and after, and how many replacements.
pub fn preview(
    path: &Path,
    replacements: &[Replacement],
) -> Result<(String, String, usize), String> {
    let xml = document_xml(path)?;
    let (_, changed, count) = replace_in_xml(&xml, replacements);
    let old: String = changed.iter().map(|(o, _)| format!("{o}\n")).collect();
    let new: String = changed.iter().map(|(_, n)| format!("{n}\n")).collect();
    Ok((old, new, count))
}

/// Replace text and save. Returns how many replacements; an error if nothing matched.
pub fn replace(path: &Path, replacements: &[Replacement]) -> Result<usize, String> {
    let xml = document_xml(path)?;
    let (new_xml, _, count) = replace_in_xml(&xml, replacements);
    if count == 0 {
        let finds: Vec<&str> = replacements.iter().map(|r| r.find.as_str()).collect();
        return Err(format!(
            "none of the text was found: {} (it must match exactly)",
            finds.join(", ")
        ));
    }
    // Copy every other part as it is; write the new document text.
    let source = std::fs::File::open(path).map_err(|e| format!("couldn't open it: {e}"))?;
    let mut archive = zip::ZipArchive::new(source).map_err(|e| format!("couldn't read it: {e}"))?;
    let temp = path.with_extension("docx.thrall-tmp");
    {
        let out = std::fs::File::create(&temp).map_err(|e| format!("couldn't write: {e}"))?;
        let mut writer = zip::ZipWriter::new(out);
        for i in 0..archive.len() {
            let entry = archive
                .by_index_raw(i)
                .map_err(|e| format!("couldn't read it: {e}"))?;
            if entry.name().is_ok_and(|n| n == "word/document.xml") {
                drop(entry);
                let options = zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Deflated);
                writer
                    .start_file("word/document.xml", options)
                    .map_err(|e| format!("couldn't write: {e}"))?;
                writer
                    .write_all(new_xml.as_bytes())
                    .map_err(|e| format!("couldn't write: {e}"))?;
            } else {
                writer
                    .raw_copy_file(entry)
                    .map_err(|e| format!("couldn't write: {e}"))?;
            }
        }
        writer
            .finish()
            .map_err(|e| format!("couldn't write: {e}"))?;
    }
    drop(archive);
    std::fs::rename(&temp, path).map_err(|e| {
        let _ = std::fs::remove_file(&temp);
        format!("couldn't replace the file (is it open in Word?): {e}")
    })?;
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(find: &str, replace: &str) -> Replacement {
        Replacement {
            find: find.into(),
            replace: replace.into(),
        }
    }

    #[test]
    fn new_documents_get_real_styles_tables_and_lists() {
        let dir = std::env::temp_dir().join(format!("thrall-word-{}", std::process::id()));
        let path = dir.join("report.docx");
        create(
            &path,
            "# Report\n\n## Costs\n\n| Item | Cost |\n|---|--:|\n| Rent | 1200 |\n\n\
             - one\n- two\n\n1. first\n2. second\n\n> quoted\n\n---\n\n```\ncode\n```\n",
            &Layout::default(),
        )
        .unwrap();
        let xml = document_xml(&path).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        assert!(xml.contains(r#"<w:pStyle w:val="Title" />"#), "{xml}");
        assert!(xml.contains(r#"<w:pStyle w:val="Heading2" />"#));
        assert!(xml.contains("<w:tbl>"), "tables are real tables");
        assert!(!xml.contains("|---"), "no markdown left over");
        assert!(xml.contains("<w:numPr>"), "lists are numbered by Word");
        assert!(xml.contains(r#"<w:pStyle w:val="Quote" />"#));
        assert!(xml.contains(r#"<w:pStyle w:val="Code" />"#));
        assert!(
            xml.contains(r#"<w:jc w:val="right" />"#),
            "column alignment kept"
        );
    }

    #[test]
    fn empty_list_items_keep_their_numbers() {
        let dir = std::env::temp_dir().join(format!("thrall-word-empty-{}", std::process::id()));
        let path = dir.join("form.docx");
        create(&path, "Three things:\n\n1.\n2.\n3.\n", &Layout::default()).unwrap();
        let xml = document_xml(&path).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(xml.matches("<w:numPr>").count(), 3, "{xml}");
    }

    #[test]
    fn a_match_inside_one_run_keeps_its_formatting() {
        let xml =
            r#"<w:body><w:p><w:r><w:rPr><w:b/></w:rPr><w:t>Dear Sam,</w:t></w:r></w:p></w:body>"#;
        let (out, changed, count) = replace_in_xml(xml, &[r("Sam", "Alex")]);
        assert_eq!(count, 1);
        assert_eq!(
            changed,
            vec![("Dear Sam,".to_owned(), "Dear Alex,".to_owned())]
        );
        assert!(
            out.contains("<w:rPr><w:b/></w:rPr><w:t>Dear Alex,</w:t>"),
            "{out}"
        );
    }

    #[test]
    fn a_match_across_runs_is_replaced_whole() {
        let xml = r#"<w:p><w:r><w:t>Due </w:t></w:r><w:r><w:t>Fri</w:t></w:r><w:r><w:t>day.</w:t></w:r></w:p>"#;
        let (out, changed, count) = replace_in_xml(xml, &[r("Friday", "Monday")]);
        assert_eq!(count, 1);
        assert_eq!(changed[0].1, "Due Monday.");
        let nodes = text_nodes(&out, 0, out.len());
        let text: String = nodes.iter().map(|n| n.text.as_str()).collect();
        assert_eq!(text, "Due Monday.");
    }

    #[test]
    fn entities_and_spaces_survive() {
        let xml = r#"<w:p><w:r><w:t>Fish &amp; chips</w:t></w:r></w:p><w:p><w:r><w:tab/><w:t>x</w:t></w:r></w:p>"#;
        let (out, _, count) = replace_in_xml(xml, &[r("Fish", "Salmon "), r("x", " y ")]);
        assert_eq!(count, 2);
        assert!(out.contains("Salmon  &amp; chips"), "{out}");
        assert!(
            out.contains("<w:tab/><w:t xml:space=\"preserve\"> y </w:t>"),
            "{out}"
        );
    }
}
