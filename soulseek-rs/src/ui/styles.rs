// Reusable styles and colors for consistent UI appearance
#![allow(dead_code)]

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Padding};
use soulseek_rs::DownloadStatus;

/// One column of breathing room on each side, so content never touches the
/// border. Vertical rows stay unpadded — they are too scarce in a terminal.
pub const PANE_PADDING: Padding = Padding::horizontal(1);

pub const VINYL_2: Color = Color::Rgb(0x26, 0x21, 0x1D);
/// A warm oxide wash for the cursor row — dark enough that every column keeps
/// its own colour on top of it.
pub const SELECTION: Color = Color::Rgb(0x30, 0x1B, 0x14);
pub const DUST: Color = Color::Rgb(0x8A, 0x81, 0x78);
pub const PAPER: Color = Color::Rgb(0xE8, 0xE1, 0xD6);
pub const PAPER_DIM: Color = Color::Rgb(0xC9, 0xC1, 0xB4);
pub const OXIDE: Color = Color::Rgb(0xC1, 0x50, 0x2E);
pub const TAPE: Color = Color::Rgb(0xD8, 0xA6, 0x57);
pub const PHOSPHOR: Color = Color::Rgb(0x7F, 0xB6, 0x85);
pub const SIGNAL: Color = Color::Rgb(0x6E, 0x9D, 0xC9);
pub const ALARM: Color = Color::Rgb(0xB3, 0x3A, 0x3A);

pub const COLOR_PRIMARY: Color = PAPER;
pub const COLOR_ACCENT: Color = OXIDE;
pub const COLOR_SUCCESS: Color = PHOSPHOR;
pub const COLOR_WARNING: Color = TAPE;
pub const COLOR_ERROR: Color = ALARM;
pub const COLOR_INFO: Color = SIGNAL;
pub const COLOR_INACTIVE: Color = DUST;
pub const COLOR_FOCUSED_BORDER: Color = PHOSPHOR;

pub const GLYPH_QUEUED: &str = "⋯";
pub const GLYPH_ACTIVE: &str = "↯";
pub const GLYPH_PAUSED: &str = "⏸";
pub const GLYPH_DONE: &str = "✓";
pub const GLYPH_FAILED: &str = "✗";
pub const GLYPH_TIMED_OUT: &str = "⧗";
pub const GLYPH_CURSOR: &str = "▮";
pub const HIGHLIGHT_SYMBOL: &str = "›";
pub const SHORTCUT_ARROW: &str = "→";

pub fn header_style() -> Style {
    Style::default().fg(DUST).add_modifier(Modifier::BOLD)
}

pub fn highlight_style() -> Style {
    Style::default()
        .bg(SELECTION)
        .fg(PAPER)
        .add_modifier(Modifier::BOLD)
}

/// The cursor row in a table. No foreground: a table row already colours its
/// own cells, and setting one here would flatten them all to a single tone.
pub fn row_highlight_style() -> Style {
    Style::default().bg(SELECTION).add_modifier(Modifier::BOLD)
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
    Style::default().fg(DUST)
}

pub fn body_style() -> Style {
    Style::default().fg(PAPER_DIM)
}

pub fn focused_border_style() -> Style {
    Style::default().fg(COLOR_FOCUSED_BORDER)
}

pub fn unfocused_border_style() -> Style {
    Style::default().fg(DUST)
}

pub const fn focused_border_type() -> BorderType {
    BorderType::Rounded
}

pub const fn unfocused_border_type() -> BorderType {
    BorderType::Rounded
}

pub fn border_style(focused: bool) -> Style {
    if focused {
        focused_border_style()
    } else {
        unfocused_border_style()
    }
}

pub const fn border_type(focused: bool) -> BorderType {
    if focused {
        focused_border_type()
    } else {
        unfocused_border_type()
    }
}

pub fn title_style(focused: bool) -> Style {
    if focused {
        Style::default().fg(PHOSPHOR).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(DUST)
    }
}

pub fn pane_title(tag: &str, title: &str, focused: bool) -> Line<'static> {
    let label = if title.is_empty() {
        " ".to_string()
    } else {
        format!(" {title} ")
    };
    Line::from(vec![
        Span::styled(format!(" [{tag}]"), Style::default().fg(SIGNAL)),
        Span::styled(label, title_style(focused)),
    ])
}

pub fn plain_title(title: impl Into<String>, focused: bool) -> Line<'static> {
    Line::from(Span::styled(
        format!(" {} ", title.into()),
        title_style(focused),
    ))
}

