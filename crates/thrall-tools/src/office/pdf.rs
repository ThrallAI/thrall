//! PDF changes that keep the pages as they are: merge, pick or remove pages, rotate, stamp a line
//! of text, and fill in forms. (Rewriting text in place isn't possible: a PDF stores positioned
//! glyphs, not flowing text.)

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use lopdf::{Dictionary, Document, Object, ObjectId};

use crate::constants::{STAMP_BOTTOM_MARGIN, STAMP_TOP_MARGIN};

fn load(path: &Path) -> Result<Document, String> {
    Document::load(path).map_err(|e| format!("couldn't open the PDF: {e}"))
}

fn save(mut doc: Document, path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("couldn't make the folder: {e}"))?;
    }
    doc.compress();
    doc.save(path)
        .map(|_| ())
        .map_err(|e| format!("couldn't save the PDF: {e}"))
}

/// How many pages.
pub fn page_count(path: &Path) -> Result<usize, String> {
    Ok(load(path)?.get_pages().len())
}

/// Pages from a spec like `1-3, 5, 8-` (1-based, `8-` = to the end), checked against `total`.
pub fn parse_pages(spec: &str, total: usize) -> Result<BTreeSet<u32>, String> {
    let mut pages = BTreeSet::new();
    for part in spec.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        let number = |s: &str| -> Result<u32, String> {
            s.trim()
                .parse::<u32>()
                .map_err(|_| format!("`{part}` isn't a page or range like 2-5"))
        };
        let (from, to) = match part.split_once('-') {
            Some((a, "")) => (number(a)?, u32::try_from(total).unwrap_or(u32::MAX)),
            Some((a, b)) => (number(a)?, number(b)?),
            None => {
                let n = number(part)?;
                (n, n)
            }
        };
        if from == 0 || from > to || to as usize > total {
            return Err(format!("pages {part} aren't in this {total}-page PDF"));
        }
        pages.extend(from..=to);
    }
    if pages.is_empty() {
        return Err("say which pages, like 1-3, 5".to_owned());
    }
    Ok(pages)
}

/// Keep only `pages` (or, with `remove`, drop them).
pub fn select_pages(
    input: &Path,
    spec: &str,
    remove: bool,
    output: &Path,
) -> Result<usize, String> {
    let mut doc = load(input)?;
    let total = doc.get_pages().len();
    let chosen = parse_pages(spec, total)?;
    let drop: Vec<u32> = (1..=u32::try_from(total).unwrap_or(0))
        .filter(|n| chosen.contains(n) == remove)
        .collect();
    if drop.len() == total {
        return Err("that would leave no pages".to_owned());
    }
    doc.delete_pages(&drop);
    doc.prune_objects();
    let kept = total - drop.len();
    save(doc, output)?;
    Ok(kept)
}

/// Rotate `pages` (all when `None`) by a multiple of 90 degrees.
pub fn rotate(
    input: &Path,
    spec: Option<&str>,
    degrees: i64,
    output: &Path,
) -> Result<usize, String> {
    if degrees % 90 != 0 {
        return Err("rotate by 90, 180 or 270 degrees".to_owned());
    }
    let mut doc = load(input)?;
    let pages = doc.get_pages();
    let chosen = match spec {
        Some(s) => parse_pages(s, pages.len())?,
        None => pages.keys().copied().collect(),
    };
    for (number, id) in &pages {
        if !chosen.contains(number) {
            continue;
        }
        let dict = doc
            .get_object_mut(*id)
            .and_then(Object::as_dict_mut)
            .map_err(|e| format!("page {number}: {e}"))?;
        let now = dict.get(b"Rotate").and_then(Object::as_i64).unwrap_or(0);
        dict.set("Rotate", (now + degrees).rem_euclid(360));
    }
    save(doc, output)?;
    Ok(chosen.len())
}

