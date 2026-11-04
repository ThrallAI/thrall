//! Text out of rich documents: PDF, Word (.docx), PowerPoint (.pptx) and spreadsheets (.xlsx,
//! .xlsm, .xls, .ods). Used for files the user attaches and by the `read_document` tool.

use std::io::Read;
use std::path::Path;

use quick_xml::events::Event;

use crate::constants::{
    PDF_WIDTH_RANGE_MAX, SCANNED_PDF_CHARS_PER_PAGE, SHEET_MAX_ROWS, SLIDES_MAX,
};

/// What kind of document a file is, from its extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// PDF.
    Pdf,
    /// Word.
    Word,
    /// PowerPoint.
    Slides,
    /// Excel or OpenDocument spreadsheet.
    Spreadsheet,
}

impl Kind {
    /// The kind of `path`, if it's a document this module reads.
    pub fn of(path: &Path) -> Option<Self> {
        let ext = path.extension()?.to_string_lossy().to_lowercase();
        match ext.as_str() {
            "pdf" => Some(Self::Pdf),
            "docx" => Some(Self::Word),
            "pptx" => Some(Self::Slides),
            "xlsx" | "xlsm" | "xls" | "ods" => Some(Self::Spreadsheet),
            _ => None,
        }
    }

    /// In words, for messages.
    pub fn label(self) -> &'static str {
        match self {
            Self::Pdf => "PDF",
            Self::Word => "Word document",
            Self::Slides => "PowerPoint deck",
            Self::Spreadsheet => "spreadsheet",
        }
    }
}

/// A document's text and a short note about its size ("3 pages", "2 sheets").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document {
    /// What it is.
    pub kind: Kind,
    /// The text, with page, slide or sheet headings.
    pub text: String,
    /// Its size in words.
    pub detail: String,
    /// A PDF that's mostly pictures of pages (little text per page): its text needs reading off
    /// the pages.
    pub scanned: bool,
}

/// Read a document's text. Slow for big files: call it off the async runtime.
pub fn extract(path: &Path) -> Result<Document, String> {
    let kind = Kind::of(path)
        .ok_or_else(|| "not a PDF, Word, PowerPoint or spreadsheet file".to_owned())?;
    let ((text, detail), scanned) = match kind {
        Kind::Pdf => {
            let (text, detail, scanned) = pdf(path)?;
            ((text, detail), scanned)
        }
        Kind::Word => (word(path)?, false),
        Kind::Slides => (slides(path)?, false),
        Kind::Spreadsheet => (spreadsheet(path)?, false),
    };
    Ok(Document {
        kind,
        text,
        detail,
        scanned,
    })
}

fn plural(n: usize, one: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {one}s")
    }
}

/// A font's `W` (widths) array with every range (`first last width`) written out as a plain
/// list (`first [width width …]`), or `None` when it has no ranges or isn't laid out as expected.
/// pdf-extract 0.12 misreads ranges: their glyphs fall back to the default width, so it sees gaps
/// that aren't there and splits words ("P romptly"). Chrome writes ranges in every PDF it prints.
fn widths_as_lists(w: &[lopdf::Object]) -> Option<Vec<lopdf::Object>> {
    use lopdf::Object;
    let mut out = Vec::with_capacity(w.len());
    let mut ranges = false;
    let mut i = 0;
    while i < w.len() {
        let first = w[i].as_i64().ok()?;
        match w.get(i + 1)? {
            Object::Array(list) => {
                out.push(Object::Integer(first));
                out.push(Object::Array(list.clone()));
                i += 2;
            }
            last => {
                let last = last.as_i64().ok()?;
                let width = w.get(i + 2)?;
                width.as_float().ok()?;
                if last < first || last - first >= PDF_WIDTH_RANGE_MAX {
                    return None;
                }
                let count = usize::try_from(last - first + 1).ok()?;
                out.push(Object::Integer(first));
                out.push(Object::Array(vec![width.clone(); count]));
                ranges = true;
                i += 3;
            }
        }
    }
    ranges.then_some(out)
}

