//! Changing cells in an Excel workbook (.xlsx), keeping its formatting.

use std::path::Path;

use serde_json::Value;

/// One cell to set: `Sheet1!B2` style address (sheet optional) and its new content.
#[derive(Debug, Clone, PartialEq)]
pub struct CellChange {
    /// Sheet name; `None` = the first sheet.
    pub sheet: Option<String>,
    /// Cell, like `B2`.
    pub cell: String,
    /// New content.
    pub value: CellValue,
}

/// What goes in a cell.
#[derive(Debug, Clone, PartialEq)]
pub enum CellValue {
    /// Text.
    Text(String),
    /// A number.
    Number(f64),
    /// True or false.
    Bool(bool),
    /// A formula, without the leading `=`.
    Formula(String),
    /// Empty the cell.
    Clear,
}

impl CellValue {
    /// From a JSON value: strings starting with `=` are formulas, `null` clears.
    pub fn from_json(v: &Value) -> Self {
        match v {
            Value::Null => Self::Clear,
            Value::Bool(b) => Self::Bool(*b),
            Value::Number(n) => n.as_f64().map_or(Self::Clear, Self::Number),
            Value::String(s) => match s.strip_prefix('=') {
                Some(f) => Self::Formula(f.to_owned()),
                None => Self::Text(s.clone()),
            },
            other => Self::Text(other.to_string()),
        }
    }

    /// As shown in a preview.
    pub fn shown(&self) -> String {
        match self {
            Self::Text(s) => s.clone(),
            Self::Number(n) => n.to_string(),
            Self::Bool(b) => b.to_string().to_uppercase(),
            Self::Formula(f) => format!("={f}"),
            Self::Clear => "(empty)".to_owned(),
        }
    }
}

/// Is `cell` a plain address like `B2` or `AA10`?
pub fn valid_cell(cell: &str) -> bool {
    let letters = cell.chars().take_while(char::is_ascii_alphabetic).count();
    let digits = &cell[letters..];
    (1..=3).contains(&letters)
        && !digits.is_empty()
        && digits.chars().all(|c| c.is_ascii_digit())
        && !digits.starts_with('0')
}

/// Split `Sheet1!B2` (or `'My sheet'!B2`, or `B2`) into sheet and cell.
pub fn parse_address(address: &str) -> Option<(Option<String>, String)> {
    let (sheet, cell) = match address.rsplit_once('!') {
        Some((s, c)) => (Some(s.trim().trim_matches('\'').to_owned()), c),
        None => (None, address),
    };
    let cell = cell.trim().to_uppercase().replace('$', "");
    valid_cell(&cell).then_some((sheet.filter(|s| !s.is_empty()), cell))
}

fn open(path: &Path) -> Result<umya_spreadsheet::Workbook, String> {
    if path.exists() {
        umya_spreadsheet::reader::xlsx::read(path)
            .map_err(|e| format!("couldn't open the workbook: {e}"))
    } else {
        Ok(umya_spreadsheet::new_file())
    }
}

fn sheet_name(book: &umya_spreadsheet::Workbook, wanted: Option<&str>) -> String {
    match wanted {
        Some(name) => name.to_owned(),
        None => book
            .sheet(0)
            .map(|s| s.name().to_owned())
            .unwrap_or_else(|_| "Sheet1".to_owned()),
    }
}

/// What a cell holds now, as shown in a preview (formulas with their `=`).
fn current(book: &umya_spreadsheet::Workbook, sheet: &str, cell: &str) -> String {
    let Ok(ws) = book.sheet_by_name(sheet) else {
        return "(empty)".to_owned();
    };
    match ws.cell(cell) {
        Some(c) if !c.formula().is_empty() => format!("={}", c.formula()),
        Some(c) if !c.value().is_empty() => c.value().into_owned(),
        _ => "(empty)".to_owned(),
    }
}

/// The preview: each changed cell's old and new content, as two texts for a diff.
pub fn preview(path: &Path, changes: &[CellChange]) -> Result<(String, String), String> {
    let book = open(path)?;
    let mut old = String::new();
    let mut new = String::new();
    for c in changes {
        let sheet = sheet_name(&book, c.sheet.as_deref());
        old.push_str(&format!(
            "{sheet}!{}: {}\n",
            c.cell,
            current(&book, &sheet, &c.cell)
        ));
        new.push_str(&format!("{sheet}!{}: {}\n", c.cell, c.value.shown()));
    }
    Ok((old, new))
}

/// Apply the changes and save (creating the workbook and any missing sheets). Returns how many
/// cells changed.
pub fn apply(path: &Path, changes: &[CellChange]) -> Result<usize, String> {
    let mut book = open(path)?;
    for c in changes {
        let sheet = sheet_name(&book, c.sheet.as_deref());
        if book.sheet_by_name(&sheet).is_err() {
            book.new_sheet(sheet.clone())
                .map_err(|e| format!("couldn't add sheet {sheet}: {e}"))?;
        }
        let ws = book
            .sheet_by_name_mut(&sheet)
            .map_err(|e| format!("no sheet {sheet}: {e}"))?;
        let cell = ws.cell_mut(c.cell.as_str());
        match &c.value {
            CellValue::Text(s) => {
                cell.set_value_string(s.clone());
            }
            CellValue::Number(n) => {
                cell.set_value_number(*n);
            }
            CellValue::Bool(b) => {
                cell.set_value_bool(*b);
            }
            CellValue::Formula(f) => {
                cell.set_formula(f.clone());
            }
            CellValue::Clear => {
                cell.set_value_string(String::new());
            }
        }
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("couldn't make the folder: {e}"))?;
    }
    umya_spreadsheet::writer::xlsx::write(&book, path)
        .map_err(|e| format!("couldn't save the workbook: {e}"))?;
    Ok(changes.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_parse() {
        assert_eq!(parse_address("b2"), Some((None, "B2".into())));
        assert_eq!(
            parse_address("'Q1 Sales'!$C$10"),
            Some((Some("Q1 Sales".into()), "C10".into()))
        );
        assert_eq!(parse_address("B0"), None);
        assert_eq!(parse_address("Sheet1!"), None);
        assert_eq!(parse_address("12"), None);
    }

    #[test]
    fn json_values_map_to_cells() {
        assert_eq!(
            CellValue::from_json(&serde_json::json!("=SUM(A1:A3)")),
            CellValue::Formula("SUM(A1:A3)".into())
        );
        assert_eq!(
            CellValue::from_json(&serde_json::json!(12.5)),
            CellValue::Number(12.5)
        );
        assert_eq!(
            CellValue::from_json(&serde_json::json!(null)),
            CellValue::Clear
        );
        assert_eq!(
            CellValue::from_json(&serde_json::json!("hi")),
            CellValue::Text("hi".into())
        );
    }
}
