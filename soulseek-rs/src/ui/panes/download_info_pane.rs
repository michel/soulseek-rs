use crate::models::DownloadEntry;
use crate::ui::{
    border_style, border_type, dimmed_style, error_style, format_bytes,
    format_progress_bar, format_speed, inactive_style, info_style,
    primary_style, success_style, warning_style,
};
use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};
use soulseek_rs::DownloadStatus;
use soulseek_rs::utils::path::expand_tilde;

const LABEL_WIDTH: usize = 20;

pub fn render_download_info_pane(
    frame: &mut Frame,
    area: Rect,
    selected: Option<&DownloadEntry>,
    focused: bool,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style(focused))
        .border_type(border_type(focused))
        .title("[Info]");

    let Some(entry) = selected else {
        let paragraph = Paragraph::new(Line::from(Span::styled(
            "Select a download for details.",
            dimmed_style(),
        )))
        .block(block);
        frame.render_widget(paragraph, area);
        return;
    };

    let lines = build_info_lines(&entry.download);

    let paragraph = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, area);
}

fn build_info_lines(
    download: &soulseek_rs::types::Download,
) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();

    let (basename, parent_dir) = split_filename(&download.filename);

    lines.push(Line::from(Span::styled(
        basename,
        primary_style().add_modifier(ratatui::style::Modifier::BOLD),
    )));
    if !parent_dir.is_empty() {
        lines.push(Line::from(Span::styled(parent_dir, dimmed_style())));
    }
    lines.push(Line::from(""));

    lines.push(label_value("User", &download.username));
    lines.push(label_value("Size", &format_bytes(download.size)));

    let (status_text, status_style) = match &download.status {
        DownloadStatus::Queued => ("Queued".to_string(), inactive_style()),
        DownloadStatus::InProgress { .. } => {
            ("In progress".to_string(), warning_style())
        }
        DownloadStatus::Paused { .. } => ("Paused".to_string(), info_style()),
        DownloadStatus::Completed => ("Completed".to_string(), success_style()),
        DownloadStatus::Failed => ("Failed".to_string(), error_style()),
        DownloadStatus::TimedOut => ("Timed out".to_string(), error_style()),
    };
    lines.push(label_value_styled("Status", status_text, status_style));

    let save_path = expand_tilde(&download.download_directory)
        .to_string_lossy()
        .to_string();
    lines.push(label_value("Save to", &save_path));

    if let Some(bitrate) = download.metadata.bitrate {
        lines.push(label_value("Bitrate", &format!("{} kbps", bitrate)));
    }
    if let Some(length) = download.metadata.length_seconds {
        lines.push(label_value("Length", &format_duration(length)));
    }

    match &download.status {
        DownloadStatus::Queued => {
            lines.push(Line::from(""));
            let position_text = match download.queue_position {
                Some(p) => format!("#{}", p),
                None => "unknown".to_string(),
            };
            lines.push(label_value("Queue position", &position_text));

            if let Some(slots) = download.metadata.peer_free_slots {
                let slot_text = if slots > 0 {
                    format!("{} available", slots)
                } else {
                    "all busy".to_string()
                };
                lines.push(label_value("Peer slots", &slot_text));
            }

            if let Some(speed) = download.metadata.peer_upload_speed {
                lines.push(label_value(
                    "Peer upload speed",
                    &format_speed(speed as f64),
                ));
            }
        }
        DownloadStatus::InProgress {
            bytes_downloaded,
            total_bytes,
            speed_bytes_per_sec,
        } => {
            lines.push(Line::from(""));
            push_progress_lines(&mut lines, *bytes_downloaded, *total_bytes);
            lines.push(label_value(
                "Speed",
                &format_speed(*speed_bytes_per_sec),
            ));

            if *speed_bytes_per_sec > 0.0 && *total_bytes > *bytes_downloaded {
                let remaining = *total_bytes - *bytes_downloaded;
                let eta_secs = (remaining as f64 / speed_bytes_per_sec) as u32;
                lines.push(label_value("ETA", &format_duration(eta_secs)));
            }
        }
        DownloadStatus::Paused {
            bytes_downloaded,
            total_bytes,
        } => {
            lines.push(Line::from(""));
            push_progress_lines(&mut lines, *bytes_downloaded, *total_bytes);
        }
        DownloadStatus::Completed
        | DownloadStatus::Failed
        | DownloadStatus::TimedOut => {}
    }

    lines
}

fn label_value(label: &str, value: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!("{:<width$}", label, width = LABEL_WIDTH),
            dimmed_style(),
        ),
        Span::styled(value.to_string(), primary_style()),
    ])
}

fn label_value_styled(
    label: &str,
    value: String,
    value_style: Style,
) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!("{:<width$}", label, width = LABEL_WIDTH),
            dimmed_style(),
        ),
        Span::styled(value, value_style),
    ])
}

fn push_progress_lines(
    lines: &mut Vec<Line<'static>>,
    bytes_downloaded: u64,
    total_bytes: u64,
) {
    let (progress, percent) = if total_bytes > 0 {
        let ratio = bytes_downloaded as f64 / total_bytes as f64;
        (ratio, (ratio * 100.0) as u8)
    } else {
        (0.0, 0)
    };
    lines.push(format_progress_bar(progress, 20, percent));
    lines.push(label_value(
        "Downloaded",
        &format!(
            "{} / {}",
            format_bytes(bytes_downloaded),
            format_bytes(total_bytes)
        ),
    ));
}

fn split_filename(path: &str) -> (String, String) {
    let normalized = path.replace('\\', "/");
    if let Some(idx) = normalized.rfind('/') {
        let (parent, basename) = normalized.split_at(idx);
        (
            basename.trim_start_matches('/').to_string(),
            parent.to_string(),
        )
    } else {
        (normalized, String::new())
    }
}

fn format_duration(seconds: u32) -> String {
    let hours = seconds / 3600;
    let minutes = (seconds % 3600) / 60;
    let secs = seconds % 60;
    if hours > 0 {
        format!("{}h {:02}m {:02}s", hours, minutes, secs)
    } else if minutes > 0 {
        format!("{}m {:02}s", minutes, secs)
    } else {
        format!("{}s", secs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_duration_seconds_only() {
        assert_eq!(format_duration(0), "0s");
        assert_eq!(format_duration(45), "45s");
    }

    #[test]
    fn format_duration_minutes_and_seconds() {
        assert_eq!(format_duration(60), "1m 00s");
        assert_eq!(format_duration(125), "2m 05s");
    }

    #[test]
    fn format_duration_hours_minutes_seconds() {
        assert_eq!(format_duration(3600), "1h 00m 00s");
        assert_eq!(format_duration(3725), "1h 02m 05s");
    }

    #[test]
    fn split_filename_with_backslash_path() {
        let (basename, parent) =
            split_filename("@@drive\\Music\\Album\\track.mp3");
        assert_eq!(basename, "track.mp3");
        assert_eq!(parent, "@@drive/Music/Album");
    }

    #[test]
    fn split_filename_with_forward_slash_path() {
        let (basename, parent) = split_filename("/home/user/track.mp3");
        assert_eq!(basename, "track.mp3");
        assert_eq!(parent, "/home/user");
    }

    #[test]
    fn split_filename_with_no_separator() {
        let (basename, parent) = split_filename("track.mp3");
        assert_eq!(basename, "track.mp3");
        assert_eq!(parent, "");
    }
}
