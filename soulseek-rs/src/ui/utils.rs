use ratatui::{
    style::Style,
    text::{Line, Span},
};

use crate::ui::COLOR_PRIMARY;

pub fn format_bytes(bytes: u64) -> String {
    let mb = bytes as f64 / 1_048_576.0;
    format!("{:.1} MB", mb)
}

pub fn format_bytes_progress(downloaded: u64, total: u64) -> Line<'static> {
    let downloaded_mb = downloaded as f64 / 1_048_576.0;
    let total_mb = total as f64 / 1_048_576.0;

    let mut spans = vec![];
    spans.push(Span::styled(
        format!("{:.1}/{:.1}", downloaded_mb, total_mb),
        Style::default().fg(COLOR_PRIMARY),
    ));
    spans.push(Span::raw(" MB"));

    Line::from(spans)
}

pub fn format_speed(speed_bytes_per_sec: f64) -> String {
    let mb = speed_bytes_per_sec / 1_048_576.0;
    format!("{:.1} MB/s", mb)
}

pub fn get_bitrate(
    attribs: &std::collections::HashMap<u32, u32>,
) -> Option<u32> {
    attribs.get(&0).copied()
}

// Spinner animation chars
const SPINNER_CHARS: [&str; 10] =
    ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

pub fn get_spinner_char(state: usize) -> &'static str {
    SPINNER_CHARS[state % SPINNER_CHARS.len()]
}
