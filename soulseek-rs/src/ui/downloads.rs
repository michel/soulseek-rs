use crate::models::DownloadEntry;
use crate::ui::{
    BYTES_PER_MB, HIGHLIGHT_SYMBOL, accent_style, body_style, dimmed_style,
    download_status_glyph, error_style, format_bytes_progress,
    format_progress_bar, format_shortcuts_styled, format_speed, header_style,
    highlight_style, info_style, pane_block, plain_title, primary_style,
    success_style, warning_style,
};
use color_eyre::Result;
use ratatui::{
    DefaultTerminal, Frame,
    crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, poll},
    layout::{Alignment, Constraint, Layout},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{
        Cell, HighlightSpacing, Paragraph, Row, StatefulWidget, Table,
        TableState,
    },
};
use soulseek_rs::{Client, DownloadStatus};
use std::{
    sync::{Arc, mpsc, mpsc::Receiver},
    thread,
    time::Duration,
};

const VERSION: &str = env!("CARGO_PKG_VERSION");

pub struct MultiDownloadProgress {
    downloads: Vec<DownloadEntry>,
    receiver_channel:
        Receiver<(soulseek_rs::types::Download, Receiver<DownloadStatus>)>,
    list_state: TableState,
    max_concurrent: usize,
    active_count: usize,
    should_exit: bool,
    queuing_status: String,
}

impl MultiDownloadProgress {
    pub fn new(
        receiver_channel: Receiver<(
            soulseek_rs::types::Download,
            Receiver<DownloadStatus>,
        )>,
        max_concurrent: usize,
    ) -> Self {
        let mut list_state = TableState::default();
        list_state.select(Some(0));

        Self {
            downloads: Vec::new(),
            receiver_channel,
            list_state,
            max_concurrent,
            active_count: 0,
            should_exit: false,
            queuing_status: String::from("Queuing downloads..."),
        }
    }

    pub fn run(&mut self, mut terminal: DefaultTerminal) -> Result<()> {
        self.start_next_batch();

        loop {
            terminal.draw(|frame| self.render(frame))?;

            while let Ok((download, receiver)) =
                self.receiver_channel.try_recv()
            {
                self.downloads.push(DownloadEntry {
                    download,
                    receiver: Some(receiver),
                });
                self.queuing_status =
                    format!("{} downloads queued", self.downloads.len());
            }

            // Poll all receivers for status updates
            let mut need_start_next = false;
            for download_entry in &mut self.downloads {
                if let Some(receiver) = &download_entry.receiver
                    && let Ok(status) = receiver.try_recv()
                {
                    let was_active = !download_entry.download.is_finished();
                    download_entry.download.status = status;
                    let is_finished = download_entry.download.is_finished();

                    // If download just finished, decrement active count
                    if was_active && is_finished {
                        self.active_count = self.active_count.saturating_sub(1);
                        need_start_next = true;
                    }
                }
            }

            if need_start_next {
                self.start_next_batch();
            }

            if poll(Duration::from_millis(100))?
                && let Event::Key(key) = event::read()?
            {
                self.handle_key(key);
            }

            // Exit when user cancels or all downloads finished
            if self.should_exit || self.all_finished() {
                break;
            }
        }

        Ok(())
    }

    fn start_next_batch(&mut self) {
        for download_entry in &self.downloads {
            if self.active_count >= self.max_concurrent {
                break;
            }

            // Only start if download is queued AND receiver is available
            if matches!(download_entry.download.status, DownloadStatus::Queued)
                && download_entry.receiver.is_some()
            {
                self.active_count += 1;
                // Download will start automatically since receiver exists
            }
        }
    }

    fn all_finished(&self) -> bool {
        self.downloads.iter().all(|d| d.download.is_finished())
    }

