use crate::models::DownloadEntry;
use crate::ui::{
    HIGHLIGHT_SYMBOL, border_style, border_type, error_style, format_bytes,
    format_speed, header_style, highlight_style, inactive_style, info_style,
    success_style, warning_style,
};
use ratatui::{
    Frame,
    layout::Rect,
    widgets::{
        Block, Borders, Cell, HighlightSpacing, Paragraph, Row, Table,
        TableState,
    },
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
        let empty_block = Block::default()
            .borders(Borders::ALL)
            .border_style(border_style(focused))
            .border_type(border_type(focused))
            .title("[3] Downloads/Uploads");

        let paragraph = Paragraph::new(
            "No transfers. Select files from Results and press Enter.",
        )
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
            let (status_icon, status_style) = match &download.status {
                DownloadStatus::Queued => ("⋯", inactive_style()),
                DownloadStatus::InProgress { .. } => ("⧗", warning_style()),
                DownloadStatus::Paused { .. } => ("⏸", info_style()),
                DownloadStatus::Completed => ("✓", success_style()),
                DownloadStatus::Failed(_) => ("✗", error_style()),
                DownloadStatus::TimedOut => ("⏱", error_style()),
            };

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

            Row::new(vec![
                Cell::from(status_icon).style(status_style),
                Cell::from(download.filename.clone()),
                Cell::from(download.username.clone()),
                Cell::from(progress_text),
                Cell::from(speed_text),
            ])
        })
        .collect();

    rows.extend(uploads.iter().map(|upload| {
        let (status_icon, status_style) = match &upload.status {
            UploadStatus::InProgress => ("⧗", warning_style()),
            UploadStatus::Completed => ("✓", success_style()),
            UploadStatus::Cancelled => ("✗", inactive_style()),
            UploadStatus::Failed(_) => ("✗", error_style()),
        };
        let progress_text = match &upload.status {
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
            UploadStatus::Completed => format_bytes(upload.size),
            UploadStatus::Cancelled => "Cancelled".to_string(),
            UploadStatus::Failed(_) => "Failed".to_string(),
        };
        let basename = upload
            .filename
            .rsplit(['\\', '/'])
            .next()
            .unwrap_or(&upload.filename);
        Row::new(vec![
            Cell::from(format!("↑ {status_icon}")).style(status_style),
            Cell::from(basename.to_string()),
            Cell::from(upload.username.clone()),
            Cell::from(progress_text),
            Cell::from(String::new()),
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
        .row_highlight_style(highlight_style())
        .highlight_symbol(HIGHLIGHT_SYMBOL)
        .highlight_spacing(HighlightSpacing::Always)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(border_style(focused))
                .border_type(border_type(focused))
                .title("[3] Downloads/Uploads"),
        );

    frame.render_stateful_widget(table, area, table_state);
}
