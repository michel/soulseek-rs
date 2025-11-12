use crate::models::FileDownloadState;
use crate::ui::{format_bytes_progress, format_speed, header_style, highlight_style};
use color_eyre::Result;
use ratatui::{
    crossterm::event::{self, poll, Event, KeyCode, KeyEvent, KeyEventKind},
    layout::{Constraint, Layout},
    style::{Color, Style},
    widgets::{Block, Borders, Cell, Paragraph, Row, StatefulWidget, Table, TableState},
    DefaultTerminal, Frame,
};
use soulseek_rs::DownloadStatus;
use std::{sync::mpsc::Receiver, time::Duration};

pub struct MultiDownloadProgress {
    downloads: Vec<FileDownloadState>,
    receivers: Vec<Option<Receiver<DownloadStatus>>>,
    list_state: TableState,
    max_concurrent: usize,
    active_count: usize,
    should_exit: bool,
}

impl MultiDownloadProgress {
    pub fn new(
        downloads: Vec<FileDownloadState>,
        receivers: Vec<Receiver<DownloadStatus>>,
        max_concurrent: usize,
    ) -> Self {
        let mut list_state = TableState::default();
        list_state.select(Some(0));

        let receivers = receivers.into_iter().map(Some).collect();

        Self {
            downloads,
            receivers,
            list_state,
            max_concurrent,
            active_count: 0,
            should_exit: false,
        }
    }

    pub fn run(&mut self, mut terminal: DefaultTerminal) -> Result<()> {
        // Start initial batch of downloads
        self.start_next_batch();

        loop {
            terminal.draw(|frame| self.render(frame))?;

            // Poll all receivers for status updates
            for i in 0..self.receivers.len() {
                if let Some(receiver) = &self.receivers[i] {
                    if let Ok(status) = receiver.try_recv() {
                        let was_active = !self.downloads[i].is_finished();
                        self.downloads[i].update_status(status);
                        let is_finished = self.downloads[i].is_finished();

                        // If download just finished, decrement active count and start next
                        if was_active && is_finished {
                            self.active_count =
                                self.active_count.saturating_sub(1);
                            self.start_next_batch();
                        }
                    }
                }
            }

            // Handle keyboard input
            if poll(Duration::from_millis(100))? {
                if let Event::Key(key) = event::read()? {
                    self.handle_key(key);
                }
            }

            // Exit when user cancels or all downloads finished
            if self.should_exit || self.all_finished() {
                break;
            }
        }

        Ok(())
    }

    fn start_next_batch(&mut self) {
        // Start queued downloads up to max_concurrent limit
        for i in 0..self.downloads.len() {
            if self.active_count >= self.max_concurrent {
                break;
            }

            if matches!(self.downloads[i].status, DownloadStatus::Queued) {
                self.active_count += 1;
                // Download will start automatically since receiver exists
            }
        }
    }

    fn all_finished(&self) -> bool {
        self.downloads.iter().all(|d| d.is_finished())
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

    fn select_previous(&mut self) {
        let i = match self.list_state.selected() {
            Some(i) if i > 0 => i - 1,
            _ => self.downloads.len().saturating_sub(1),
        };
        self.list_state.select(Some(i));
    }

    fn select_next(&mut self) {
        let i = match self.list_state.selected() {
            Some(i) if i < self.downloads.len().saturating_sub(1) => i + 1,
            _ => 0,
        };
        self.list_state.select(Some(i));
    }

    fn select_first(&mut self) {
        self.list_state.select(Some(0));
    }

    fn select_last(&mut self) {
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
        self.render_controls(frame, chunks[2]);
    }

    fn render_stats(&self, frame: &mut Frame, area: ratatui::layout::Rect) {
        let completed = self
            .downloads
            .iter()
            .filter(|d| matches!(d.status, DownloadStatus::Completed))
            .count();
        let failed = self
            .downloads
            .iter()
            .filter(|d| {
                matches!(
                    d.status,
                    DownloadStatus::Failed | DownloadStatus::TimedOut
                )
            })
            .count();
        let queued = self
            .downloads
            .iter()
            .filter(|d| matches!(d.status, DownloadStatus::Queued))
            .count();

        let total_downloaded: u64 =
            self.downloads.iter().map(|d| d.bytes_downloaded).sum();
        let total_size: u64 =
            self.downloads.iter().map(|d| d.total_bytes).sum();
        let overall_progress = if total_size > 0 {
            (total_downloaded as f64 / total_size as f64 * 100.0) as u8
        } else {
            0
        };
        let total_speed: f64 = self
            .downloads
            .iter()
            .filter(|d| matches!(d.status, DownloadStatus::InProgress { .. }))
            .map(|d| d.speed_bytes_per_sec)
            .sum();
        let speed_mb = total_speed / 1_048_576.0;

        let stats_text = format!(
            "Downloads: {} active, {} completed, {} failed, {} queued | Overall: {}% • {:.1} MB/s",
            self.active_count, completed, failed, queued, overall_progress, speed_mb
        );

        let stats_widget = Paragraph::new(stats_text)
            .block(Block::default().borders(Borders::ALL).title("Status"))
            .style(Style::default().fg(Color::Cyan));

        frame.render_widget(stats_widget, area);
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
            Cell::from("%"),
            Cell::from("Speed"),
        ])
        .style(header_style())
        .height(1);