    fn handle_key(&mut self, key: KeyEvent) {
        if key.kind != KeyEventKind::Press {
            return;
        }

        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.select_previous(),
            KeyCode::Down | KeyCode::Char('j') => self.select_next(),
            KeyCode::Home | KeyCode::Char('g') => self.select_first(),
            KeyCode::End | KeyCode::Char('G') => self.select_last(),
            KeyCode::Esc | KeyCode::Char('q') => {
                self.should_exit = true;
            }
            _ => {}
        }
    }

    const fn select_previous(&mut self) {
        let i = match self.list_state.selected() {
            Some(i) if i > 0 => i - 1,
            _ => self.downloads.len().saturating_sub(1),
        };
        self.list_state.select(Some(i));
    }

    const fn select_next(&mut self) {
        let i = match self.list_state.selected() {
            Some(i) if i < self.downloads.len().saturating_sub(1) => i + 1,
            _ => 0,
        };
        self.list_state.select(Some(i));
    }

    const fn select_first(&mut self) {
        self.list_state.select(Some(0));
    }

    const fn select_last(&mut self) {
        if !self.downloads.is_empty() {
            self.list_state.select(Some(self.downloads.len() - 1));
        }
    }

    fn render(&mut self, frame: &mut Frame) {
        let area = frame.area();

        let chunks = Layout::vertical([
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(3),
        ])
        .split(area);

        // Top: Overall stats
        self.render_stats(frame, chunks[0]);

        // Middle: List of downloads
        self.render_downloads_list(frame, chunks[1]);

        // Bottom: Controls
        Self::render_controls(frame, chunks[2]);
    }

    fn render_stats(&self, frame: &mut Frame, area: ratatui::layout::Rect) {
        render_download_stats(frame, area, &self.downloads, self.active_count);
    }

    fn render_downloads_list(
        &mut self,
        frame: &mut Frame,
        area: ratatui::layout::Rect,
    ) {
        let header = Row::new(vec![
            Cell::from("St"),
            Cell::from("Filename"),
            Cell::from("Username"),
            Cell::from("Size"),
            Cell::from("Progress"),
            Cell::from("Speed"),
        ])
        .style(header_style())
        .height(1);

        let rows: Vec<Row> = self
            .downloads
            .iter()
            .map(|download_entry| {
                let download = &download_entry.download;
                let (status_icon, status_style) =
                    download_status_glyph(&download.status);

                let progress = if download.size > 0 {
                    download.bytes_downloaded() as f64 / download.size as f64
                } else {
                    0.0
                };

                let percent = (progress * 100.0) as u8;
                let bar_width = 20;
                let progress_bar =
                    format_progress_bar(progress, bar_width, percent);

                let size_str = format_bytes_progress(
                    download.bytes_downloaded(),
                    download.size,
                );

                let speed_str = match download.status {
                    DownloadStatus::InProgress { .. } => {
                        format_speed(download.speed_bytes_per_sec())
                    }
                    _ => "-".to_string(),
                };

                let cells = vec![
                    Cell::from(status_icon).style(status_style),
                    Cell::from(download.filename.clone()).style(body_style()),
                    Cell::from(download.username.clone()).style(info_style()),
                    Cell::from(size_str).style(warning_style()),
                    Cell::from(progress_bar),
                    Cell::from(speed_str).style(dimmed_style()),
                ];

                Row::new(cells).height(1)
            })
            .collect();

        let widths = [
            Constraint::Length(3),
            Constraint::Min(30),
            Constraint::Length(15),
            Constraint::Length(20),
            Constraint::Length(28),
            Constraint::Length(12),
        ];

        let table = Table::new(rows, widths)
            .header(header)
            .block(pane_block(true).title(plain_title("Downloads", true)))
            .column_spacing(1)
            .row_highlight_style(highlight_style())
            .highlight_symbol(HIGHLIGHT_SYMBOL)
            .highlight_spacing(HighlightSpacing::Always);

        StatefulWidget::render(
            table,
            area,
            frame.buffer_mut(),
            &mut self.list_state,
        );
    }

    fn render_controls(frame: &mut Frame, area: ratatui::layout::Rect) {
        let controls_line = format_shortcuts_styled(&[
            ("↑↓/jk", "scroll"),
            ("Home/End", "jump"),
            ("Esc/q", "cancel all"),
        ]);

        let controls_widget = Paragraph::new(controls_line)
            .block(pane_block(false).title(plain_title("Controls", false)));

        frame.render_widget(controls_widget, area);
    }
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

    let total_downloaded: u64 = downloads
        .iter()
        .map(|d| d.download.bytes_downloaded())
        .sum();
    let total_size: u64 = downloads.iter().map(|d| d.download.size).sum();
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
    let total_speed: f64 = downloads
        .iter()
        .filter(|d| {
            matches!(d.download.status, DownloadStatus::InProgress { .. })
        })
        .map(|d| d.download.speed_bytes_per_sec())
        .sum();
    let speed_mb = (total_speed / BYTES_PER_MB * 100.0).round() / 100.0;

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

pub fn show_multi_download_progress(
    terminal: DefaultTerminal,
    client: Arc<Client>,
    selected_files: Vec<(String, String, u64)>,
    download_dir: String,
    max_concurrent: usize,
) -> Result<()> {
    soulseek_rs::utils::logger::enable_buffering();

    let (tx, rx) = mpsc::channel();

    // Spawn background thread to initialize downloads
    let init_client = client;
    thread::spawn(move || {
        for (filename, username, size) in selected_files {
            // Initiate download
            match init_client.download(
                filename.clone(),
                username.clone(),
                size,
                download_dir.clone(),
            ) {
                Ok((download, receiver)) => {
                    let _ = tx.send((download, receiver));
                }
                Err(e) => {
                    soulseek_rs::warn!(
                        "Failed to start download for {filename}: {e}"
                    );
                }
            }
        }
    });

    let mut progress = MultiDownloadProgress::new(rx, max_concurrent);
    let result = progress.run(terminal);
    ratatui::restore();

    soulseek_rs::utils::logger::flush_buffered_logs();

    result
}