/// The PDF with its fonts' width ranges written as plain lists (see [`widths_as_lists`]), as
/// bytes to read the text from. `None` when nothing needs changing or it can't be done (the file
/// is then read as it is). The file itself is never changed.
fn with_plain_widths(path: &Path) -> Option<Vec<u8>> {
    use lopdf::Object;
    let mut doc = lopdf::Document::load(path).ok()?;
    if doc.is_encrypted() {
        return None;
    }
    // Where each font's widths live: in the font itself, or in an object of their own.
    let mut inline = Vec::new();
    let mut separate = Vec::new();
    for (id, obj) in &doc.objects {
        let Ok(font) = obj.as_dict() else { continue };
        let cid = matches!(
            font.get(b"Subtype").and_then(Object::as_name),
            Ok(b"CIDFontType0" | b"CIDFontType2")
        );
        let Some(w) = cid.then(|| font.get(b"W").ok()).flatten() else {
            continue;
        };
        match w {
            Object::Array(list) => {
                if let Some(plain) = widths_as_lists(list) {
                    inline.push((*id, plain));
                }
            }
            Object::Reference(target) => {
                if let Some(plain) = doc
                    .get_object(*target)
                    .and_then(Object::as_array)
                    .ok()
                    .and_then(|list| widths_as_lists(list))
                {
                    separate.push((*target, plain));
                }
            }
            _ => {}
        }
    }
    if inline.is_empty() && separate.is_empty() {
        return None;
    }
    for (id, plain) in inline {
        if let Some(font) = doc.objects.get_mut(&id).and_then(|o| o.as_dict_mut().ok()) {
            font.set("W", Object::Array(plain));
        }
    }
    for (id, plain) in separate {
        doc.objects.insert(id, Object::Array(plain));
    }
    let mut bytes = Vec::new();
    doc.save_to(&mut bytes).ok()?;
    Some(bytes)
}

fn pdf(path: &Path) -> Result<(String, String, bool), String> {
    let pages = with_plain_widths(path)
        .and_then(|bytes| pdf_extract::extract_text_from_mem_by_pages(&bytes).ok())
        .map_or_else(|| pdf_extract::extract_text_by_pages(path), Ok)
        .map_err(|e| format!("couldn't read the PDF: {e}"))?;
    let mut out = String::new();
    for (i, page) in pages.iter().enumerate() {
        let page = page.trim();
        if page.is_empty() {
            continue;
        }
        out.push_str(&format!("--- Page {} ---\n{page}\n\n", i + 1));
    }
    let mut detail = plural(pages.len(), "page");
    let chars: usize = pages.iter().map(|p| p.trim().chars().count()).sum();
    let scanned = !pages.is_empty() && chars / pages.len() < SCANNED_PDF_CHARS_PER_PAGE;
    if out.trim().is_empty() {
        detail.push_str(", no text found: the pages are pictures");
    } else if scanned {
        detail.push_str(", mostly pictures of pages: little text could be pulled out");
    }
    Ok((out.trim_end().to_owned(), detail, scanned))
}

/// One XML file out of a zip-based Office document.
fn zip_entry(archive: &mut zip::ZipArchive<std::fs::File>, name: &str) -> Option<String> {
    let mut entry = archive.by_name(name).ok()?;
    let mut text = String::new();
    entry.read_to_string(&mut text).ok()?;
    Some(text)
}

fn open_zip(path: &Path) -> Result<zip::ZipArchive<std::fs::File>, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("couldn't open it: {e}"))?;
    zip::ZipArchive::new(file).map_err(|e| format!("it isn't a valid Office file: {e}"))
}

