//! Editing real Excel, Word and PDF files, then reading them back.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use thrall_tools::documents::extract;
use thrall_tools::office::sheets::{CellChange, CellValue};
use thrall_tools::office::word::Replacement;
use thrall_tools::office::{pdf, sheets, word};

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("thrall-office-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    let _ = std::fs::remove_file(&path);
    path
}

fn cell(address: &str, value: CellValue) -> CellChange {
    let (sheet, cell) = sheets::parse_address(address).unwrap();
    CellChange { sheet, cell, value }
}

#[test]
fn spreadsheets_are_created_and_edited_keeping_formatting() {
    let path = temp("budget.xlsx");
    sheets::apply(
        &path,
        &[
            cell("A1", CellValue::Text("Item".into())),
            cell("B1", CellValue::Text("Cost".into())),
            cell("A2", CellValue::Text("Coffee".into())),
            cell("B2", CellValue::Number(4.5)),
            cell("B3", CellValue::Formula("SUM(B2:B2)".into())),
            cell("Notes!A1", CellValue::Text("Second sheet".into())),
        ],
    )
    .unwrap();
    let text = extract(&path).unwrap().text;
    assert!(text.contains("Item\tCost"), "{text}");
    assert!(text.contains("Coffee\t4.5"), "{text}");
    assert!(text.contains("--- Sheet: Notes ---"), "{text}");

    // Make A1 bold, then change another cell: the bold stays.
    let mut book = umya_spreadsheet::reader::xlsx::read(&path).unwrap();
    book.sheet_by_name_mut("Sheet1")
        .unwrap()
        .style_mut("A1")
        .font_mut()
        .set_bold(true);
    umya_spreadsheet::writer::xlsx::write(&book, &path).unwrap();

    let changes = [cell("B2", CellValue::Number(5.0))];
    let (old, new) = sheets::preview(&path, &changes).unwrap();
    assert_eq!(old, "Sheet1!B2: 4.5\n");
    assert_eq!(new, "Sheet1!B2: 5\n");
    sheets::apply(&path, &changes).unwrap();

    let book = umya_spreadsheet::reader::xlsx::read(&path).unwrap();
    let ws = book.sheet_by_name("Sheet1").unwrap();
    assert_eq!(ws.value("B2"), "5");
    assert_eq!(ws.cell("B3").unwrap().formula(), "SUM(B2:B2)");
    assert!(ws.style("A1").font().unwrap().bold());
}

#[test]
fn word_documents_are_made_and_text_replaced() {
    let path = temp("letter.docx");
    word::create(
        &path,
        "# Invoice 42\n\nDear Sam,\n\n- Coffee beans\n- **Urgent:** pay by Friday\n",
        &thrall_tools::office::layout::Layout::default(),
    )
    .unwrap();
    let text = extract(&path).unwrap().text;
    assert!(text.contains("Invoice 42"), "{text}");
    assert!(text.contains("Dear Sam,"), "{text}");
    assert!(text.contains("Urgent: pay by Friday"), "{text}");

    let replacements = [
        Replacement {
            find: "Sam".into(),
            replace: "Alex".into(),
        },
        // Spread over two runs (bold "Urgent:" and the rest).
        Replacement {
            find: "Urgent: pay".into(),
            replace: "Please pay".into(),
        },
    ];
    let (old, new, count) = word::preview(&path, &replacements).unwrap();
    assert_eq!(count, 2);
    assert!(
        old.contains("Dear Sam,") && new.contains("Dear Alex,"),
        "{old} / {new}"
    );
    assert_eq!(word::replace(&path, &replacements).unwrap(), 2);
    let text = extract(&path).unwrap().text;
    assert!(text.contains("Dear Alex,"), "{text}");
    assert!(text.contains("Please pay by Friday"), "{text}");
    assert!(!text.contains("Sam"), "{text}");

    let missing = word::replace(
        &path,
        &[Replacement {
            find: "nowhere".into(),
            replace: "x".into(),
        }],
    );
    assert!(missing.unwrap_err().contains("none of the text was found"));
}

