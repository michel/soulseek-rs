// Reusable styles and colors for consistent UI appearance
#![allow(dead_code)]

use ratatui::style::{Color, Modifier, Style};

// Color scheme
pub const COLOR_PRIMARY: Color = Color::Cyan;
pub const COLOR_SUCCESS: Color = Color::Green;
pub const COLOR_WARNING: Color = Color::Yellow;
pub const COLOR_ERROR: Color = Color::Red;
pub const COLOR_INACTIVE: Color = Color::Gray;
pub const COLOR_BACKGROUND: Color = Color::DarkGray;
pub const COLOR_HIGHLIGHT_BG: Color = Color::Magenta;
pub const COLOR_HIGHLIGHT_FG: Color = Color::Black;

// Reusable styles
pub fn header_style() -> Style {
    Style::default()
        .fg(COLOR_PRIMARY)
        .bg(COLOR_BACKGROUND)
        .add_modifier(Modifier::BOLD)
}

pub fn highlight_style() -> Style {
    Style::default()
        .bg(COLOR_HIGHLIGHT_BG)
        .fg(COLOR_HIGHLIGHT_FG)
        .add_modifier(Modifier::BOLD)
}

pub fn success_style() -> Style {
    Style::default().fg(COLOR_SUCCESS)
}

pub fn warning_style() -> Style {
    Style::default().fg(COLOR_WARNING)
}

pub fn error_style() -> Style {
    Style::default().fg(COLOR_ERROR)
}

pub fn inactive_style() -> Style {
    Style::default().fg(COLOR_INACTIVE)
}

pub fn primary_style() -> Style {
    Style::default().fg(COLOR_PRIMARY)
}