/// Join PDFs, in order, into one.
pub fn merge(inputs: &[&Path], output: &Path) -> Result<usize, String> {
    if inputs.len() < 2 {
        return Err("give at least two PDFs to merge".to_owned());
    }
    let mut max_id = 1;
    // Pages in reading order, then every other object.
    let mut pages: Vec<(ObjectId, Object)> = Vec::new();
    let mut objects: BTreeMap<ObjectId, Object> = BTreeMap::new();
    for path in inputs {
        let mut doc = load(path)?;
        doc.renumber_objects_with(max_id);
        max_id = doc.max_id + 1;
        for (_, id) in doc.get_pages() {
            if let Ok(page) = doc.get_object(id) {
                pages.push((id, page.clone()));
            }
        }
        objects.extend(doc.objects);
    }
    let mut merged = Document::with_version("1.5");
    let mut catalog: Option<(ObjectId, Dictionary)> = None;
    let mut tree: Option<(ObjectId, Dictionary)> = None;
    for (id, object) in &objects {
        match object.type_name().unwrap_or(b"") {
            b"Catalog" => {
                if catalog.is_none()
                    && let Ok(d) = object.as_dict()
                {
                    catalog = Some((*id, d.clone()));
                }
            }
            b"Pages" => {
                if tree.is_none()
                    && let Ok(d) = object.as_dict()
                {
                    tree = Some((*id, d.clone()));
                }
            }
            b"Page" | b"Outlines" | b"Outline" => {}
            _ => {
                merged.objects.insert(*id, object.clone());
            }
        }
    }
    let (Some((catalog_id, mut catalog)), Some((tree_id, mut tree))) = (catalog, tree) else {
        return Err("one of the PDFs has no page tree".to_owned());
    };
    let count = pages.len();
    let mut kids = Vec::with_capacity(count);
    for (id, page) in pages {
        if let Object::Dictionary(mut d) = page {
            d.set("Parent", tree_id);
            merged.objects.insert(id, Object::Dictionary(d));
            kids.push(Object::Reference(id));
        }
    }
    tree.set("Count", i64::try_from(count).unwrap_or(0));
    tree.set("Kids", kids);
    merged.objects.insert(tree_id, Object::Dictionary(tree));
    catalog.set("Pages", tree_id);
    catalog.remove(b"Outlines");
    merged
        .objects
        .insert(catalog_id, Object::Dictionary(catalog));
    merged.trailer.set("Root", catalog_id);
    merged.max_id = u32::try_from(merged.objects.len()).unwrap_or(u32::MAX);
    merged.renumber_objects();
    merged.adjust_zero_pages();
    save(merged, output)?;
    Ok(count)
}

/// A PDF string: Latin-1 only (the standard font's encoding), with `\`, `(` and `)` escaped.
fn pdf_text(text: &str) -> Vec<u8> {
    let mut out = Vec::new();
    for c in text.chars() {
        let byte = u8::try_from(u32::from(c)).unwrap_or(b'?');
        if matches!(byte, b'\\' | b'(' | b')') {
            out.push(b'\\');
        }
        out.push(if byte < 0x20 { b' ' } else { byte });
    }
    out
}

/// The page's resources dictionary (or a parent's, which pages inherit), resolved.
fn inherited_resources(doc: &Document, page: ObjectId) -> Option<Dictionary> {
    let mut at = page;
    for _ in 0..32 {
        let dict = doc.get_object(at).and_then(Object::as_dict).ok()?;
        match dict.get(b"Resources") {
            Ok(Object::Reference(r)) => {
                return doc.get_object(*r).and_then(Object::as_dict).ok().cloned();
            }
            Ok(Object::Dictionary(d)) => return Some(d.clone()),
            _ => at = dict.get(b"Parent").and_then(Object::as_reference).ok()?,
        }
    }
    None
}

/// Make `font` available on `page` as /ThrallStamp, whether its resources are its own, shared
/// (a reference) or inherited from the page tree, and whether its fonts are listed directly or
/// through a reference.
fn add_font(doc: &mut Document, page: ObjectId, font: ObjectId) -> Result<(), String> {
    // Resources that live on a parent: give the page its own copy first, so adding a font
    // doesn't hide the inherited ones.
    let own = doc
        .get_object(page)
        .and_then(Object::as_dict)
        .map_err(|e| e.to_string())?
        .get(b"Resources")
        .ok()
        .cloned();
    let resources_id = match own {
        Some(Object::Reference(r)) => Some(r),
        Some(Object::Dictionary(_)) => None,
        _ => {
            let copy = inherited_resources(doc, page).unwrap_or_default();
            doc.get_object_mut(page)
                .and_then(Object::as_dict_mut)
                .map_err(|e| e.to_string())?
                .set("Resources", copy);
            None
        }
    };
    let resources = match resources_id {
        Some(r) => doc.get_object_mut(r),
        None => doc
            .get_object_mut(page)
            .and_then(Object::as_dict_mut)
            .and_then(|d| d.get_mut(b"Resources")),
    }
    .and_then(Object::as_dict_mut)
    .map_err(|e| e.to_string())?;
    let fonts_ref = match resources.get(b"Font") {
        Ok(Object::Reference(r)) => Some(*r),
        Ok(Object::Dictionary(_)) => None,
        _ => {
            resources.set("Font", Dictionary::new());
            None
        }
    };
    let fonts = match fonts_ref {
        Some(r) => doc.get_object_mut(r).and_then(Object::as_dict_mut),
        None => resources.get_mut(b"Font").and_then(Object::as_dict_mut),
    }
    .map_err(|e| e.to_string())?;
    fonts.set("ThrallStamp", font);
    Ok(())
}

