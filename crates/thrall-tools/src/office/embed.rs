//! Pictures in a page that's printed to PDF. The browser prints from an empty tab, which can't
//! open files on the disk, so every local picture the page names (`src="…"`, CSS `url(…)`) is put
//! into the page itself as a `data:` address first. A picture that can't be found or read stops
//! the PDF and is named, instead of leaving an empty space.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use base64::Engine as _;
use regex::{Captures, Regex};

use crate::constants::{
    PDF_MAX_PICTURES, PDF_PICTURE_MAX_BYTES, PDF_PICTURE_TYPES, PDF_PICTURES_MAX_BYTES,
    PICTURE_SRC_PATTERN, PICTURE_URL_PATTERN,
};
use crate::paths::{display, inside, resolve};

static SRC: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new(PICTURE_SRC_PATTERN).ok());
static CSS_URL: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new(PICTURE_URL_PATTERN).ok());

/// Where pictures may come from: the working folder, and Thrall's own saved files (uploads and
/// pictures it made).
pub struct Sources<'a> {
    /// The working folder; relative names start here.
    pub workspace: &'a Path,
    /// Other folders pictures may be read from.
    pub also: Vec<PathBuf>,
}

/// A page with its pictures inside it.
#[derive(Debug)]
pub struct Embedded {
    /// The page.
    pub html: String,
    /// Different pictures put in.
    pub pictures: usize,
}

/// Whether `reference` names a file on this computer (not a web address or inline data).
fn is_local(reference: &str) -> bool {
    let r = reference.trim().to_lowercase();
    !(r.is_empty()
        || r.starts_with("data:")
        || r.starts_with("http:")
        || r.starts_with("https:")
        || r.starts_with("blob:")
        || r.starts_with("about:")
        || r.starts_with('#'))
}

/// `%20` and friends back to the characters they stand for.
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(b) = u8::from_str_radix(&text[i + 1..i + 3], 16)
        {
            out.push(b);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The file a local reference names.
fn file_of(sources: &Sources<'_>, reference: &str) -> PathBuf {
    let r = reference.trim();
    let r = r
        .strip_prefix("file:///")
        .or_else(|| r.strip_prefix("file://"))
        .unwrap_or(r);
    // `C:/…` keeps its drive; `/home/…` stays absolute on Unix.
    let r = if cfg!(windows) && r.starts_with('/') && r.get(2..3) == Some(":") {
        &r[1..]
    } else {
        r
    };
    resolve(sources.workspace, &percent_decode(r))
}

fn media_type(path: &Path) -> Option<&'static str> {
    let ext = path.extension()?.to_string_lossy().to_lowercase();
    PDF_PICTURE_TYPES
        .iter()
        .find(|(e, _)| *e == ext)
        .map(|(_, t)| *t)
}

/// Why `path` can't go into the PDF, if it can't (checked without reading it).
pub fn problem(sources: &Sources<'_>, path: &Path) -> Option<String> {
    let shown = display(sources.workspace, path);
    let allowed = inside(sources.workspace, path) || sources.also.iter().any(|d| inside(d, path));
    if !allowed {
        return Some(format!(
            "{shown} is outside the working folder (copy it in first)"
        ));
    }
    let Some(_) = media_type(path) else {
        let types: Vec<&str> = PDF_PICTURE_TYPES.iter().map(|(e, _)| *e).collect();
        return Some(format!(
            "{shown} isn't a picture this can use ({})",
            types.join(", ")
        ));
    };
    match std::fs::metadata(path) {
        Err(_) => Some(format!("{shown} wasn't found")),
        Ok(m) if !m.is_file() => Some(format!("{shown} isn't a file")),
        Ok(m) if m.len() > PDF_PICTURE_MAX_BYTES => Some(format!(
            "{shown} is too big ({} MB; the most is {} MB)",
            m.len() / (1024 * 1024),
            PDF_PICTURE_MAX_BYTES / (1024 * 1024)
        )),
        Ok(_) => None,
    }
}

/// Every local picture `html` names, in order, without repeats.
pub fn local_pictures(sources: &Sources<'_>, html: &str) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = Vec::new();
    let mut add = |reference: &str| {
        if is_local(reference) {
            let path = file_of(sources, reference);
            if !found.contains(&path) {
                found.push(path);
            }
        }
    };
    let (Some(src), Some(css_url)) = (SRC.as_ref(), CSS_URL.as_ref()) else {
        return found;
    };
    for c in src.captures_iter(html) {
        if let Some(m) = c.get(2).or_else(|| c.get(3)) {
            add(m.as_str());
        }
    }
    for c in css_url.captures_iter(html) {
        if let Some(m) = c.get(1).or_else(|| c.get(2)).or_else(|| c.get(3)) {
            add(m.as_str());
        }
    }
    found
}