/// The standard bordered pane. Chain `.title(..)` for a legend.
pub fn pane_block(focused: bool) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_style(border_style(focused))
        .border_type(border_type(focused))
        .padding(PANE_PADDING)
}

pub fn download_status_glyph(status: &DownloadStatus) -> (&'static str, Style) {
    match status {
        DownloadStatus::Queued => (GLYPH_QUEUED, warning_style()),
        DownloadStatus::InProgress { .. } => (GLYPH_ACTIVE, accent_style()),
        DownloadStatus::Paused { .. } => (GLYPH_PAUSED, info_style()),
        DownloadStatus::Completed => (GLYPH_DONE, success_style()),
        DownloadStatus::Failed(_) => (GLYPH_FAILED, error_style()),
        DownloadStatus::TimedOut => (GLYPH_TIMED_OUT, error_style()),
    }
}

pub fn format_shortcuts_styled(shortcuts: &[(&str, &str)]) -> Line<'static> {
    let mut spans = Vec::new();

    for (i, (key, action)) in shortcuts.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw(" "));
        }

        spans.push(Span::styled("[", dimmed_style()));
        spans.push(Span::styled(key.to_string(), Style::default().fg(SIGNAL)));
        spans.push(Span::styled(format!(" {SHORTCUT_ARROW} "), dimmed_style()));
        spans.push(Span::styled(action.to_string(), body_style()));
        spans.push(Span::styled("]", dimmed_style()));
    }

    Line::from(spans)
}

pub fn format_progress_bar(
    progress: f64,
    width: usize,
    percentage: u8,
) -> Line<'static> {
    let filled = (progress * width as f64) as usize;
    let empty = width.saturating_sub(filled);

    let spans = vec![
        Span::styled("[", dimmed_style()),
        Span::styled("█".repeat(filled), Style::default().fg(SIGNAL)),
        Span::styled("░".repeat(empty), Style::default().fg(VINYL_2)),
        Span::styled("]", dimmed_style()),
        Span::raw(" "),
        Span::styled(
            format!("{percentage}%"),
            primary_style().add_modifier(Modifier::BOLD),
        ),
    ];

    Line::from(spans)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn colors(line: &Line<'static>) -> Vec<Option<Color>> {
        line.spans.iter().map(|s| s.style.fg).collect()
    }

    #[test]
    fn pane_title_tags_in_signal_and_tracks_focus() {
        assert_eq!(
            colors(&pane_title("2", "Results", true)),
            vec![Some(SIGNAL), Some(PHOSPHOR)]
        );
        assert_eq!(
            colors(&pane_title("2", "Results", false)),
            vec![Some(SIGNAL), Some(DUST)]
        );
    }

    #[test]
    fn titles_are_inset_from_the_border() {
        let inset = |line: &Line<'static>| {
            let text: String =
                line.spans.iter().map(|s| s.content.as_ref()).collect();
            text.starts_with(' ') && text.ends_with(' ')
        };
        assert!(inset(&pane_title("2", "Results", true)));
        assert!(inset(&pane_title("Info", "", true)));
        assert!(inset(&plain_title("Status", false)));
    }

    #[test]
    fn download_status_glyphs_carry_their_semantic_color() {
        for (status, glyph, color) in [
            (DownloadStatus::Queued, GLYPH_QUEUED, TAPE),
            (DownloadStatus::Completed, GLYPH_DONE, PHOSPHOR),
            (DownloadStatus::Failed(None), GLYPH_FAILED, ALARM),
            (DownloadStatus::TimedOut, GLYPH_TIMED_OUT, ALARM),
        ] {
            let (got_glyph, style) = download_status_glyph(&status);
            assert_eq!(got_glyph, glyph);
            assert_eq!(style.fg, Some(color));
        }
    }

    #[test]
    fn table_cursor_row_tints_the_background_without_recolouring_cells() {
        let cursor = row_highlight_style();
        assert_eq!(cursor.fg, None);
        assert_eq!(cursor.bg, Some(SELECTION));
    }

    #[test]
    fn progress_bar_splits_fill_and_track() {
        let bar = format_progress_bar(0.25, 20, 25);
        assert_eq!(bar.spans[1].content.chars().count(), 5);
        assert_eq!(bar.spans[1].style.fg, Some(SIGNAL));
        assert_eq!(bar.spans[2].content.chars().count(), 15);
    }
}