/// The readable text in Office XML: `<w:t>`/`<a:t>` runs, with paragraphs on their own lines,
/// table cells split by " | " and tabs and line breaks kept.
fn office_text(xml: &str) -> String {
    let mut reader = quick_xml::Reader::from_str(xml);
    let mut out = String::new();
    let mut in_text = false;
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                if e.local_name().as_ref() == "t" {
                    in_text = true;
                }
            }
            Ok(Event::Empty(e)) => match e.local_name().as_ref() {
                "tab" => out.push('\t'),
                "br" | "cr" => out.push('\n'),
                _ => {}
            },
            Ok(Event::End(e)) => match e.local_name().as_ref() {
                "t" => in_text = false,
                "p" => out.push('\n'),
                "tc" => out.push_str(" | "),
                "tr" => out.push('\n'),
                _ => {}
            },
            Ok(Event::Text(t)) if in_text => {
                out.push_str(&t.xml10_content());
            }
            Ok(Event::GeneralRef(r)) if in_text => {
                if let Ok(Some(c)) = r.resolve_char_ref() {
                    out.push(c);
                } else {
                    out.push_str(match &*r.xml10_content() {
                        "amp" => "&",
                        "lt" => "<",
                        "gt" => ">",
                        "quot" => "\"",
                        "apos" => "'",
                        _ => "",
                    });
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    // Collapse runs of blank lines.
    let mut tidy = String::new();
    let mut blank = 0;
    for line in out.lines() {
        if line.trim().is_empty() {
            blank += 1;
            if blank > 1 {
                continue;
            }
        } else {
            blank = 0;
        }
        tidy.push_str(line.trim_end());
        tidy.push('\n');
    }
    tidy.trim().to_owned()
}

fn word(path: &Path) -> Result<(String, String), String> {
    let mut archive = open_zip(path)?;
    let xml = zip_entry(&mut archive, "word/document.xml")
        .ok_or_else(|| "it has no document text (is it really a .docx?)".to_owned())?;
    let text = office_text(&xml);
    let words = text.split_whitespace().count();
    Ok((text, plural(words, "word")))
}

fn slides(path: &Path) -> Result<(String, String), String> {
    let mut archive = open_zip(path)?;
    let mut out = String::new();
    let mut count = 0;
    for n in 1..=SLIDES_MAX {
        let Some(xml) = zip_entry(&mut archive, &format!("ppt/slides/slide{n}.xml")) else {
            break;
        };
        count = n;
        let text = office_text(&xml);
        if !text.is_empty() {
            out.push_str(&format!("--- Slide {n} ---\n{text}\n\n"));
        }
    }
    if count == 0 {
        return Err("it has no slides (is it really a .pptx?)".to_owned());
    }
    Ok((out.trim_end().to_owned(), plural(count, "slide")))
}

fn spreadsheet(path: &Path) -> Result<(String, String), String> {
    use calamine::Reader;
    let mut book = calamine::open_workbook_auto(path)
        .map_err(|e| format!("couldn't read the spreadsheet: {e}"))?;
    let names = book.sheet_names().to_owned();
    let mut out = String::new();
    for name in &names {
        let Ok(range) = book.worksheet_range(name) else {
            continue;
        };
        out.push_str(&format!("--- Sheet: {name} ---\n"));
        let total = range.height();
        for row in range.rows().take(SHEET_MAX_ROWS) {
            let cells: Vec<String> = row.iter().map(ToString::to_string).collect();
            // Skip rows that are all empty.
            if cells.iter().any(|c| !c.is_empty()) {
                out.push_str(&cells.join("\t"));
                out.push('\n');
            }
        }
        if total > SHEET_MAX_ROWS {
            out.push_str(&format!(
                "… {} more rows not shown\n",
                total - SHEET_MAX_ROWS
            ));
        }
        out.push('\n');
    }
    Ok((out.trim_end().to_owned(), plural(names.len(), "sheet")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_come_from_the_extension() {
        assert_eq!(Kind::of(Path::new("a/Report.PDF")), Some(Kind::Pdf));
        assert_eq!(Kind::of(Path::new("notes.docx")), Some(Kind::Word));
        assert_eq!(Kind::of(Path::new("deck.pptx")), Some(Kind::Slides));
        assert_eq!(Kind::of(Path::new("budget.xls")), Some(Kind::Spreadsheet));
        assert_eq!(Kind::of(Path::new("notes.txt")), None);
    }

    #[test]
    fn width_ranges_become_plain_lists() {
        use lopdf::Object::{Array, Integer, Real};
        // As Chrome writes them: a list, then a range (glyphs 46 to 51 all 667 wide).
        let w = vec![
            Integer(37),
            Array(vec![Real(667.0), Real(722.0)]),
            Integer(46),
            Integer(51),
            Real(667.0),
        ];
        let plain = widths_as_lists(&w).unwrap();
        assert_eq!(plain.len(), 4);
        assert_eq!(plain[0], Integer(37));
        assert_eq!(plain[2], Integer(46));
        assert_eq!(plain[3], Array(vec![Real(667.0); 6]));
        // Only lists already: nothing to change. Anything odd: left alone.
        assert!(widths_as_lists(&w[..2]).is_none());
        assert!(widths_as_lists(&[Integer(5), Integer(1), Real(500.0)]).is_none());
        assert!(widths_as_lists(&[Integer(1), Integer(2)]).is_none());
        assert!(widths_as_lists(&[Integer(0), Integer(1_000_000), Real(500.0)]).is_none());
    }

    #[test]
    fn office_xml_keeps_paragraphs_tables_and_entities() {
        let xml = r#"<w:document xmlns:w="w"><w:body>
            <w:p><w:r><w:t>Fish &amp; chips</w:t></w:r><w:r><w:tab/><w:t>$12</w:t></w:r></w:p>
            <w:tbl><w:tr><w:tc><w:p><w:r><w:t>A1</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>B1</w:t></w:r></w:p></w:tc></w:tr></w:tbl>
            <w:p><w:r><w:t xml:space="preserve">Last line</w:t></w:r></w:p>
        </w:body></w:document>"#;
        let text = office_text(xml);
        assert!(text.contains("Fish & chips\t$12"), "{text}");
        assert!(text.contains("A1"), "{text}");
        assert!(text.contains("B1"), "{text}");
        assert!(text.ends_with("Last line"), "{text}");
    }
}
