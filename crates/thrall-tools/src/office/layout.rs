//! Page and type settings shared by the PDF and Word writers: text sizes (px or pt), spacing,
//! the running header and footer, paper, orientation and margins. Defaults follow the web app's
//! chat: headings only a little larger than the text, roomy but not airy lines.

use serde_json::Value;

use crate::constants::{
    LAYOUT_BODY_PT, LAYOUT_HEADING_PT, LAYOUT_MARGIN_MM, LAYOUT_MAX_PT, LAYOUT_MIN_PT,
    LAYOUT_SUBHEADING_PT, LAYOUT_TITLE_OVER_HEADING, LAYOUT_TITLE_PT,
};

/// Paper size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Paper {
    /// 210 × 297 mm.
    A4,
    /// 8.5 × 11 in.
    Letter,
}

impl Paper {
    /// Width and height in millimetres (portrait).
    pub fn mm(self) -> (f64, f64) {
        match self {
            Self::A4 => (210.0, 297.0),
            Self::Letter => (215.9, 279.4),
        }
    }
}

/// How a document is laid out. Sizes are in points.
#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    /// The first `#` heading.
    pub title_pt: f64,
    /// `##` headings.
    pub heading_pt: f64,
    /// `###` and smaller headings.
    pub subheading_pt: f64,
    /// Body text; lists, tables and code scale from it.
    pub body_pt: f64,
    /// Tighter lines and gaps (forms, dense notes).
    pub compact: bool,
    /// The title at the top of each page and "Page X of Y" at the bottom.
    pub header_footer: bool,
    /// Paper size.
    pub paper: Paper,
    /// Landscape instead of portrait.
    pub landscape: bool,
    /// Margin on every side, in millimetres.
    pub margin_mm: f64,
}

impl Default for Layout {
    fn default() -> Self {
        Self {
            title_pt: LAYOUT_TITLE_PT,
            heading_pt: LAYOUT_HEADING_PT,
            subheading_pt: LAYOUT_SUBHEADING_PT,
            body_pt: LAYOUT_BODY_PT,
            compact: false,
            header_footer: true,
            paper: Paper::A4,
            landscape: false,
            margin_mm: LAYOUT_MARGIN_MM,
        }
    }
}

/// A size like `"12px"` or `"9pt"`, in points. The unit is required: people mean px or pt
/// depending on where they're coming from (screens or Word), and guessing gets it a third wrong.
pub fn parse_size(value: &Value) -> Result<f64, String> {
    let Value::String(s) = value else {
        return Err(bad(value));
    };
    let s = s.trim().to_lowercase();
    let (number, px) = if let Some(n) = s.strip_suffix("px") {
        (n.trim().parse::<f64>().map_err(|_| bad(value))?, true)
    } else if let Some(n) = s.strip_suffix("pt") {
        (n.trim().parse::<f64>().map_err(|_| bad(value))?, false)
    } else {
        return Err(bad(value));
    };
    let pt = if px { number * 0.75 } else { number };
    if !(LAYOUT_MIN_PT..=LAYOUT_MAX_PT).contains(&pt) {
        return Err(format!(
            "{value} is out of range: sizes go from {LAYOUT_MIN_PT}pt to {LAYOUT_MAX_PT}pt"
        ));
    }
    Ok(pt)
}

fn bad(value: &Value) -> String {
    format!(
        "{value} isn't a size: give the unit, like \"12px\" or \"9pt\" (use the unit the user \
         used; 1pt = 1.33px)"
    )
}

