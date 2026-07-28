use crate::models::DownloadEntry;
use crate::ui::{
    accent_style, dimmed_style, error_style, format_bytes, format_progress_bar,
    format_speed, inactive_style, info_style, pane_block, pane_title,
    primary_style, success_style, warning_style,
};
use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::{Paragraph, Wrap},
};
use soulseek_rs::DownloadStatus;
use soulseek_rs::types::{UploadInfo, UploadStatus};
use soulseek_rs::utils::path::expand_tilde;

const LABEL_WIDTH: usize = 20;

/// Whichever transfer the table has highlighted. Rows are downloads first and
/// uploads after, so a raw index has to be resolved against both lists.
pub enum SelectedTransfer<'a> {
    Download(&'a DownloadEntry),
    Upload(&'a UploadInfo),
}

/// Resolve the transfers table's selected row index into the download or
/// upload it points at, matching the order [`render_downloads_pane`] renders.
#[must_use]
pub fn selected_transfer<'a>(
    index: Option<usize>,
    downloads: &'a [DownloadEntry],
    uploads: &'a [UploadInfo],
) -> Option<SelectedTransfer<'a>> {
    let index = index?;
    if let Some(download) = downloads.get(index) {
        return Some(SelectedTransfer::Download(download));
    }
    // Past the downloads, so this row is an upload.
    uploads
        .get(index - downloads.len())
        .map(SelectedTransfer::Upload)
}

pub fn render_download_info_pane(
    frame: &mut Frame,
    area: Rect,
    selected: Option<SelectedTransfer>,
    focused: bool,
) {
    let block = pane_block(focused).title(pane_title("Info", "", focused));

    let Some(selected) = selected else {
        let paragraph = Paragraph::new(Line::from(Span::styled(
            "Select a transfer for details.",
            dimmed_style(),
        )))
        .block(block);
        frame.render_widget(paragraph, area);
        return;
    };

    let lines = match selected {
        SelectedTransfer::Download(entry) => build_info_lines(&entry.download),
        SelectedTransfer::Upload(upload) => build_upload_info_lines(upload),
    };

    let paragraph = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, area);
}

/// Details for a file we are serving to a peer, laid out like the download
/// view so the pane reads the same whichever direction is selected.
fn build_upload_info_lines(upload: &UploadInfo) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();

    let (basename, parent_dir) = split_filename(&upload.filename);
    lines.push(Line::from(Span::styled(
        basename,
        primary_style().add_modifier(ratatui::style::Modifier::BOLD),
    )));
    if !parent_dir.is_empty() {
        lines.push(Line::from(Span::styled(parent_dir, dimmed_style())));
    }
    lines.push(Line::from(""));

    // The pane is shared with downloads, so name the direction explicitly.
    lines.push(label_value("Direction", "↑ Upload"));
    lines.push(label_value("To user", &upload.username));
    lines.push(label_value("Size", &format_bytes(upload.size)));

    let (status_text, status_style) = match &upload.status {
        UploadStatus::Queued(place) => {
            (format!("Queued (#{place})"), inactive_style())
        }
        UploadStatus::InProgress => {
            ("In progress".to_string(), warning_style())
        }
        UploadStatus::Completed => ("Completed".to_string(), success_style()),
        UploadStatus::Cancelled => ("Cancelled".to_string(), inactive_style()),
        UploadStatus::Failed(_) => ("Failed".to_string(), error_style()),
    };
    lines.push(label_value_styled("Status", status_text, status_style));

    match &upload.status {
        UploadStatus::InProgress => {
            lines.push(Line::from(""));
            push_transfer_progress(
                &mut lines,
                "Sent",
                upload.bytes_sent,
                upload.size,
            );
            lines.push(label_value(
                "Speed",
                &format_speed(upload.speed_bytes_per_sec),
            ));

            if upload.speed_bytes_per_sec > 0.0
                && upload.size > upload.bytes_sent
            {
                let remaining = upload.size - upload.bytes_sent;
                let eta_secs =
                    (remaining as f64 / upload.speed_bytes_per_sec) as u32;
                lines.push(label_value("ETA", &format_duration(eta_secs)));
            }
        }
        UploadStatus::Failed(reason) => {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                format!("{:<LABEL_WIDTH$}", "Error"),
                dimmed_style(),
            )));
            lines.push(Line::from(Span::styled(reason.clone(), error_style())));
        }
        UploadStatus::Queued(_)
        | UploadStatus::Completed
        | UploadStatus::Cancelled => {}
    }

    lines
}

