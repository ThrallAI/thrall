//! Markdown laid out as a print-ready HTML page: Thrall's typefaces (embedded), sizes and
//! spacing from a [`Layout`], real tables, and rules that keep headings with their text and table
//! rows whole. The browser prints it to PDF, with an optional running title and page numbers.

use base64::Engine as _;

use crate::browser::PrintSetup;
use crate::office::layout::Layout;
use crate::office::markdown;

const SANS_400: &[u8] = include_bytes!("../../assets/fonts/instrument-sans-latin-400-normal.woff2");
const SANS_600: &[u8] = include_bytes!("../../assets/fonts/instrument-sans-latin-600-normal.woff2");
const SANS_400_ITALIC: &[u8] =
    include_bytes!("../../assets/fonts/instrument-sans-latin-400-italic.woff2");
const SANS_600_ITALIC: &[u8] =
    include_bytes!("../../assets/fonts/instrument-sans-latin-600-italic.woff2");
const MONO_400: &[u8] = include_bytes!("../../assets/fonts/ibm-plex-mono-latin-400-normal.woff2");

/// Millimetres per inch.
const MM_PER_IN: f64 = 25.4;
/// The running header and footer need this much margin (mm) to sit in.
const HEADER_ROOM_MM: f64 = 14.0;

fn font_face(family: &str, weight: u16, style: &str, bytes: &[u8]) -> String {
    format!(
        "@font-face{{font-family:'{family}';font-weight:{weight};font-style:{style};\
         src:url(data:font/woff2;base64,{}) format('woff2');}}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    )
}

/// The stylesheet for `layout`. Gaps are in `em` of the body text, so they scale with it.
fn style(l: &Layout) -> String {
    // Line height and the gap between blocks: like the chat (24px lines on 15px text), or tight.
    let (line, gap) = if l.compact { (1.35, 0.35) } else { (1.6, 0.75) };
    let head_gap = if l.compact { 0.8 } else { 1.6 };
    format!(
        r"
:root{{--ink:#1c1b23;--muted:#5f5b6e;--accent:#6a4be0;--accent-soft:#efeafd;--line:#e4e1ec;--tint:#f7f6fb;}}
*{{box-sizing:border-box;}}
html{{-webkit-print-color-adjust:exact;print-color-adjust:exact;}}
body{{margin:0;font-family:'Instrument Sans','Segoe UI',Arial,sans-serif;font-size:{body}pt;line-height:{line};color:var(--ink);font-kerning:normal;text-rendering:optimizeLegibility;}}
h1,h2,h3,h4,h5,h6{{font-weight:600;line-height:1.3;break-after:avoid;page-break-after:avoid;margin:{head_gap}em 0 {gap}em;}}
h1{{font-size:{heading}pt;}}
body>h1:first-child{{font-size:{title}pt;margin:0 0 {title_gap}em;padding-bottom:0.35em;border-bottom:1.5pt solid var(--accent);}}
h2{{font-size:{heading}pt;}}
h3,h4,h5,h6{{font-size:{sub}pt;}}
h3{{color:var(--accent);}}
h4,h5,h6{{color:var(--muted);}}
p{{margin:0 0 {gap}em;orphans:3;widows:3;}}
strong{{font-weight:600;}}
a{{color:var(--accent);text-decoration:none;}}
hr{{border:0;border-top:0.75pt solid var(--line);margin:{head_gap}em 0;}}
ul,ol{{margin:0 0 {gap}em;padding-left:1.5em;}}
li{{margin:0.15em 0;}}
li>p{{margin:0 0 0.2em;}}
li::marker{{color:var(--accent);}}
table{{width:100%;border-collapse:collapse;margin:{gap}em 0 {table_after}em;font-size:0.92em;line-height:1.4;}}
thead{{display:table-header-group;}}
tr{{break-inside:avoid;page-break-inside:avoid;}}
th{{background:var(--accent-soft);font-weight:600;text-align:left;padding:{cell}em 0.7em;border-bottom:1.25pt solid var(--accent);}}
td{{padding:{cell}em 0.7em;border-bottom:0.5pt solid var(--line);vertical-align:top;}}
tbody tr:nth-child(even) td{{background:var(--tint);}}
code{{font-family:'IBM Plex Mono',Consolas,monospace;font-size:0.86em;background:var(--tint);border:0.5pt solid var(--line);border-radius:3pt;padding:0 0.3em;}}
pre{{font-family:'IBM Plex Mono',Consolas,monospace;font-size:0.84em;line-height:1.5;background:var(--tint);border:0.5pt solid var(--line);border-left:2.5pt solid var(--accent);border-radius:4pt;padding:0.8em 1em;margin:{gap}em 0;white-space:pre-wrap;word-break:break-word;break-inside:avoid;}}
pre code{{background:none;border:0;padding:0;font-size:inherit;}}
blockquote{{margin:{gap}em 0;padding:0.5em 1.1em;border-left:3pt solid var(--accent);background:var(--tint);color:var(--muted);border-radius:0 4pt 4pt 0;}}
blockquote p:last-child{{margin-bottom:0;}}
img{{max-width:100%;}}
.page-break{{break-after:page;page-break-after:always;}}
",
        body = l.body_pt,
        title = l.title_pt,
        heading = l.heading_pt,
        sub = l.subheading_pt,
        title_gap = if l.compact { 0.5 } else { 0.9 },
        table_after = gap * 1.4,
        cell = if l.compact { 0.25 } else { 0.4 },
    )
}

/// Escape text for HTML.
pub fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Thrall's typefaces as `@font-face` rules (embedded, so the PDF never falls back).
pub fn fonts() -> String {
    [
        font_face("Instrument Sans", 400, "normal", SANS_400),
        font_face("Instrument Sans", 600, "normal", SANS_600),
        font_face("Instrument Sans", 400, "italic", SANS_400_ITALIC),
        font_face("Instrument Sans", 600, "italic", SANS_600_ITALIC),
        font_face("IBM Plex Mono", 400, "normal", MONO_400),
    ]
    .concat()
}

/// The full HTML page for `markdown`, titled `title`.
pub fn document(markdown: &str, title: &str, layout: &Layout) -> String {
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><title>{}</title>\
         <style>{}{}</style></head><body>{}</body></html>",
        escape(title),
        fonts(),
        style(layout),
        markdown::to_html(markdown)
    )
}

/// The running header printed on every page: the document's title.
fn header(title: &str, side_mm: f64) -> String {
    format!(
        "<div style=\"width:100%;padding:0 {side_mm}mm;font-family:'Segoe UI',Arial,sans-serif;\
         font-size:7.5px;color:#8a8699;\">{}</div>",
        escape(title)
    )
}

/// The footer printed on every page: page X of Y.
fn footer(side_mm: f64) -> String {
    format!(
        "<div style=\"width:100%;padding:0 {side_mm}mm;font-family:'Segoe UI',Arial,sans-serif;\
         font-size:7.5px;color:#8a8699;text-align:right;\">\
         Page <span class=\"pageNumber\"></span> of <span class=\"totalPages\"></span></div>"
    )
}

/// Paper, margins, and the header and footer (if wanted) for printing `layout`.
pub fn setup(layout: &Layout, title: &str) -> PrintSetup {
    let (w, h) = layout.paper.mm();
    let (w, h) = if layout.landscape { (h, w) } else { (w, h) };
    let side = layout.margin_mm;
    let edge = if layout.header_footer {
        side.max(HEADER_ROOM_MM)
    } else {
        side
    };
    PrintSetup {
        width_in: w / MM_PER_IN,
        height_in: h / MM_PER_IN,
        margin_top_in: edge / MM_PER_IN,
        margin_bottom_in: edge / MM_PER_IN,
        margin_side_in: side / MM_PER_IN,
        header: layout
            .header_footer
            .then(|| (header(title, side), footer(side))),
        css_page_size: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::office::layout::Paper;

    #[test]
    fn the_page_embeds_fonts_and_uses_the_layout_sizes() {
        let layout = Layout {
            body_pt: 9.0,
            title_pt: 13.5,
            ..Layout::default()
        };
        let html = document(
            "# Report\n\n| A | B |\n|---|---|\n| 1 | 2 |\n",
            "Report <1>",
            &layout,
        );
        assert!(html.contains("<title>Report &lt;1&gt;</title>"));
        assert!(html.contains("data:font/woff2;base64,"));
        assert!(html.contains("font-size:9pt"));
        assert!(html.contains("font-size:13.5pt"));
        assert!(html.contains("<table>"));
    }

    #[test]
    fn page_setup_follows_paper_orientation_and_header_choice() {
        let plain = setup(
            &Layout {
                header_footer: false,
                landscape: true,
                paper: Paper::Letter,
                ..Layout::default()
            },
            "T",
        );
        assert!(plain.header.is_none());
        assert!((plain.width_in - 11.0).abs() < 0.01 && (plain.height_in - 8.5).abs() < 0.01);
        let with = setup(&Layout::default(), "A & B");
        let (h, f) = with.header.unwrap();
        assert!(h.contains("A &amp; B") && f.contains("totalPages"));
    }
}