/// The layout asked for in a tool's arguments: `font_size` scales everything from the body text;
/// `sizes` sets any of title / heading / subheading / text exactly; then spacing, header_footer,
/// paper, orientation, margin_mm.
pub fn from_args(args: &Value) -> Result<Layout, String> {
    let mut layout = Layout::default();
    if let Some(v) = args.get("font_size").filter(|v| !v.is_null()) {
        let body = parse_size(v)?;
        let k = body / layout.body_pt;
        layout.title_pt *= k;
        layout.heading_pt *= k;
        layout.subheading_pt *= k;
        layout.body_pt = body;
    }
    let mut title_given = false;
    if let Some(sizes) = args.get("sizes").and_then(Value::as_object) {
        for (name, v) in sizes {
            title_given |= name == "title";
            let pt = parse_size(v)?;
            match name.as_str() {
                "title" => layout.title_pt = pt,
                "heading" => layout.heading_pt = pt,
                "subheading" => layout.subheading_pt = pt,
                "text" => layout.body_pt = pt,
                other => {
                    return Err(format!(
                        "sizes has no `{other}`; use title, heading, subheading, text"
                    ));
                }
            }
        }
    }
    // The title stays a step above the headings unless it was sized on purpose.
    if !title_given {
        layout.title_pt = layout
            .title_pt
            .max(layout.heading_pt * LAYOUT_TITLE_OVER_HEADING);
    }
    match args.get("spacing").and_then(Value::as_str) {
        None | Some("normal") => {}
        Some("compact") => layout.compact = true,
        Some(other) => return Err(format!("spacing is compact or normal, not {other}")),
    }
    if let Some(b) = args.get("header_footer").and_then(Value::as_bool) {
        layout.header_footer = b;
    }
    match args
        .get("paper")
        .and_then(Value::as_str)
        .map(str::to_lowercase)
        .as_deref()
    {
        None | Some("a4") => {}
        Some("letter") => layout.paper = Paper::Letter,
        Some(other) => return Err(format!("paper is A4 or Letter, not {other}")),
    }
    match args.get("orientation").and_then(Value::as_str) {
        None | Some("portrait") => {}
        Some("landscape") => layout.landscape = true,
        Some(other) => return Err(format!("orientation is portrait or landscape, not {other}")),
    }
    if let Some(m) = args.get("margin_mm").and_then(Value::as_f64) {
        if !(0.0..=50.0).contains(&m) {
            return Err("margin_mm goes from 0 to 50".to_owned());
        }
        layout.margin_mm = m;
    }
    Ok(layout)
}

/// The layout options as JSON-schema properties, shared by make_pdf and edit_word.
pub fn schema_properties() -> serde_json::Map<String, Value> {
    let size = serde_json::json!({ "type": "string", "description": "With its unit: \"12px\" or \"9pt\"" });
    let mut sizes = serde_json::Map::new();
    for name in ["title", "heading", "subheading", "text"] {
        sizes.insert(name.to_owned(), size.clone());
    }
    let mut p = serde_json::Map::new();
    p.insert("font_size".into(), serde_json::json!({ "type": "string", "description": "Body text size with its unit (\"12px\" or \"9pt\"); headings keep their proportions" }));
    p.insert("sizes".into(), serde_json::json!({ "type": "object", "description": "Exact sizes for any of title (first #), heading (##), subheading (### and smaller), text", "properties": sizes }));
    p.insert(
        "spacing".into(),
        serde_json::json!({ "type": "string", "enum": ["normal", "compact"] }),
    );
    p.insert("header_footer".into(), serde_json::json!({ "type": "boolean", "description": "Title at the top of each page and page numbers at the bottom (default true)" }));
    p.insert(
        "paper".into(),
        serde_json::json!({ "type": "string", "enum": ["A4", "Letter"] }),
    );
    p.insert(
        "orientation".into(),
        serde_json::json!({ "type": "string", "enum": ["portrait", "landscape"] }),
    );
    p.insert("margin_mm".into(), serde_json::json!({ "type": "number", "description": "Margin on every side in mm (default 18)" }));
    p
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn sizes_read_in_px_or_pt() {
        assert_eq!(parse_size(&json!("12px")).unwrap(), 9.0);
        assert_eq!(parse_size(&json!("9pt")).unwrap(), 9.0);
        assert!(parse_size(&json!(9)).is_err(), "a bare number has no unit");
        assert!(parse_size(&json!("9")).is_err());
        assert!(parse_size(&json!("huge")).is_err());
        assert!(parse_size(&json!("400px")).is_err());
    }

    #[test]
    fn font_size_scales_headings_and_sizes_set_them_exactly() {
        let l = from_args(&json!({"font_size": "12px"})).unwrap();
        assert_eq!(l.body_pt, 9.0);
        assert!(l.title_pt > l.heading_pt && l.heading_pt > l.body_pt);
        let l = from_args(&json!({"sizes": {"title": "18px", "heading": "14px", "text": "12px"}}))
            .unwrap();
        assert_eq!((l.heading_pt, l.body_pt), (10.5, 9.0));
        assert!(
            l.title_pt > l.heading_pt,
            "the title stays above the headings"
        );
        let l = from_args(&json!({"sizes": {"title": "16px", "heading": "18px"}})).unwrap();
        assert_eq!(l.title_pt, 12.0, "unless it was sized on purpose");
        assert!(from_args(&json!({"sizes": {"huge": 9}})).is_err());
    }

    #[test]
    fn page_options() {
        let l = from_args(
            &json!({"spacing": "compact", "header_footer": false, "paper": "Letter",
            "orientation": "landscape", "margin_mm": 10}),
        )
        .unwrap();
        assert!(l.compact && !l.header_footer && l.landscape);
        assert_eq!((l.paper, l.margin_mm), (Paper::Letter, 10.0));
        assert!(from_args(&json!({"spacing": "airy"})).is_err());
    }
}