fn build_info_lines(
    download: &soulseek_rs::types::Download,
) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();

    let (basename, parent_dir) = split_filename(&download.filename);

    lines.push(Line::from(Span::styled(
        basename,
        accent_style().add_modifier(ratatui::style::Modifier::BOLD),
    )));
    if !parent_dir.is_empty() {
        lines.push(Line::from(Span::styled(parent_dir, dimmed_style())));
    }
    lines.push(Line::from(""));

    lines.push(label_value_styled(
        "User",
        download.username.clone(),
        info_style(),
    ));
    lines.push(label_value_styled(
        "Size",
        format_bytes(download.size),
        warning_style(),
    ));

    let (status_text, status_style) = match &download.status {
        DownloadStatus::Queued => ("Queued".to_string(), inactive_style()),
        DownloadStatus::InProgress { .. } => {
            ("In progress".to_string(), warning_style())
        }
        DownloadStatus::Paused { .. } => ("Paused".to_string(), info_style()),
        DownloadStatus::Completed => ("Completed".to_string(), success_style()),
        DownloadStatus::Failed(_) => ("Failed".to_string(), error_style()),
        DownloadStatus::TimedOut => ("Timed out".to_string(), error_style()),
    };
    lines.push(label_value_styled("Status", status_text, status_style));

    let save_path = expand_tilde(&download.download_directory)
        .to_string_lossy()
        .to_string();
    lines.push(label_value_styled("Save to", save_path, info_style()));

    if let Some(bitrate) = download.metadata.bitrate {
        lines.push(label_value("Bitrate", &format!("{bitrate} kbps")));
    }
    if let Some(length) = download.metadata.length_seconds {
        lines.push(label_value("Length", &format_duration(length)));
    }

    match &download.status {
        DownloadStatus::Queued => {
            lines.push(Line::from(""));
            let position_text = match download.queue_position {
                Some(p) => format!("#{p}"),
                None => "unknown".to_string(),
            };
            lines.push(label_value("Queue position", &position_text));

            if let Some(slots) = download.metadata.peer_free_slots {
                let (slot_text, slot_style) = if slots > 0 {
                    (format!("{slots} available"), success_style())
                } else {
                    ("all busy".to_string(), inactive_style())
                };
                lines.push(label_value_styled(
                    "Free slots",
                    slot_text,
                    slot_style,
                ));
            }

            if let Some(speed) = download.metadata.peer_upload_speed {
                lines.push(label_value(
                    "Upload speed",
                    &format_speed(f64::from(speed)),
                ));
            }
        }
        DownloadStatus::InProgress {
            bytes_downloaded,
            total_bytes,
            speed_bytes_per_sec,
        } => {
            lines.push(Line::from(""));
            push_transfer_progress(
                &mut lines,
                "Downloaded",
                *bytes_downloaded,
                *total_bytes,
            );
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
            push_transfer_progress(
                &mut lines,
                "Downloaded",
                *bytes_downloaded,
                *total_bytes,
            );
        }
        DownloadStatus::Failed(reason) => {
            if let Some(reason) = reason {
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    format!("{:<LABEL_WIDTH$}", "Error"),
                    dimmed_style(),
                )));
                lines.push(Line::from(Span::styled(
                    reason.clone(),
                    error_style(),
                )));
            }
        }
        DownloadStatus::Completed | DownloadStatus::TimedOut => {}
    }

    lines
}

fn label_value(label: &str, value: &str) -> Line<'static> {
    label_value_styled(label, value.to_string(), primary_style())
}

fn label_value_styled(
    label: &str,
    value: String,
    value_style: Style,
) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{label:<LABEL_WIDTH$}"), dimmed_style()),
        Span::styled(value, value_style),
    ])
}

/// A progress bar plus a `<label>  <done> / <total>` line, shared by the
/// download ("Downloaded") and upload ("Sent") views.
fn push_transfer_progress(
    lines: &mut Vec<Line<'static>>,
    label: &str,
    transferred: u64,
    total_bytes: u64,
) {
    let (progress, percent) = if total_bytes > 0 {
        let ratio = transferred as f64 / total_bytes as f64;
        (ratio, (ratio * 100.0) as u8)
    } else {
        (0.0, 0)
    };
    lines.push(format_progress_bar(progress, 20, percent));
    lines.push(label_value(
        label,
        &format!(
            "{} / {}",
            format_bytes(transferred),
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
        format!("{hours}h {minutes:02}m {secs:02}s")
    } else if minutes > 0 {
        format!("{minutes}m {secs:02}s")
    } else {
        format!("{secs}s")
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

    fn download_with_status(
        status: DownloadStatus,
    ) -> soulseek_rs::types::Download {
        let (sender, _rx) = std::sync::mpsc::channel();
        soulseek_rs::types::Download {
            username: "peer".to_string(),
            filename: "song.mp3".to_string(),
            token: 1,
            size: 100,
            download_directory: "~/dl".to_string(),
            status,
            sender,
            queue_position: None,
            metadata: soulseek_rs::types::DownloadMetadata::default(),
        }
    }

    fn lines_to_text(lines: &[Line<'static>]) -> String {
        lines
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn failed_download_renders_its_error_reason() {
        let download = download_with_status(DownloadStatus::Failed(Some(
            "Peer disconnected".to_string(),
        )));
        let text = lines_to_text(&build_info_lines(&download));
        assert!(text.contains("Error"), "missing Error label: {text}");
        assert!(text.contains("Peer disconnected"), "missing reason: {text}");
    }

    #[test]
    fn failed_download_without_reason_shows_no_error_line() {
        let download = download_with_status(DownloadStatus::Failed(None));
        let text = lines_to_text(&build_info_lines(&download));
        assert!(text.contains("Failed"), "status should still show Failed");
        assert!(!text.contains("Error"), "unexpected Error line: {text}");
    }
}
