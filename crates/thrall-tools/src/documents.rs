//! Text out of rich documents: PDF, Word (.docx), PowerPoint (.pptx) and spreadsheets (.xlsx,
//! .xlsm, .xls, .ods). Used for files the user attaches and by the `read_document` tool.

use std::io::Read;
use std::path::Path;

use quick_xml::events::Event;

use crate::constants::{SCANNED_PDF_CHARS_PER_PAGE, SHEET_MAX_ROWS, SLIDES_MAX};

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

fn pdf(path: &Path) -> Result<(String, String, bool), String> {
    let pages = pdf_extract::extract_text_by_pages(path)
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
