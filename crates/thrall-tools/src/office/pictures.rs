//! The two make_pdf layouts that aren't a document: `pictures` (one picture per page, for
//! colouring books, photo albums and scans) and `custom` (a page Thrall designs itself, printed as
//! it is). Neither uses the document's title, headings or page numbers.

use std::path::Path;

use crate::constants::PICTURE_PAGE_SLACK_MM;
use crate::office::layout::Layout;
use crate::office::print::{escape, fonts};

/// One page of the pictures layout.
#[derive(Debug, Clone, PartialEq)]
pub struct Picture {
    /// The picture, as given (a path in or below the working folder).
    pub path: String,
    /// A line under it.
    pub caption: Option<String>,
}

/// How pictures sit on their pages.
#[derive(Debug, Clone, PartialEq)]
pub struct PictureLayout {
    /// Fill the page edge to edge (cropping if the shapes differ) instead of showing the whole
    /// picture.
    pub fill: bool,
    /// Turn each page to match its picture (else every page keeps the paper's orientation).
    pub turn_to_fit: bool,
    /// A first page with this title.
    pub cover: Option<String>,
    /// A blank page after each picture, so a page printed on both sides doesn't show through.
    pub blank_backs: bool,
}

/// Whether the picture at `path` is wider than it is tall (unknown: no).
fn is_wide(path: &Path) -> bool {
    imagesize::size(path).is_ok_and(|s| s.width > s.height)
}

/// The pictures layout as a page to print. `file` turns a picture's path into its file, to read
/// its shape. Pages are sized by CSS, so print it with the CSS page size.
pub fn pictures_page(
    pictures: &[Picture],
    how: &PictureLayout,
    layout: &Layout,
    title: &str,
    file: impl Fn(&str) -> std::path::PathBuf,
) -> String {
    let (w, h) = layout.paper.mm();
    let (w, h) = if layout.landscape { (h, w) } else { (w, h) };
    let m = layout.margin_mm;
    let fit = if how.fill { "cover" } else { "contain" };
    let style = format!(
        "@page{{size:{w}mm {h}mm;margin:0;}}\
         @page turned{{size:{h}mm {w}mm;margin:0;}}\
         html,body{{margin:0;padding:0;}}\
         body{{font-family:'Instrument Sans','Segoe UI',Arial,sans-serif;color:#1c1b23;}}\
         .sheet{{box-sizing:border-box;width:{w}mm;height:{sh}mm;padding:{m}mm;display:flex;\
         flex-direction:column;align-items:center;justify-content:center;gap:4mm;\
         overflow:hidden;break-after:page;page-break-after:always;}}\
         .sheet.turned{{page:turned;width:{h}mm;height:{th}mm;}}\
         .sheet:last-child{{break-after:auto;page-break-after:auto;}}\
         .sheet img{{display:block;flex:1 1 0;min-height:0;width:100%;object-fit:{fit};}}\
         .caption{{font-size:14pt;text-align:center;}}\
         .cover h1{{font-size:32pt;font-weight:600;text-align:center;margin:0;\
         text-wrap:balance;}}",
        sh = h - PICTURE_PAGE_SLACK_MM,
        th = w - PICTURE_PAGE_SLACK_MM,
    );
    let mut body = String::new();
    let blank = "<div class=\"sheet\"></div>";
    if let Some(cover) = how.cover.as_deref().filter(|c| !c.trim().is_empty()) {
        body.push_str(&format!(
            "<div class=\"sheet cover\"><h1>{}</h1></div>",
            escape(cover)
        ));
        if how.blank_backs {
            body.push_str(blank);
        }
    }
    let paper_is_wide = w > h;
    for p in pictures {
        let turned = how.turn_to_fit && is_wide(&file(&p.path)) != paper_is_wide;
        body.push_str(&format!(
            "<div class=\"sheet{}\"><img src=\"{}\" alt=\"\">{}</div>",
            if turned { " turned" } else { "" },
            escape(&p.path),
            p.caption
                .as_deref()
                .filter(|c| !c.trim().is_empty())
                .map(|c| format!("<div class=\"caption\">{}</div>", escape(c)))
                .unwrap_or_default()
        ));
        if how.blank_backs {
            body.push_str(blank);
        }
    }
    if how.blank_backs {
        // No blank sheet after the last picture.
        body.truncate(body.len() - blank.len());
    }
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><title>{}</title>\
         <style>{}{style}</style></head><body>{body}</body></html>",
        escape(title),
        fonts()
    )
}

/// Thrall's own page, ready to print: a whole HTML page is kept as it is (Thrall's fonts added),
/// and a fragment gets a plain page around it.
pub fn custom_page(html: &str, title: &str) -> String {
    let fonts = format!("<style>{}</style>", fonts());
    let lower = html.to_lowercase();
    if let Some(at) = lower.find("<head>") {
        let at = at + "<head>".len();
        return format!("{}{fonts}{}", &html[..at], &html[at..]);
    }
    if lower.contains("<html") || lower.contains("<body") {
        return html.to_owned();
    }
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><title>{}</title>\
         {fonts}<style>body{{margin:0;font-family:'Instrument Sans','Segoe UI',Arial,\
         sans-serif;}}</style></head><body>{html}</body></html>",
        escape(title)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pics(n: usize) -> Vec<Picture> {
        (1..=n)
            .map(|i| Picture {
                path: format!("page-{i}.png"),
                caption: (i == 1).then(|| "The picnic".to_owned()),
            })
            .collect()
    }

    #[test]
    fn one_picture_per_page_with_no_document_parts() {
        let how = PictureLayout {
            fill: false,
            turn_to_fit: true,
            cover: Some("Woodland Friends".to_owned()),
            blank_backs: false,
        };
        let html = pictures_page(&pics(3), &how, &Layout::default(), "Book", |p| p.into());
        assert_eq!(html.matches("<img src=").count(), 3);
        assert_eq!(
            html.matches("class=\"sheet").count(),
            4,
            "a cover and three pictures"
        );
        assert!(html.contains("<h1>Woodland Friends</h1>"));
        assert!(html.contains("<div class=\"caption\">The picnic</div>"));
        assert!(html.contains("object-fit:contain"));
        assert!(html.contains("size:210mm 297mm"));
        assert!(!html.contains("pageNumber"), "no page numbers");
    }

    #[test]
    fn blank_backs_go_between_pages_but_not_after_the_last() {
        let how = PictureLayout {
            fill: true,
            turn_to_fit: false,
            cover: None,
            blank_backs: true,
        };
        let html = pictures_page(&pics(3), &how, &Layout::default(), "Book", |p| p.into());
        assert_eq!(html.matches("<div class=\"sheet\"></div>").count(), 2);
        assert!(html.ends_with("</div></body></html>") && html.contains("object-fit:cover"));
        assert!(
            !html
                .trim_end_matches("</body></html>")
                .ends_with("<div class=\"sheet\"></div>")
        );
    }

    #[test]
    fn custom_pages_are_printed_as_written() {
        let whole =
            "<!doctype html><html><head><style>@page{size:A5}</style></head><body>Hi</body></html>";
        let page = custom_page(whole, "T");
        assert!(page.contains("@page{size:A5}") && page.contains("Instrument Sans"));
        assert!(page.starts_with("<!doctype html><html><head><style>@font-face"));
        let bit = custom_page("<h1>Certificate</h1>", "Award");
        assert!(bit.contains("<title>Award</title>") && bit.contains("<body><h1>Certificate</h1>"));
    }
}