/// Write a line of text on `pages` (all when `None`), at the bottom or top left.
pub fn stamp(
    input: &Path,
    text: &str,
    spec: Option<&str>,
    top: bool,
    output: &Path,
) -> Result<usize, String> {
    let mut doc = load(input)?;
    let pages = doc.get_pages();
    let chosen = match spec {
        Some(s) => parse_pages(s, pages.len())?,
        None => pages.keys().copied().collect(),
    };
    let mut font = Dictionary::new();
    font.set("Type", Object::Name(b"Font".to_vec()));
    font.set("Subtype", Object::Name(b"Type1".to_vec()));
    font.set("BaseFont", Object::Name(b"Helvetica".to_vec()));
    font.set("Encoding", Object::Name(b"WinAnsiEncoding".to_vec()));
    let font = doc.add_object(font);
    for (number, id) in &pages {
        if !chosen.contains(number) {
            continue;
        }
        // Where the top is: the page's own box, else US Letter.
        let height = doc
            .get_object(*id)
            .and_then(Object::as_dict)
            .and_then(|d| d.get(b"MediaBox"))
            .and_then(Object::as_array)
            .ok()
            .and_then(|b| b.get(3).and_then(|v| v.as_float().ok()))
            .unwrap_or(792.0);
        // In the outer margin, clear of the page's own header or footer.
        let y = if top {
            height - STAMP_TOP_MARGIN
        } else {
            STAMP_BOTTOM_MARGIN
        };
        add_font(&mut doc, *id, font).map_err(|e| format!("page {number}: {e}"))?;
        let old = doc
            .get_page_content(*id)
            .map_err(|e| format!("page {number}: {e}"))?;
        // The page's own drawing in its own graphics state, then the stamp on top.
        let mut content = b"q\n".to_vec();
        content.extend_from_slice(&old);
        content.extend_from_slice(format!("\nQ\nBT /ThrallStamp 10 Tf 36 {y:.1} Td (").as_bytes());
        content.extend_from_slice(&pdf_text(text));
        content.extend_from_slice(b") Tj ET\n");
        doc.change_page_content(*id, content)
            .map_err(|e| format!("page {number}: {e}"))?;
    }
    save(doc, output)?;
    Ok(chosen.len())
}

/// A form field: its full name, type and current value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    /// Full name (`parent.child`).
    pub name: String,
    /// `text`, `checkbox`, `choice`, `signature` or `other`.
    pub kind: String,
    /// Current value ("" when empty).
    pub value: String,
}

fn object_text(doc: &Document, obj: &Object) -> String {
    let obj = match obj {
        Object::Reference(id) => doc.get_object(*id).unwrap_or(obj),
        o => o,
    };
    match obj {
        Object::String(bytes, _) => {
            if bytes.starts_with(&[0xFE, 0xFF]) {
                let units: Vec<u16> = bytes[2..]
                    .chunks(2)
                    .map(|c| u16::from_be_bytes([c[0], *c.get(1).unwrap_or(&0)]))
                    .collect();
                String::from_utf16_lossy(&units)
            } else {
                bytes.iter().map(|&b| char::from(b)).collect()
            }
        }
        Object::Name(name) => String::from_utf8_lossy(name).into_owned(),
        Object::Integer(i) => i.to_string(),
        Object::Real(r) => r.to_string(),
        _ => String::new(),
    }
}

