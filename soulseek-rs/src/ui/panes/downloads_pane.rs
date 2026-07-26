use crate::models::DownloadEntry;
use crate::ui::{
    GLYPH_ACTIVE, GLYPH_DONE, GLYPH_FAILED, GLYPH_QUEUED, HIGHLIGHT_SYMBOL,
    accent_style, body_style, dimmed_style, download_status_glyph, error_style,
    format_bytes, format_speed, header_style, inactive_style, info_style,
    pane_block, pane_title, primary_style, row_highlight_style, success_style,
    warning_style,
};
use ratatui::{
    Frame,
    layout::Rect,
    widgets::{Cell, HighlightSpacing, Paragraph, Row, Table, TableState},
};
use soulseek_rs::DownloadStatus;
use soulseek_rs::types::{UploadInfo, UploadStatus};

pub fn render_downloads_pane(
    frame: &mut Frame,
    area: Rect,
    downloads: &[DownloadEntry],
    uploads: &[UploadInfo],
    table_state: &mut TableState,
    focused: bool,
) {
    if downloads.is_empty() && uploads.is_empty() {
        let empty_block = pane_block(focused).title(pane_title(
            "3",
            "Downloads/Uploads",
            focused,
        ));

        let paragraph = Paragraph::new(
            "No transfers. Select files in [2] Results and press Enter.",
        )
        .style(dimmed_style())
        .block(empty_block);
        frame.render_widget(paragraph, area);
        return;
    }

    let header = Row::new(vec![
        Cell::from("Status").style(header_style()),
        Cell::from("Filename").style(header_style()),
        Cell::from("User").style(header_style()),
        Cell::from("Progress").style(header_style()),
        Cell::from("Speed").style(header_style()),
    ])
    .height(1);

    let mut rows: Vec<Row> = downloads
        .iter()
        .map(|download_entry| {
            let download = &download_entry.download;
            let (status_icon, status_style) =
                download_status_glyph(&download.status);

            let progress_text = match &download.status {
                DownloadStatus::Queued => "Queued".to_string(),
                DownloadStatus::InProgress { .. } => {
                    let percent = if download.size > 0 {
                        (download.bytes_downloaded() as f64
                            / download.size as f64
                            * 100.0) as u8
                    } else {
                        0
                    };
                    format!(
                        "{}/{} ({}%)",
                        format_bytes(download.bytes_downloaded()),
                        format_bytes(download.size),
                        percent
                    )
                }
                DownloadStatus::Paused { .. } => {
                    let percent = if download.size > 0 {
                        (download.bytes_downloaded() as f64
                            / download.size as f64
                            * 100.0) as u8
                    } else {
                        0
                    };
                    format!(
                        "Paused {}/{} ({}%)",
                        format_bytes(download.bytes_downloaded()),
                        format_bytes(download.size),
                        percent
                    )
                }
                DownloadStatus::Completed => "Completed".to_string(),
                DownloadStatus::Failed(_) => "Failed".to_string(),
                DownloadStatus::TimedOut => "Timed out".to_string(),
            };

            let speed_text = match &download.status {
                DownloadStatus::InProgress { .. } => {
                    format_speed(download.speed_bytes_per_sec())
                }
                _ => "-".to_string(),
            };

            let progress_style = match &download.status {
                DownloadStatus::Completed => success_style(),
                DownloadStatus::Failed(_) | DownloadStatus::TimedOut => {
                    error_style()
                }
                DownloadStatus::Queued => inactive_style(),
                _ => primary_style(),
            };

            Row::new(vec![
                Cell::from(status_icon).style(status_style),
                Cell::from(download.filename.clone()).style(body_style()),
                Cell::from(download.username.clone()).style(info_style()),
                Cell::from(progress_text).style(progress_style),
                Cell::from(speed_text).style(warning_style()),
            ])
        })
        .collect();

    rows.extend(uploads.iter().map(|upload| {
        let (status_icon, status_style) = match &upload.status {
            UploadStatus::Queued(_) => (GLYPH_QUEUED, warning_style()),
            UploadStatus::InProgress => (GLYPH_ACTIVE, accent_style()),
            UploadStatus::Completed => (GLYPH_DONE, success_style()),
            UploadStatus::Cancelled => (GLYPH_FAILED, inactive_style()),
            UploadStatus::Failed(_) => (GLYPH_FAILED, error_style()),
        };
        let progress_text = match &upload.status {
            UploadStatus::Queued(place) => format!("Queued #{place}"),
            UploadStatus::InProgress => {
                let percent = if upload.size > 0 {
                    (upload.bytes_sent as f64 / upload.size as f64 * 100.0)
                        as u8
                } else {
                    0
                };
                format!(
                    "{}/{} ({}%)",
                    format_bytes(upload.bytes_sent),
                    format_bytes(upload.size),
                    percent
                )
            }
            UploadStatus::Completed => "Completed".to_string(),
            UploadStatus::Cancelled => "Cancelled".to_string(),
            UploadStatus::Failed(_) => "Failed".to_string(),
        };
        // Same rule as a download: a rate only while the transfer is running.
        let speed_text = match &upload.status {
            UploadStatus::InProgress => {
                format_speed(upload.speed_bytes_per_sec)
            }
            _ => "-".to_string(),
        };
        let basename = upload
            .filename
            .rsplit(['\\', '/'])
            .next()
            .unwrap_or(&upload.filename);
        Row::new(vec![
            Cell::from(format!("↑ {status_icon}")).style(status_style),
            Cell::from(basename.to_string()).style(body_style()),
            Cell::from(upload.username.clone()).style(info_style()),
            Cell::from(progress_text).style(primary_style()),
            Cell::from(speed_text).style(warning_style()),
        ])
    }));

    let widths = [
        ratatui::layout::Constraint::Length(8),
        ratatui::layout::Constraint::Fill(2),
        ratatui::layout::Constraint::Length(15),
        ratatui::layout::Constraint::Length(25),
        ratatui::layout::Constraint::Length(12),
    ];

    let table = Table::new(rows, widths)
        .header(header)
        .row_highlight_style(row_highlight_style())
        .highlight_symbol(HIGHLIGHT_SYMBOL)
        .highlight_spacing(HighlightSpacing::Always)
        .block(pane_block(focused).title(pane_title(
            "3",
            "Downloads/Uploads",
            focused,
        )));

    frame.render_stateful_widget(table, area, table_state);
}
