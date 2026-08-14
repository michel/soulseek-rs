use ratatui::{
    style::Style,
    text::{Line, Span},
};

use crate::ui::COLOR_PRIMARY;

pub const BYTES_PER_MB: f64 = 1_048_576.0;

pub fn format_bytes(bytes: u64) -> String {
    let mb = bytes as f64 / BYTES_PER_MB;
    format!("{mb:.1} MB")
}

pub fn format_bytes_progress(downloaded: u64, total: u64) -> Line<'static> {
    let downloaded_mb = downloaded as f64 / BYTES_PER_MB;
    let total_mb = total as f64 / BYTES_PER_MB;

    Line::from(vec![
        Span::styled(
            format!("{downloaded_mb:.1}/{total_mb:.1}"),
            Style::default().fg(COLOR_PRIMARY),
        ),
        Span::raw(" MB"),
    ])
}

pub fn format_speed(speed_bytes_per_sec: f64) -> String {
    let mb = speed_bytes_per_sec / BYTES_PER_MB;
    format!("{mb:.1} MB/s")
}

/// Wrap a chat message into display lines: the prefix spans (time, sender)
/// start the first line, and continuation lines are indented to the prefix
/// width so the message text stays column-aligned.
// ponytail: char-count wrap; wide glyphs may spill a cell — switch to
// unicode-width if that ever matters.
pub fn wrap_chat_line(
    prefix: Vec<Span<'static>>,
    text: &str,
    style: Style,
    width: usize,
) -> Vec<Line<'static>> {
    let prefix_width: usize = prefix.iter().map(Span::width).sum();
    let avail = width.saturating_sub(prefix_width).max(1);
    let chars: Vec<char> = text.chars().collect();
    let indent = " ".repeat(prefix_width);

    let mut lines = Vec::new();
    let mut i = 0;
    loop {
        let chunk: String = chars[i..].iter().take(avail).collect();
        let spans = if i == 0 {
            let mut spans = prefix.clone();
            spans.push(Span::styled(chunk, style));
            spans
        } else {
            vec![Span::raw(indent.clone()), Span::styled(chunk, style)]
        };
        lines.push(Line::from(spans));
        i += avail;
        if i >= chars.len() {
            return lines;
        }
    }
}

const SPINNER_CHARS: [&str; 10] =
    ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

pub const fn get_spinner_char(state: usize) -> &'static str {
    SPINNER_CHARS[state % SPINNER_CHARS.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_chat_line_wraps_and_indents() {
        let prefix = vec![Span::raw("12:00 "), Span::raw("<bob> ")];
        let lines =
            wrap_chat_line(prefix, "abcdefghij", Style::default(), 16);
        // prefix width 12, so 4 text chars per row.
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].to_string(), "12:00 <bob> abcd");
        assert_eq!(lines[1].to_string(), "            efgh");
        assert_eq!(lines[2].to_string(), "            ij");
    }

    #[test]
    fn wrap_chat_line_short_message_is_one_line() {
        let lines =
            wrap_chat_line(vec![Span::raw("x ")], "hi", Style::default(), 80);
        assert_eq!(lines.len(), 1);
    }
}