/// Every form field (id, name, kind, value), walking parents and kids.
fn walk_fields(doc: &Document) -> Vec<(ObjectId, Field)> {
    let mut out = Vec::new();
    let Ok(catalog) = doc.catalog() else {
        return out;
    };
    let form = match catalog.get(b"AcroForm") {
        Ok(Object::Reference(id)) => doc.get_object(*id).and_then(Object::as_dict).ok(),
        Ok(Object::Dictionary(d)) => Some(d),
        _ => None,
    };
    let Some(fields) = form.and_then(|f| f.get(b"Fields").and_then(Object::as_array).ok()) else {
        return out;
    };
    let mut stack: Vec<(ObjectId, String, Option<String>)> = fields
        .iter()
        .filter_map(|f| f.as_reference().ok())
        .map(|id| (id, String::new(), None))
        .rev()
        .collect();
    while let Some((id, parent, parent_kind)) = stack.pop() {
        let Ok(dict) = doc.get_object(id).and_then(Object::as_dict) else {
            continue;
        };
        let own = dict
            .get(b"T")
            .map(|t| object_text(doc, t))
            .unwrap_or_default();
        let name = match (parent.is_empty(), own.is_empty()) {
            (true, _) => own.clone(),
            (false, true) => parent.clone(),
            (false, false) => format!("{parent}.{own}"),
        };
        let kind = dict
            .get(b"FT")
            .ok()
            .map(|t| object_text(doc, t))
            .or(parent_kind);
        let kids: Vec<ObjectId> = dict
            .get(b"Kids")
            .and_then(Object::as_array)
            .map(|k| k.iter().filter_map(|o| o.as_reference().ok()).collect())
            .unwrap_or_default();
        // A field with named kids is a group; widgets (kids without names) belong to it.
        let named_kids = kids.iter().any(|k| {
            doc.get_object(*k)
                .and_then(Object::as_dict)
                .is_ok_and(|d| d.has(b"T"))
        });
        if named_kids {
            for k in kids.into_iter().rev() {
                stack.push((k, name.clone(), kind.clone()));
            }
            continue;
        }
        if own.is_empty() {
            continue;
        }
        let kind = match kind.as_deref() {
            Some("Tx") => "text",
            Some("Btn") => "checkbox",
            Some("Ch") => "choice",
            Some("Sig") => "signature",
            _ => "other",
        };
        let value = dict
            .get(b"V")
            .map(|v| object_text(doc, v))
            .unwrap_or_default();
        out.push((
            id,
            Field {
                name,
                kind: kind.to_owned(),
                value,
            },
        ));
    }
    out
}

/// The PDF's form fields.
pub fn form_fields(path: &Path) -> Result<Vec<Field>, String> {
    Ok(walk_fields(&load(path)?)
        .into_iter()
        .map(|(_, f)| f)
        .collect())
}

/// Fill form fields by name. Checkboxes take yes/no (true/false); viewers redraw the fields.
pub fn fill_form(
    input: &Path,
    values: &BTreeMap<String, String>,
    output: &Path,
) -> Result<usize, String> {
    let mut doc = load(input)?;
    let fields = walk_fields(&doc);
    if fields.is_empty() {
        return Err("this PDF has no form fields".to_owned());
    }
    let unknown: Vec<&str> = values
        .keys()
        .filter(|k| !fields.iter().any(|(_, f)| &f.name == *k))
        .map(String::as_str)
        .collect();
    if !unknown.is_empty() {
        let names: Vec<&str> = fields.iter().map(|(_, f)| f.name.as_str()).collect();
        return Err(format!(
            "no field called {}. The fields are: {}",
            unknown.join(", "),
            names.join(", ")
        ));
    }
    let mut filled = 0;
    for (id, field) in &fields {
        let Some(value) = values.get(&field.name) else {
            continue;
        };
        let Ok(dict) = doc.get_object_mut(*id).and_then(Object::as_dict_mut) else {
            continue;
        };
        if field.kind == "checkbox" {
            let on = matches!(
                value.to_lowercase().as_str(),
                "yes" | "true" | "on" | "1" | "x"
            );
            let state = if on { "Yes" } else { "Off" };
            dict.set("V", Object::Name(state.as_bytes().to_vec()));
            dict.set("AS", Object::Name(state.as_bytes().to_vec()));
        } else {
            dict.set("V", Object::string_literal(value.as_str()));
            // The old drawing would show the old value; viewers draw the new one.
            dict.remove(b"AP");
        }
        filled += 1;
    }
    // Ask viewers to draw the fields from their values.
    let form_id = doc
        .catalog()
        .ok()
        .and_then(|c| c.get(b"AcroForm").ok())
        .and_then(|f| f.as_reference().ok());
    let form = match form_id {
        Some(id) => doc.get_object_mut(id).and_then(Object::as_dict_mut).ok(),
        None => doc
            .catalog_mut()
            .ok()
            .and_then(|c| c.get_mut(b"AcroForm").ok())
            .and_then(|f| f.as_dict_mut().ok()),
    };
    if let Some(form) = form {
        form.set("NeedAppearances", true);
    }
    save(doc, output)?;
    Ok(filled)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_specs_parse() {
        assert_eq!(
            parse_pages("1-3, 5", 6).unwrap(),
            BTreeSet::from([1, 2, 3, 5])
        );
        assert_eq!(parse_pages("4-", 6).unwrap(), BTreeSet::from([4, 5, 6]));
        assert!(parse_pages("0", 6).is_err());
        assert!(parse_pages("5-9", 6).is_err());
        assert!(parse_pages("a", 6).is_err());
        assert!(parse_pages("", 6).is_err());
    }

    #[test]
    fn stamp_text_is_escaped() {
        assert_eq!(pdf_text("a (b) \\ é"), b"a \\(b\\) \\\\ \xe9".to_vec());
        assert_eq!(pdf_text("→"), b"?".to_vec());
    }
}
