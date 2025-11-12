// Reusable styles and colors for consistent UI appearance
#![allow(dead_code)]

use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols;
use ratatui::text::{Line, Span};
use ratatui::widgets::BorderType;

// Color scheme - using Light variants to match openapi-tui
pub const COLOR_PRIMARY: Color = Color::LightCyan;
pub const COLOR_SUCCESS: Color = Color::LightGreen;
pub const COLOR_WARNING: Color = Color::LightYellow;
pub const COLOR_ERROR: Color = Color::LightRed;
pub const COLOR_INACTIVE: Color = Color::Gray;
pub const COLOR_INFO: Color = Color::LightBlue;
pub const COLOR_ACCENT: Color = Color::LightMagenta;

// Border colors
pub const COLOR_FOCUSED_BORDER: Color = Color::LightGreen;

// Highlight symbol - using arrow like openapi-tui
pub const HIGHLIGHT_SYMBOL: &str = symbols::scrollbar::HORIZONTAL.end;

// Reusable styles
pub fn header_style() -> Style {
    Style::default().add_modifier(Modifier::BOLD)
}

pub fn highlight_style() -> Style {
    Style::default().add_modifier(Modifier::BOLD)
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

pub fn info_style() -> Style {
    Style::default().fg(COLOR_INFO)
}

pub fn accent_style() -> Style {
    Style::default().fg(COLOR_ACCENT)
}

pub fn dimmed_style() -> Style {
    Style::default().add_modifier(Modifier::DIM)
}

// Focus-based border styling (matching openapi-tui)
pub fn focused_border_style() -> Style {
    Style::default().fg(COLOR_FOCUSED_BORDER)
}

pub fn unfocused_border_style() -> Style {
    Style::default()
}

pub fn focused_border_type() -> BorderType {
    BorderType::Rounded
}

pub fn unfocused_border_type() -> BorderType {
    BorderType::Rounded
}

// Helper function to get border style based on focus state
pub fn border_style(focused: bool) -> Style {
    if focused {
        focused_border_style()
    } else {
        unfocused_border_style()
    }
}

pub fn border_type(focused: bool) -> BorderType {
    if focused {
        focused_border_type()
    } else {
        unfocused_border_type()
    }
}

// Shortcut formatting helper (matching openapi-tui format)
pub fn format_shortcuts(shortcuts: &[(&str, &str)]) -> String {
    shortcuts
        .iter()
        .map(|(key, action)| {
            format!("[{} {} {}]", key, HIGHLIGHT_SYMBOL, action)
        })
        .collect::<Vec<_>>()
        .join(" ")
}

// Styled shortcut formatting helper (returns Line with colored Spans)
pub fn format_shortcuts_styled(shortcuts: &[(&str, &str)]) -> Line<'static> {
    let mut spans = Vec::new();

    for (i, (key, action)) in shortcuts.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw(" "));
        }

        spans.push(Span::styled("[", dimmed_style()));
        spans.push(Span::styled(
            key.to_string(),
            Style::default()
                .fg(COLOR_PRIMARY)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(
            format!(" {} ", HIGHLIGHT_SYMBOL),
            dimmed_style(),
        ));
        spans.push(Span::raw(action.to_string()));
        spans.push(Span::styled("]", dimmed_style()));
    }

    Line::from(spans)
}

// Status colors for downloads
pub fn status_color(status: &str) -> Color {
    match status {
        "queued" => COLOR_INACTIVE,
        "in_progress" => COLOR_WARNING,
        "completed" => COLOR_PRIMARY,
        "failed" | "timed_out" => COLOR_ERROR,
        _ => Color::default(),
    }
}

// Format progress bar with styled filled/unfilled portions
pub fn format_progress_bar(
    progress: f64,
    width: usize,
    percentage: u8,
) -> Line<'static> {
    let filled = (progress * width as f64) as usize;
    let empty = width.saturating_sub(filled);

    let filled_part = "█".repeat(filled);
    let empty_part = "░".repeat(empty);

    let spans = vec![
        Span::raw("["),
        Span::styled(filled_part, primary_style()),
        Span::styled(empty_part, dimmed_style()),
        Span::raw("]"),
        Span::raw(" "),
        Span::styled(
            format!("{}%", percentage),
            primary_style()
                .add_modifier(Modifier::BOLD)
                .fg(Color::default()),
        ),
    ];

    Line::from(spans)
}