        let rows: Vec<Row> = self
            .downloads
            .iter()
            .map(|download| {
                let status_icon = match download.status {
                    DownloadStatus::Queued => "⋯",
                    DownloadStatus::InProgress { .. } => "⧗",
                    DownloadStatus::Completed => "✓",
                    DownloadStatus::Failed => "✗",
                    DownloadStatus::TimedOut => "⏱",
                };

                let progress = if download.total_bytes > 0 {
                    download.bytes_downloaded as f64
                        / download.total_bytes as f64
                } else {
                    0.0
                };

                let bar_width = 20;
                let filled = (progress * bar_width as f64) as usize;
                let empty = bar_width - filled;
                let progress_bar =
                    format!("[{}{}]", "█".repeat(filled), "░".repeat(empty));

                let percent = (progress * 100.0) as u8;
                let percent_str = format!("{}%", percent);

                let size_str = format_bytes_progress(
                    download.bytes_downloaded,
                    download.total_bytes,
                );

                let speed_str = match download.status {
                    DownloadStatus::InProgress { .. } => {
                        format_speed(download.speed_bytes_per_sec)
                    }
                    _ => "-".to_string(),
                };

                let cells = vec![
                    Cell::from(status_icon),
                    Cell::from(download.filename.clone()),
                    Cell::from(download.username.clone()),
                    Cell::from(size_str),
                    Cell::from(progress_bar),
                    Cell::from(percent_str),
                    Cell::from(speed_str),
                ];

                let style = match download.status {
                    DownloadStatus::Queued => Style::default().fg(Color::Gray),
                    DownloadStatus::InProgress { .. } => {
                        Style::default().fg(Color::Yellow)
                    }
                    DownloadStatus::Completed => {
                        Style::default().fg(Color::Green)
                    }
                    DownloadStatus::Failed | DownloadStatus::TimedOut => {
                        Style::default().fg(Color::Red)
                    }
                };

                Row::new(cells).style(style).height(1)
            })
            .collect();

        let widths = [
            Constraint::Length(3),
            Constraint::Min(30),
            Constraint::Length(15),
            Constraint::Length(20),
            Constraint::Length(22),
            Constraint::Length(5),
            Constraint::Length(12),
        ];

        let table = Table::new(rows, widths)
            .header(header)
            .block(Block::default().borders(Borders::ALL).title("Files"))
            .column_spacing(1)
            .row_highlight_style(highlight_style())
            .highlight_symbol("> ");

        StatefulWidget::render(
            table,
            area,
            frame.buffer_mut(),
            &mut self.list_state,
        );
    }

    fn render_controls(&self, frame: &mut Frame, area: ratatui::layout::Rect) {
        let controls_text =
            "↑↓/jk: scroll • Home/End: jump • Esc/q: cancel all";

        let controls_widget = Paragraph::new(controls_text)
            .block(Block::default().borders(Borders::ALL).title("Controls"))
            .style(Style::default().fg(Color::DarkGray));

        frame.render_widget(controls_widget, area);
    }
}

pub fn show_multi_download_progress(
    downloads: Vec<FileDownloadState>,
    receivers: Vec<Receiver<DownloadStatus>>,
    max_concurrent: usize,
) -> Result<()> {
    soulseek_rs::utils::logger::enable_buffering();

    let terminal = ratatui::init();
    let mut progress =
        MultiDownloadProgress::new(downloads, receivers, max_concurrent);
    let result = progress.run(terminal);
    ratatui::restore();

    soulseek_rs::utils::logger::flush_buffered_logs();

    result
}
