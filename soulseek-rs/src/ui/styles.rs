// Reusable styles and colors for consistent UI appearance

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
/// Between a key and its action in the legend, spaced.
pub const SHORTCUT_ARROW: &str = " → ";

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

pub fn border_style(focused: bool) -> Style {
    if focused {
        focused_border_style()
    } else {
        unfocused_border_style()
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

/// A popup's title: `idle` until its list is filtered, then what it is
/// filtered by, a caret after it while that is still being typed, and how
/// the filter ends.
#[must_use]
pub fn filter_title(
    head: &str,
    filter: &str,
    typing: bool,
    idle: &str,
) -> String {
    if !typing && filter.is_empty() {
        return idle.to_string();
    }
    let caret = if typing { "_" } else { "" };
    format!(" {head} · filter: {filter}{caret}  (Enter: keep, Esc: clear) ")
}

/// The standard bordered pane. Chain `.title(..)` for a legend.
pub fn pane_block(focused: bool) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_style(border_style(focused))
        .border_type(BorderType::Rounded)
        .padding(PANE_PADDING)
}

pub fn download_status_glyph(status: &DownloadStatus) -> (&'static str, Style) {
    match status {
        DownloadStatus::Queued => (GLYPH_QUEUED, warning_style()),
        DownloadStatus::InProgress { .. } => (GLYPH_ACTIVE, accent_style()),
        DownloadStatus::Paused { .. } => (GLYPH_PAUSED, info_style()),
        DownloadStatus::Completed => (GLYPH_DONE, success_style()),
        DownloadStatus::Cancelled => (GLYPH_FAILED, inactive_style()),
        DownloadStatus::Failed(_) => (GLYPH_FAILED, error_style()),
        DownloadStatus::TimedOut => (GLYPH_TIMED_OUT, error_style()),
    }
}

/// One `[key → action]` legend entry.
fn shortcut_spans(
    key: &'static str,
    action: &'static str,
) -> [Span<'static>; 5] {
    [
        Span::styled("[", dimmed_style()),
        Span::styled(key, info_style()),
        Span::styled(SHORTCUT_ARROW, dimmed_style()),
        Span::styled(action, body_style()),
        Span::styled("]", dimmed_style()),
    ]
}

/// The most rows the shortcuts bar grows to. Past that a terminal is too
/// narrow for the legend to be worth the content rows it would take.
pub const SHORTCUT_ROWS_MAX: usize = 3;

/// Lay the legend out in rows no wider than `width`, never breaking inside an
/// entry: a `[key → action]` split over two rows reads as two keys. Entries
/// that do not fit in [`SHORTCUT_ROWS_MAX`] rows are dropped, so a bar always
/// has at least one row and never more than that.
pub fn pack_shortcuts(
    shortcuts: &[(&'static str, &'static str)],
    width: u16,
) -> Vec<Line<'static>> {
    let width = usize::from(width);
    let mut rows: Vec<Line<'static>> = Vec::new();
    let mut row: Vec<Span<'static>> = Vec::new();
    let mut row_width = 0;

    for (key, action) in shortcuts {
        let entry = shortcut_spans(key, action);
        let entry_width: usize = entry.iter().map(Span::width).sum();
        if !row.is_empty() && row_width + 1 + entry_width > width {
            rows.push(Line::from(std::mem::take(&mut row)));
            if rows.len() == SHORTCUT_ROWS_MAX {
                return rows;
            }
            row_width = 0;
        }
        if !row.is_empty() {
            row.push(Span::raw(" "));
            row_width += 1;
        }
        row.extend(entry);
        row_width += entry_width;
    }

    rows.push(Line::from(row));
    rows
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
            (DownloadStatus::Cancelled, GLYPH_FAILED, COLOR_INACTIVE),
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

    fn texts(rows: &[Line<'static>]) -> Vec<String> {
        rows.iter()
            .map(|line| line.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect()
    }

    #[test]
    fn shortcuts_stay_on_one_row_when_they_fit() {
        let rows = pack_shortcuts(&[("q", "quit"), ("s", "search")], 80);
        assert_eq!(texts(&rows), ["[q → quit] [s → search]"]);
    }

    #[test]
    fn shortcuts_wrap_between_entries_never_inside_one() {
        let rows = pack_shortcuts(
            &[("Space", "select"), ("Enter", "download"), ("q", "quit")],
            30,
        );
        // "[Space → select] [Enter → download]" is 35 cells, over the 30 the
        // bar has, so the second entry starts a new row whole.
        assert_eq!(
            texts(&rows),
            ["[Space → select]", "[Enter → download] [q → quit]"]
        );
    }

    #[test]
    fn an_entry_wider_than_the_bar_still_gets_a_row() {
        let rows = pack_shortcuts(&[("Enter", "download"), ("q", "quit")], 5);
        assert_eq!(texts(&rows), ["[Enter → download]", "[q → quit]"]);
    }

    #[test]
    fn the_bar_never_grows_past_its_cap() {
        let keys: Vec<(&str, &str)> = (0..10).map(|_| ("k", "act")).collect();
        let rows = pack_shortcuts(&keys, 12);
        assert_eq!(rows.len(), SHORTCUT_ROWS_MAX);
        assert_eq!(texts(&rows)[0], "[k → act]");
    }

    #[test]
    fn progress_bar_splits_fill_and_track() {
        let bar = format_progress_bar(0.25, 20, 25);
        assert_eq!(bar.spans[1].content.chars().count(), 5);
        assert_eq!(bar.spans[1].style.fg, Some(SIGNAL));
        assert_eq!(bar.spans[2].content.chars().count(), 15);
    }
}
