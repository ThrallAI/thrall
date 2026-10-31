//! Plain-text mode for consoles that can't show symbols. The classic Windows console (conhost with
//! a raster font and an OEM code page) turns ✓ · › ● and box borders into junk like `π`, so there
//! everything on screen is mapped to ASCII. Windows Terminal, VS Code and other terminals keep the
//! symbols.

use std::borrow::Cow;

use ratatui::buffer::Buffer;
use ratatui::symbols::border;
use ratatui::text::{Line, Span};

use crate::constants::{ASCII_ENV_VAR, SPINNER, SPINNER_ASCII};

/// Whether to draw with ASCII only. `THRALL_ASCII=1` or `=0` overrides the guess.
pub fn wanted() -> bool {
    match std::env::var(ASCII_ENV_VAR).ok().as_deref().map(str::trim) {
        Some("1" | "true" | "yes") => return true,
        Some("0" | "false" | "no") => return false,
        _ => {}
    }
    if !cfg!(windows) {
        return false;
    }
    // Windows Terminal sets WT_SESSION; VS Code and most others set TERM_PROGRAM.
    std::env::var_os("WT_SESSION").is_none() && std::env::var_os("TERM_PROGRAM").is_none()
}

/// The ASCII stand-in for a character, or `None` to keep it.
fn replace(c: char) -> Option<&'static str> {
    Some(match c {
        '·' => "-",
        '›' => ">",
        '●' | '•' => "*",
        '✓' => "+",
        '✗' => "x",
        '⊘' => "!",
        '○' => "o",
        '◐' => ">",
        '☑' => "[x]",
        '☐' => "[ ]",
        '…' => "...",
        '−' | '—' | '–' | '─' => "-",
        '“' | '”' => "\"",
        '‘' | '’' => "'",
        '│' => "|",
        '↓' => "v",
        _ => return None,
    })
}

/// `text` with symbols replaced by ASCII.
pub fn text(text: &str) -> Cow<'_, str> {
    if !text.chars().any(|c| replace(c).is_some()) {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match replace(c) {
            Some(r) => out.push_str(r),
            None => out.push(c),
        }
    }
    Cow::Owned(out)
}

/// A line with every span made ASCII.
pub fn line(line: Line<'static>) -> Line<'static> {
    let Line {
        spans,
        style,
        alignment,
    } = line;
    Line {
        spans: spans
            .into_iter()
            .map(|s| Span::styled(text(&s.content).into_owned(), s.style))
            .collect(),
        style,
        alignment,
    }
}

/// One screen cell's ASCII stand-in (cells hold one character, so the swaps are one-for-one).
fn cell(symbol: &str) -> Option<&'static str> {
    let mut chars = symbol.chars();
    let c = chars.next()?;
    if chars.next().is_some() || c.is_ascii() {
        return None;
    }
    Some(match c {
        '╭' | '╮' | '╰' | '╯' | '┌' | '┐' | '└' | '┘' | '┼' | '├' | '┤' | '┬' | '┴' => {
            "+"
        }
        '─' | '━' | '−' | '—' | '–' => "-",
        '│' | '┃' => "|",
        '·' => "-",
        '›' | '◐' => ">",
        '●' | '•' => "*",
        '✓' => "+",
        '✗' => "x",
        '⊘' => "!",
        '○' => "o",
        '☑' => "x",
        '☐' => "-",
        '…' => ".",
        '“' | '”' => "\"",
        '‘' | '’' => "'",
        '↓' => "v",
        '▶' | '►' => ">",
        _ => return None,
    })
}

/// Make everything drawn in this frame ASCII: borders, symbols, and whatever else slipped through.
pub fn buffer(buf: &mut Buffer) {
    for c in &mut buf.content {
        if let Some(r) = cell(c.symbol()) {
            c.set_symbol(r);
        }
    }
}

/// Spinner frames for the mode.
pub fn spinner(ascii: bool) -> &'static [&'static str] {
    if ascii { SPINNER_ASCII } else { SPINNER }
}

/// Box borders for the mode.
pub fn borders(ascii: bool) -> border::Set<'static> {
    if ascii {
        border::Set {
            top_left: "+",
            top_right: "+",
            bottom_left: "+",
            bottom_right: "+",
            vertical_left: "|",
            vertical_right: "|",
            horizontal_top: "-",
            horizontal_bottom: "-",
        }
    } else {
        border::ROUNDED
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn symbols_become_ascii() {
        assert_eq!(text("✓ Reading a.rs · 3 lines"), "+ Reading a.rs - 3 lines");
        assert_eq!(text("Edited x (+1 −0)"), "Edited x (+1 -0)");
        assert_eq!(text("☐ task…"), "[ ] task...");
    }

    #[test]
    fn plain_text_is_untouched() {
        assert!(matches!(text("hello"), Cow::Borrowed(_)));
    }
}