/// A PDF with one page per text.
fn write_pdf(path: &Path, pages: &[&str]) {
    let mut objects: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        String::new(), // the page tree, filled in below
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>"
            .to_owned(),
    ];
    let mut kids = Vec::new();
    for text in pages {
        let stream = format!("BT /F1 24 Tf 72 720 Td ({text}) Tj ET");
        let content_id = objects.len() + 2;
        let page_id = objects.len() + 1;
        objects.push(format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents {content_id} 0 R /Resources << /Font << /F1 3 0 R >> >> >>"
        ));
        objects.push(format!(
            "<< /Length {} >>\nstream\n{stream}\nendstream",
            stream.len()
        ));
        kids.push(format!("{page_id} 0 R"));
    }
    objects[1] = format!(
        "<< /Type /Pages /Kids [{}] /Count {} >>",
        kids.join(" "),
        pages.len()
    );
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

fn rotation(path: &Path, page: u32) -> i64 {
    let doc = lopdf::Document::load(path).unwrap();
    let id = doc.get_pages()[&page];
    doc.get_object(id)
        .unwrap()
        .as_dict()
        .unwrap()
        .get(b"Rotate")
        .and_then(lopdf::Object::as_i64)
        .unwrap_or(0)
}

#[test]
fn pdfs_are_merged_split_rotated_and_stamped() {
    let a = temp("a.pdf");
    let b = temp("b.pdf");
    write_pdf(&a, &["Alpha one", "Alpha two"]);
    write_pdf(&b, &["Bravo one"]);

    let merged = temp("merged.pdf");
    assert_eq!(pdf::merge(&[&a, &b], &merged).unwrap(), 3);
    let text = extract(&merged).unwrap().text;
    let (one, two, three) = (
        text.find("Alpha one").unwrap(),
        text.find("Alpha two").unwrap(),
        text.find("Bravo one").unwrap(),
    );
    assert!(one < two && two < three, "pages keep their order: {text}");

    let picked = temp("picked.pdf");
    assert_eq!(
        pdf::select_pages(&merged, "2-3", false, &picked).unwrap(),
        2
    );
    let text = extract(&picked).unwrap().text;
    assert!(
        !text.contains("Alpha one") && text.contains("Bravo one"),
        "{text}"
    );

    let dropped = temp("dropped.pdf");
    assert_eq!(pdf::select_pages(&merged, "1", true, &dropped).unwrap(), 2);
    assert!(pdf::select_pages(&merged, "1-3", true, &dropped).is_err());

    let turned = temp("turned.pdf");
    assert_eq!(pdf::rotate(&merged, Some("2"), 90, &turned).unwrap(), 1);
    assert_eq!(rotation(&turned, 1), 0);
    assert_eq!(rotation(&turned, 2), 90);
    assert!(pdf::rotate(&merged, None, 45, &turned).is_err());

    let stamped = temp("stamped.pdf");
    assert_eq!(
        pdf::stamp(&a, "CONFIDENTIAL (draft)", None, false, &stamped).unwrap(),
        2
    );
    let text = extract(&stamped).unwrap().text;
    assert!(text.contains("CONFIDENTIAL (draft)"), "{text}");
    assert!(
        text.contains("Alpha one"),
        "the page's own text stays: {text}"
    );
}

