use crate::models::DownloadEntry;
use crate::ui::{
    BYTES_PER_MB, accent_style, dimmed_style, error_style,
    format_bytes_progress, format_progress_bar, info_style, pane_block,
    plain_title, primary_style, success_style, warning_style,
};
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};
use soulseek_rs::DownloadStatus;

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Bytes transferred, bytes expected and combined speed across the downloads
/// that are actually running — or `None` when none are, so the caller can hide
/// the progress readout entirely.
///
/// Only `InProgress` downloads count. A queued download has not started and a
/// failed one never will, so adding their size to the total just pins the bar
/// near zero for the rest of the session.
fn active_progress(downloads: &[DownloadEntry]) -> Option<(u64, u64, f64)> {
    let mut transferred = 0;
    let mut expected = 0;
    let mut speed = 0.0;
    let mut running = false;

    for entry in downloads {
        if !matches!(entry.download.status, DownloadStatus::InProgress { .. }) {
            continue;
        }
        running = true;
        transferred += entry.download.bytes_downloaded();
        expected += entry.download.size;
        speed += entry.download.speed_bytes_per_sec();
    }

    running.then_some((transferred, expected, speed))
}

/// Renders download statistics in a reusable way
pub fn render_download_stats(
    frame: &mut Frame,
    area: ratatui::layout::Rect,
    downloads: &[DownloadEntry],
    active_count: usize,
) {
    let completed = downloads
        .iter()
        .filter(|d| matches!(d.download.status, DownloadStatus::Completed))
        .count();
    let failed = downloads
        .iter()
        .filter(|d| {
            matches!(
                d.download.status,
                DownloadStatus::Failed(_) | DownloadStatus::TimedOut
            )
        })
        .count();
    let queued = downloads
        .iter()
        .filter(|d| matches!(d.download.status, DownloadStatus::Queued))
        .count();
    let paused = downloads
        .iter()
        .filter(|d| matches!(d.download.status, DownloadStatus::Paused { .. }))
        .count();

    let active = active_progress(downloads);

    let block = pane_block(false).title(plain_title("Status", false));

    let inner_area = block.inner(area);
    frame.render_widget(block, area);

    // Split into two equal 50% containers
    let chunks = Layout::horizontal([
        Constraint::Percentage(50),
        Constraint::Percentage(50),
    ])
    .split(inner_area);

    let count = |value: usize, style: Style| {
        Span::styled(value.to_string(), style.add_modifier(Modifier::BOLD))
    };
    let stats_line = Line::from(vec![
        Span::styled("soulseek-rs", primary_style()),
        Span::styled(format!(" 🦀 v{VERSION}"), dimmed_style()),
        Span::styled("  Downloads: ", dimmed_style()),
        count(active_count, accent_style()),
        Span::styled(" active, ", dimmed_style()),
        count(completed, success_style()),
        Span::styled(" completed, ", dimmed_style()),
        count(failed, error_style()),
        Span::styled(" failed, ", dimmed_style()),
        count(queued, warning_style()),
        Span::styled(" queued, ", dimmed_style()),
        count(paused, info_style()),
        Span::styled(" paused", dimmed_style()),
    ]);

    let stats_paragraph = Paragraph::new(stats_line);
    frame.render_widget(stats_paragraph, chunks[0]);

    // Nothing is transferring, so there is no progress to draw: leave the
    // right-hand half blank instead of showing an idle 0 B / 0 B bar.
    let Some((total_downloaded, total_size, total_speed)) = active else {
        return;
    };
    let overall_progress = if total_size > 0 {
        (total_downloaded as f64 / total_size as f64 * 100.0) as u8
    } else {
        0
    };
    let progress_ratio = if total_size > 0 {
        total_downloaded as f64 / total_size as f64
    } else {
        0.0
    };
    let speed_mb = (total_speed / BYTES_PER_MB * 100.0).round() / 100.0;

    let right_width = chunks[1].width as usize;
    let bar_width = right_width.saturating_sub(42).max(10);
    let progress_bar =
        format_progress_bar(progress_ratio, bar_width, overall_progress);
    let data_str = format_bytes_progress(total_downloaded, total_size);

    let mut spans: Vec<Span> = Vec::new();

    spans.extend(data_str.spans);
    spans.push(Span::styled(" · ", dimmed_style()));
    spans.push(Span::styled(
        format!("{speed_mb}"),
        warning_style().add_modifier(Modifier::BOLD),
    ));
    spans.push(Span::styled(" MB/s", dimmed_style()));
    spans.push(Span::styled(" · ", dimmed_style()));
    spans.extend(progress_bar.spans);

    let progress_line = Line::from(spans);
    let progress_paragraph =
        Paragraph::new(progress_line).alignment(Alignment::Right);
    frame.render_widget(progress_paragraph, chunks[1]);
}

#[cfg(test)]
mod tests {
    use super::{DownloadEntry, DownloadStatus, active_progress};
    use soulseek_rs::types::{Download, DownloadMetadata};

    fn entry(size: u64, status: DownloadStatus) -> DownloadEntry {
        let (sender, _receiver) = std::sync::mpsc::channel();
        DownloadEntry {
            download: Download {
                username: "peer".to_string(),
                filename: "song.flac".to_string(),
                token: 1,
                size,
                download_directory: "/tmp".to_string(),
                status,
                sender,
                queue_position: None,
                metadata: DownloadMetadata::default(),
            },
            receiver: None,
        }
    }

    #[test]
    fn only_running_downloads_count_towards_the_bar() {
        let downloads = vec![
            entry(
                100,
                DownloadStatus::InProgress {
                    bytes_downloaded: 40,
                    total_bytes: 100,
                    speed_bytes_per_sec: 512.0,
                },
            ),
            // None of these may reach the bar: a queued file has not started,
            // and a failed or timed-out one never will. Counting their size
            // would drag the bar down for the rest of the session.
            entry(9_000, DownloadStatus::Queued),
            entry(9_000, DownloadStatus::Failed(None)),
            entry(9_000, DownloadStatus::TimedOut),
        ];

        let (transferred, expected, speed) =
            active_progress(&downloads).expect("one download is running");
        assert_eq!(transferred, 40);
        assert_eq!(expected, 100, "queued and failed sizes must be excluded");
        assert!((speed - 512.0).abs() < f64::EPSILON);
    }

    #[test]
    fn nothing_running_hides_the_bar() {
        assert!(active_progress(&[]).is_none(), "no downloads at all");

        // Completed and paused transfers are not processing either, so the
        // readout stays hidden rather than showing a stale bar.
        let idle = vec![
            entry(100, DownloadStatus::Completed),
            entry(100, DownloadStatus::Queued),
            entry(
                100,
                DownloadStatus::Paused {
                    bytes_downloaded: 50,
                    total_bytes: 100,
                },
            ),
        ];
        assert!(active_progress(&idle).is_none());
    }
}
