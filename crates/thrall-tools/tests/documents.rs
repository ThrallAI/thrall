//! Reading real (minimal) PDF, Word, PowerPoint and Excel files, built here so the tests need no
//! fixtures on disk.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::Write;
use std::path::{Path, PathBuf};

use thrall_tools::documents::{Kind, extract};

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("thrall-docs-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

fn zip_file(path: &Path, parts: &[(&str, &str)]) {
    let file = std::fs::File::create(path).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (name, body) in parts {
        zip.start_file(*name, options).unwrap();
        zip.write_all(body.as_bytes()).unwrap();
    }
    zip.finish().unwrap();
}

#[test]
fn word_documents_read_as_paragraphs() {
    let path = temp("letter.docx");
    zip_file(
        &path,
        &[(
            "word/document.xml",
            r#"<?xml version="1.0"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>
<w:p><w:r><w:t>Dear Sam,</w:t></w:r></w:p>
<w:p><w:r><w:t>The invoice is </w:t></w:r><w:r><w:t>attached &amp; due Friday.</w:t></w:r></w:p>
</w:body></w:document>"#,
        )],
    );
    let doc = extract(&path).unwrap();
    assert_eq!(doc.kind, Kind::Word);
    assert!(doc.text.contains("Dear Sam,"), "{}", doc.text);
    assert!(
        doc.text.contains("The invoice is attached & due Friday."),
        "{}",
        doc.text
    );
    assert_eq!(doc.detail, "9 words");
}

#[test]
fn slides_read_in_order() {
    let path = temp("deck.pptx");
    let slide = |text: &str| {
        format!(
            r#"<p:sld xmlns:p="p" xmlns:a="a"><p:cSld><p:spTree><p:sp><p:txBody><a:p><a:r><a:t>{text}</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:cSld></p:sld>"#
        )
    };
    let (one, two) = (slide("Welcome"), slide("Roadmap"));
    zip_file(
        &path,
        &[
            ("ppt/slides/slide1.xml", &one),
            ("ppt/slides/slide2.xml", &two),
        ],
    );
    let doc = extract(&path).unwrap();
    assert_eq!(doc.detail, "2 slides");
    let welcome = doc.text.find("Welcome").unwrap();
    let roadmap = doc.text.find("Roadmap").unwrap();
    assert!(welcome < roadmap);
    assert!(doc.text.contains("--- Slide 2 ---"));
}

#[test]
fn spreadsheets_read_as_rows() {
    let path = temp("budget.xlsx");
    zip_file(
        &path,
        &[
            (
                "[Content_Types].xml",
                r#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/></Types>"#,
            ),
            (
                "_rels/.rels",
                r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#,
            ),
            (
                "xl/workbook.xml",
                r#"<?xml version="1.0" encoding="UTF-8"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Budget" sheetId="1" r:id="rId1"/></sheets></workbook>"#,
            ),
            (
                "xl/_rels/workbook.xml.rels",
                r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#,
            ),
            (
                "xl/worksheets/sheet1.xml",
                r#"<?xml version="1.0" encoding="UTF-8"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>Item</t></is></c><c r="B1" t="inlineStr"><is><t>Cost</t></is></c></row><row r="2"><c r="A2" t="inlineStr"><is><t>Coffee</t></is></c><c r="B2"><v>12.5</v></c></row></sheetData></worksheet>"#,
            ),
        ],
    );
    let doc = extract(&path).unwrap();
    assert_eq!(doc.kind, Kind::Spreadsheet);
    assert_eq!(doc.detail, "1 sheet");
    assert!(doc.text.contains("--- Sheet: Budget ---"), "{}", doc.text);
    assert!(doc.text.contains("Item\tCost"), "{}", doc.text);
    assert!(doc.text.contains("Coffee\t12.5"), "{}", doc.text);
}

/// A one-page PDF with the given text, with a correct cross-reference table.
fn write_pdf(path: &Path, text: &str) {
    let stream = format!("BT /F1 24 Tf 72 720 Td ({text}) Tj ET");
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".to_owned(),
        format!("<< /Length {} >>\nstream\n{stream}\nendstream", stream.len()),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".to_owned(),
    ];
    let mut out = String::from("%PDF-1.4\n");
    let mut offsets = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.push_str(&format!("{} 0 obj\n{body}\nendobj\n", i + 1));
    }
    let xref = out.len();
    out.push_str(&format!(
        "xref\n0 {}\n0000000000 65535 f \n",
        objects.len() + 1
    ));
    for o in offsets {
        out.push_str(&format!("{o:010} 00000 n \n"));
    }
    out.push_str(&format!(
        "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
        objects.len() + 1
    ));
    std::fs::write(path, out).unwrap();
}

#[test]
fn pdfs_read_page_by_page() {
    let path = temp("note.pdf");
    write_pdf(&path, "Quarterly report draft");
    let doc = extract(&path).unwrap();
    assert_eq!(doc.kind, Kind::Pdf);
    // So little text on the page reads as a scanned page.
    assert!(doc.scanned);
    assert!(
        doc.detail.starts_with("1 page, mostly pictures of pages"),
        "{}",
        doc.detail
    );
    assert!(doc.text.contains("--- Page 1 ---"), "{}", doc.text);
    assert!(doc.text.contains("Quarterly report draft"), "{}", doc.text);
}

#[test]
fn broken_files_say_so() {
    let path = temp("fake.docx");
    std::fs::write(&path, "not a zip").unwrap();
    assert!(
        extract(&path)
            .unwrap_err()
            .contains("isn't a valid Office file")
    );
    let plain = temp("plain.txt");
    std::fs::write(&plain, "hi").unwrap();
    assert!(extract(&plain).is_err());
}

#[test]
fn pdfs_with_real_text_are_not_scanned() {
    let path = temp("long.pdf");
    let line = "The quick brown fox jumps over the lazy dog again and again ".repeat(5);
    write_pdf(&path, &line);
    let doc = extract(&path).unwrap();
    assert!(!doc.scanned, "{}", doc.detail);
    assert_eq!(doc.detail, "1 page");
}
