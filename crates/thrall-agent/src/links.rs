//! Links in Thrall's replies that lead nowhere. Some models (OpenAI's, from ChatGPT) link files
//! they made as `sandbox:/mnt/data/name.pdf`, ChatGPT's own folder. Thrall's files are in the
//! working folder and attached in the chat, so the link is replaced by the file's name.

use std::sync::LazyLock;

use regex::{Captures, Regex};

use crate::constants::{SANDBOX_LINK, SANDBOX_PATH};

static LINK: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new(SANDBOX_LINK).ok());
static PATH: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new(SANDBOX_PATH).ok());

/// `text` with every `sandbox:/mnt/data/…` link or address turned into the file's name, and
/// whether anything changed.
pub fn fix(text: &str) -> (String, bool) {
    let (Some(link), Some(path)) = (LINK.as_ref(), PATH.as_ref()) else {
        return (text.to_owned(), false);
    };
    if !text.contains("/mnt/data/") {
        return (text.to_owned(), false);
    }
    let named = link.replace_all(text, |c: &Captures<'_>| {
        let label = c[1].trim();
        let file = &c[2];
        let name = file.rsplit('/').next().unwrap_or(file);
        if label.is_empty() || label == file || label == name {
            format!("`{file}`")
        } else {
            format!("{label} (`{file}`)")
        }
    });
    let fixed = path.replace_all(&named, "$1$2").into_owned();
    let changed = fixed != text;
    (fixed, changed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chatgpt_links_become_the_file_name() {
        let (out, changed) =
            fix("[Download the colouring book](sandbox:/mnt/data/woodland-book.pdf)");
        assert!(changed);
        assert_eq!(out, "Download the colouring book (`woodland-book.pdf`)");
        assert_eq!(
            fix("[book.pdf](sandbox:/mnt/data/book.pdf)").0,
            "`book.pdf`"
        );
        assert_eq!(
            fix("Saved to sandbox:/mnt/data/art/page.png.").0,
            "Saved to art/page.png."
        );
    }

    #[test]
    fn other_links_are_left_alone() {
        let text = "See [the docs](https://example.com/mnt/data/x) and `notes.md`.";
        assert_eq!(fix(text), (text.to_owned(), false));
    }
}
