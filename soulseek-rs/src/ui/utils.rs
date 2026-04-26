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

pub fn get_bitrate(
    attribs: &std::collections::HashMap<u32, u32>,
) -> Option<u32> {
    attribs.get(&0).copied()
}

const SPINNER_CHARS: [&str; 10] =
    ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

pub fn get_spinner_char(state: usize) -> &'static str {
    SPINNER_CHARS[state % SPINNER_CHARS.len()]
}