/// What's wrong with the pictures `html` names: one line per picture, empty when all is well.
pub fn check(sources: &Sources<'_>, html: &str) -> Vec<String> {
    let pictures = local_pictures(sources, html);
    let mut problems: Vec<String> = pictures
        .iter()
        .filter_map(|p| problem(sources, p))
        .collect();
    if pictures.len() > PDF_MAX_PICTURES {
        problems.push(format!(
            "{} pictures is too many for one PDF (the most is {PDF_MAX_PICTURES})",
            pictures.len()
        ));
    }
    problems
}

/// `html` with every local picture put inside it.
pub fn embed(sources: &Sources<'_>, html: &str) -> Result<Embedded, String> {
    let problems = check(sources, html);
    if !problems.is_empty() {
        return Err(format!(
            "these pictures can't go into the PDF:\n- {}",
            problems.join("\n- ")
        ));
    }
    let mut inline: HashMap<PathBuf, String> = HashMap::new();
    let mut total: u64 = 0;
    for path in local_pictures(sources, html) {
        let bytes = std::fs::read(&path)
            .map_err(|e| format!("couldn't read {}: {e}", display(sources.workspace, &path)))?;
        total += bytes.len() as u64;
        if total > PDF_PICTURES_MAX_BYTES {
            return Err(format!(
                "the pictures add up to more than {} MB, too much for one PDF: split it into \
                 several",
                PDF_PICTURES_MAX_BYTES / (1024 * 1024)
            ));
        }
        let kind = media_type(&path).unwrap_or("application/octet-stream");
        let data = format!(
            "data:{kind};base64,{}",
            base64::engine::general_purpose::STANDARD.encode(&bytes)
        );
        inline.insert(path, data);
    }
    let swap = |reference: &str| -> Option<&String> {
        is_local(reference)
            .then(|| inline.get(&file_of(sources, reference)))
            .flatten()
    };
    let (Some(src), Some(css_url)) = (SRC.as_ref(), CSS_URL.as_ref()) else {
        return Err("couldn't look for pictures in the page".to_owned());
    };
    let html = src.replace_all(html, |c: &Captures<'_>| {
        let value = c.get(2).or_else(|| c.get(3)).map_or("", |m| m.as_str());
        match swap(value) {
            Some(data) => format!("{}\"{data}\"", &c[1]),
            None => c[0].to_owned(),
        }
    });
    let html = css_url.replace_all(&html, |c: &Captures<'_>| {
        let value = c
            .get(1)
            .or_else(|| c.get(2))
            .or_else(|| c.get(3))
            .map_or("", |m| m.as_str());
        match swap(value) {
            Some(data) => format!("url(\"{data}\")"),
            None => c[0].to_owned(),
        }
    });
    Ok(Embedded {
        html: html.into_owned(),
        pictures: inline.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "thrall-embed-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        ));
        std::fs::create_dir_all(dir.join("art")).unwrap();
        std::fs::write(dir.join("art/page one.png"), b"PNGDATA").unwrap();
        std::fs::write(dir.join("back.jpg"), b"JPG").unwrap();
        dir
    }

    #[test]
    fn local_pictures_go_inside_the_page_and_web_ones_stay() {
        let dir = folder();
        let sources = Sources {
            workspace: &dir,
            also: Vec::new(),
        };
        let html = r#"<img src="art/page%20one.png"><img src='art/page one.png'>
            <img src="https://example.com/a.png"><div style="background:url(back.jpg)"></div>"#;
        let done = embed(&sources, html).unwrap();
        assert_eq!(done.pictures, 2, "the same picture twice counts once");
        assert_eq!(done.html.matches("data:image/png;base64,").count(), 2);
        assert!(done.html.contains("url(\"data:image/jpeg;base64,"));
        assert!(done.html.contains("https://example.com/a.png"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn missing_or_outside_pictures_are_named_instead_of_left_blank() {
        let dir = folder();
        let sources = Sources {
            workspace: &dir,
            also: Vec::new(),
        };
        let outside = std::env::temp_dir().join("elsewhere.png");
        let html = format!(
            r#"<img src="gone.png"><img src="{}"><img src="notes.txt">"#,
            outside.display()
        );
        let err = embed(&sources, &html).unwrap_err();
        assert!(err.contains("gone.png wasn't found"), "{err}");
        assert!(err.contains("outside the working folder"), "{err}");
        assert!(err.contains("isn't a picture"), "{err}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn code_that_mentions_src_is_left_alone() {
        let dir = folder();
        let sources = Sources {
            workspace: &dir,
            also: Vec::new(),
        };
        // Markdown code is escaped (`&quot;`), so it never looks like a real attribute.
        assert!(check(&sources, "<code>src=&quot;x.png&quot;</code>").is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }
}