/// A one-page PDF with a text field `name` and a checkbox `agree`.
fn write_form(path: &Path) {
    use lopdf::{Dictionary, Document, Object, Stream};
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let content = doc.add_object(Stream::new(Dictionary::new(), b"BT ET".to_vec()));
    let mut name = Dictionary::new();
    name.set("FT", Object::Name(b"Tx".to_vec()));
    name.set("T", Object::string_literal("name"));
    name.set("Type", Object::Name(b"Annot".to_vec()));
    name.set("Subtype", Object::Name(b"Widget".to_vec()));
    name.set("Rect", vec![50.into(), 700.into(), 300.into(), 720.into()]);
    let name_id = doc.add_object(name);
    let mut agree = Dictionary::new();
    agree.set("FT", Object::Name(b"Btn".to_vec()));
    agree.set("T", Object::string_literal("agree"));
    agree.set("V", Object::Name(b"Off".to_vec()));
    agree.set("Type", Object::Name(b"Annot".to_vec()));
    agree.set("Subtype", Object::Name(b"Widget".to_vec()));
    agree.set("Rect", vec![50.into(), 650.into(), 70.into(), 670.into()]);
    let agree_id = doc.add_object(agree);
    let mut page = Dictionary::new();
    page.set("Type", Object::Name(b"Page".to_vec()));
    page.set("Parent", pages_id);
    page.set("MediaBox", vec![0.into(), 0.into(), 612.into(), 792.into()]);
    page.set("Contents", content);
    page.set("Annots", vec![name_id.into(), agree_id.into()]);
    let page_id = doc.add_object(page);
    let mut pages = Dictionary::new();
    pages.set("Type", Object::Name(b"Pages".to_vec()));
    pages.set("Kids", vec![page_id.into()]);
    pages.set("Count", 1);
    doc.objects.insert(pages_id, Object::Dictionary(pages));
    let mut form = Dictionary::new();
    form.set("Fields", vec![name_id.into(), agree_id.into()]);
    let mut catalog = Dictionary::new();
    catalog.set("Type", Object::Name(b"Catalog".to_vec()));
    catalog.set("Pages", pages_id);
    catalog.set("AcroForm", form);
    let catalog_id = doc.add_object(catalog);
    doc.trailer.set("Root", catalog_id);
    doc.save(path).unwrap();
}

#[test]
fn pdf_forms_are_listed_and_filled() {
    let form = temp("form.pdf");
    write_form(&form);
    let fields = pdf::form_fields(&form).unwrap();
    assert_eq!(fields.len(), 2);
    assert_eq!(fields[0].name, "name");
    assert_eq!(fields[0].kind, "text");
    assert_eq!(fields[1].kind, "checkbox");

    let filled = temp("filled.pdf");
    let values = BTreeMap::from([
        ("name".to_owned(), "Owner".to_owned()),
        ("agree".to_owned(), "yes".to_owned()),
    ]);
    assert_eq!(pdf::fill_form(&form, &values, &filled).unwrap(), 2);
    let fields = pdf::form_fields(&filled).unwrap();
    assert_eq!(fields[0].value, "Owner");
    assert_eq!(fields[1].value, "Yes");

    let wrong = BTreeMap::from([("nope".to_owned(), "x".to_owned())]);
    let err = pdf::fill_form(&form, &wrong, &filled).unwrap_err();
    assert!(
        err.contains("no field called nope") && err.contains("name, agree"),
        "{err}"
    );
}

/// Like `write_pdf`, but laid out like many real PDFs: no resources on the pages (they're
/// inherited from the page tree), and the font list behind a reference.
fn write_shared_pdf(path: &Path, text: &str) {
    let stream = format!("BT /F1 24 Tf 72 720 Td ({text}) Tj ET");
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /Resources 5 0 R >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R >>".to_owned(),
        format!(
            "<< /Length {} >>\nstream\n{stream}\nendstream",
            stream.len()
        ),
        "<< /Font 6 0 R >>".to_owned(),
        "<< /F1 7 0 R >>".to_owned(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>"
            .to_owned(),
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
fn stamps_work_with_shared_and_inherited_fonts() {
    let src = temp("shared.pdf");
    write_shared_pdf(&src, "Inherited font text");
    let out = temp("shared-stamped.pdf");
    pdf::stamp(&src, "Homework - week 3", None, true, &out).unwrap();
    let text = extract(&out).unwrap().text;
    assert!(text.contains("Homework - week 3"), "{text}");
    assert!(
        text.contains("Inherited font text"),
        "the page keeps its own font: {text}"
    );
}
