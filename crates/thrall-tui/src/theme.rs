//! The terminal palette: Thrall's violet accent, used sparingly, on the terminal's own background.

use ratatui::style::{Color, Modifier, Style};

/// Thrall violet.
pub const ACCENT: Color = Color::Rgb(139, 108, 255);
/// Secondary text.
pub const DIM: Color = Color::Rgb(140, 140, 155);
/// Success.
pub const OK: Color = Color::Rgb(74, 222, 128);
/// Failure.
pub const ERROR: Color = Color::Rgb(255, 107, 107);
/// Caution.
pub const WARN: Color = Color::Rgb(250, 204, 21);
/// Inline code.
pub const CODE: Color = Color::Rgb(214, 170, 255);

/// Plain text.
pub fn text() -> Style {
    Style::default()
}

/// De-emphasised text.
pub fn dim() -> Style {
    Style::default().fg(DIM)
}

/// The accent.
pub fn accent() -> Style {
    Style::default().fg(ACCENT)
}

/// Bold accent, for headings and the user's prompt marker.
pub fn strong_accent() -> Style {
    accent().add_modifier(Modifier::BOLD)
}
