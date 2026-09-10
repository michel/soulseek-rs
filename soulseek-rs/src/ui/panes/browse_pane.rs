use crate::models::{BrowseState, BrowseStatus, BrowseTabs, DownloadEntry};
use crate::ui::{
    HIGHLIGHT_SYMBOL, body_style, dimmed_style, download_status_glyph,
    error_style, filter_title, format_bytes, get_spinner_char, highlight_style,
    page_window, pane_block, primary_style, render_page, row_highlight_style,
    warning_style,
};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::Modifier,
    text::{Line, Span},
    widgets::{Cell, HighlightSpacing, Paragraph, Row, Table, TableState},
};
use soulseek_rs::DownloadStatus;
use soulseek_rs::types::Download;
use std::collections::HashMap;

/// Render the browse popup: a tab bar of browsed users (when more than one is
/// open) above the active user's collapsible shared-file tree.
///
/// `downloads` is the window's transfer list; a file row whose download is
/// there shows the same status glyph the Downloads pane gives it, and a
/// folder row summarises the downloads of the files queued below it.
pub fn render_browse_pane(
    frame: &mut Frame,
    area: Rect,
    tabs: &BrowseTabs,
    table_state: &mut TableState,
    spinner_state: usize,
    downloads: &[DownloadEntry],
) {
    let Some(active) = tabs.active_tab() else {
        return;
    };

    // With multiple users open, reserve the top row for a tab bar.
    let tree_area = if tabs.tabs.len() > 1 {
        let chunks =
            Layout::vertical([Constraint::Length(1), Constraint::Fill(1)])
                .split(area);
        render_browse_tabs(frame, chunks[0], tabs);
        chunks[1]
    } else {
        area
    };

    render_browse_one(
        frame,
        tree_area,
        active,
        table_state,
        spinner_state,
        downloads,
    );
}

fn render_browse_tabs(frame: &mut Frame, area: Rect, tabs: &BrowseTabs) {
    let mut spans: Vec<Span> = Vec::new();
    for (i, tab) in tabs.tabs.iter().enumerate() {
        let style = if i == tabs.active {
            highlight_style().add_modifier(Modifier::BOLD)
        } else {
            dimmed_style()
        };
        spans.push(Span::styled(format!(" {} ", tab.username), style));
        spans.push(Span::raw(" "));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// Render one browsed user's shared-file tree.
fn render_browse_one(
    frame: &mut Frame,
    area: Rect,
    browse: &BrowseState,
    table_state: &mut TableState,
    spinner_state: usize,
    downloads: &[DownloadEntry],
) {
    let title = match browse.status {
        BrowseStatus::Loaded => {
            let shown =
                browse.rows().iter().filter(|row| !row.is_folder).count();
            filter_title(
                &format!(
                    "Browse {} — {shown}/{} files",
                    browse.username, browse.file_count
                ),
                browse.filter(),
                browse.filtering,
                &format!(
                    " Browse {} — {} files, {} folders  (Enter/d: download, /: filter, Tab: user, w: close, Esc: hide) ",
                    browse.username, browse.file_count, browse.folder_count
                ),
            )
        }
        _ => format!(" Browse {} ", browse.username),
    };
    let block = pane_block(true).title(title);

    match browse.status {
        BrowseStatus::Loading => {
            let text = format!(
                "{} Requesting shared files from {}…",
                get_spinner_char(spinner_state),
                browse.username
            );
            frame.render_widget(
                Paragraph::new(text).style(dimmed_style()).block(block),
                area,
            );
        }
        BrowseStatus::Empty => {
            let text = format!("{} is not sharing any files.", browse.username);
            frame.render_widget(
                Paragraph::new(text).style(dimmed_style()).block(block),
                area,
            );
        }
        BrowseStatus::TimedOut => {
            let text = vec![
                Line::styled(
                    format!("Couldn't reach {}.", browse.username),
                    error_style(),
                ),
                Line::raw(""),
                Line::styled(
                    "They may be offline, or their connection can't be \
                     reached (both of you may be behind a router/firewall).",
                    body_style(),
                ),
                Line::styled("Press r to try again.", dimmed_style()),
            ];
            frame.render_widget(Paragraph::new(text).block(block), area);
        }
        BrowseStatus::Loaded
            if browse.rows().is_empty() && !browse.filter().is_empty() =>
        {
            let text = format!("Nothing here matches '{}'.", browse.filter());
            frame.render_widget(
                Paragraph::new(text).style(dimmed_style()).block(block),
                area,
            );
        }
        BrowseStatus::Loaded => {
            let all = browse.rows();
            // Queued files carry the download's full share path, so a row
            // matches a transfer of this user by path alone.
            let transfers: Vec<&Download> = downloads
                .iter()
                .filter(|entry| entry.download.username == browse.username)
                .map(|entry| &entry.download)
                .collect();
            let status_of: HashMap<&str, &DownloadStatus> = transfers
                .iter()
                .map(|download| (download.filename.as_str(), &download.status))
                .collect();
            table_state.select(Some(browse.selected_row));
            let window = page_window(table_state, all.len(), area, 0);
            let start = window.start;
            let rows: Vec<Row> = all[window]
                .iter()
                .map(|row| {
                    let indent = "  ".repeat(row.depth);
                    let (label, size) = if row.is_folder {
                        // A folder's download is its files' downloads, so
                        // the folder reports the one status that describes
                        // the files queued below it.
                        let below: Vec<&Download> = transfers
                            .iter()
                            .copied()
                            .filter(|d| under_folder(&d.filename, &row.path))
                            .collect();
                        let transfer = folder_status(&below);
                        let marker = transfer.map_or_else(
                            || Span::raw(""),
                            |status| {
                                let (glyph, style) =
                                    download_status_glyph(status);
                                Span::styled(format!("{glyph} "), style)
                            },
                        );
                        let open = if row.expanded { "▾" } else { "▸" };
                        let size = if matches!(
                            transfer,
                            Some(DownloadStatus::InProgress { .. })
                        ) {
                            format!("{}%", folder_percent(&below))
                        } else {
                            String::new()
                        };
                        (
                            Cell::from(Line::from(vec![
                                Span::raw(indent),
                                marker,
                                Span::styled(
                                    format!("{open} {}", row.name),
                                    primary_style()
                                        .add_modifier(Modifier::BOLD),
                                ),
                            ])),
                            Cell::from(size).style(warning_style()),
                        )
                    } else {
                        let status = status_of.get(row.path.as_str()).copied();
                        // The two spaces before a file's name become the
                        // transfer's glyph while its download lives in the
                        // list: ⋯ queued, ↯ active, ⏸ paused, ✓ done.
                        let marker = status.map_or_else(
                            || Span::raw("  "),
                            |status| {
                                let (glyph, style) =
                                    download_status_glyph(status);
                                Span::styled(format!("{glyph} "), style)
                            },
                        );
                        let plain_size =
                            row.size.map(format_bytes).unwrap_or_default();
                        let size_text = match status {
                            Some(DownloadStatus::InProgress {
                                bytes_downloaded,
                                total_bytes,
                                ..
                            }) if *total_bytes > 0 => {
                                let percent =
                                    bytes_downloaded * 100 / total_bytes;
                                format!("{plain_size} {percent}%")
                            }
                            _ => plain_size,
                        };
                        (
                            Cell::from(Line::from(vec![
                                Span::raw(indent),
                                marker,
                                Span::styled(row.name.clone(), body_style()),
                            ])),
                            Cell::from(size_text).style(warning_style()),
                        )
                    };
                    Row::new(vec![label, size])
                })
                .collect();

            let table =
                Table::new(rows, [Constraint::Fill(1), Constraint::Length(12)])
                    .row_highlight_style(row_highlight_style())
                    .highlight_symbol(HIGHLIGHT_SYMBOL)
                    .highlight_spacing(HighlightSpacing::Always)
                    .block(block);

            render_page(frame, table, area, table_state, start);
        }
    }
}

/// Whether `file` is one of the folder's own files in a share path.
fn under_folder(file: &str, folder: &str) -> bool {
    file.strip_prefix(folder)
        .is_some_and(|rest| rest.starts_with('\\'))
}

/// How loudly a status speaks for a whole folder: moving work first,
/// then paused, then queued, then a failure worth surfacing; done only
/// says so once every file is.
const fn status_rank(status: &DownloadStatus) -> u8 {
    match status {
        DownloadStatus::InProgress { .. } => 5,
        DownloadStatus::Paused { .. } => 4,
        DownloadStatus::Queued => 3,
        DownloadStatus::Failed(_) | DownloadStatus::TimedOut => 2,
        DownloadStatus::Cancelled => 1,
        DownloadStatus::Completed => 0,
    }
}

/// The status that describes a folder's download: the loudest of its
/// files', or nothing while none of them is in the transfer list.
fn folder_status<'a>(files: &[&'a Download]) -> Option<&'a DownloadStatus> {
    files
        .iter()
        .map(|download| &download.status)
        .max_by_key(|status| status_rank(status))
}

/// How far a folder's queued bytes have come, whole percents.
fn folder_percent(files: &[&Download]) -> u8 {
    let bytes: u64 = files.iter().map(|d| d.bytes_downloaded()).sum();
    let total: u64 = files.iter().map(|d| d.size).sum();
    bytes.saturating_mul(100).checked_div(total).unwrap_or(0) as u8
}

#[cfg(test)]
mod tests {
    use super::render_browse_pane;
    use crate::models::{BrowseState, BrowseTabs, DownloadEntry};
    use crate::ui::{GLYPH_ACTIVE, GLYPH_DONE};
    use ratatui::{Terminal, backend::TestBackend, widgets::TableState};
    use soulseek_rs::types::{Download, DownloadMetadata, DownloadStatus};
    use soulseek_rs::{SharedDirectory, SharedFileEntry};
    use std::sync::mpsc;

    fn share() -> SharedDirectory {
        SharedDirectory {
            name: "share".to_string(),
            files: ["active.mp3", "done.mp3", "plain.mp3"]
                .iter()
                .map(|name| SharedFileEntry {
                    name: (*name).to_string(),
                    size: 4096,
                    attributes: Vec::new(),
                })
                .collect(),
        }
    }

    fn download(filename: &str, status: DownloadStatus) -> DownloadEntry {
        let (sender, _receiver) = mpsc::channel();
        DownloadEntry {
            download: Download {
                username: "bob".to_string(),
                // Queued files carry the full share path, as in the tree.
                filename: format!("share\\{filename}"),
                token: 1,
                size: 4096,
                download_directory: "/tmp".to_string(),
                status,
                sender,
                queue_position: None,
                metadata: DownloadMetadata::default(),
            },
            receiver: None,
        }
    }

    fn screen_with(downloads: &[DownloadEntry]) -> String {
        let mut browse = BrowseState::loading("bob".to_string());
        browse.load(&[share()]);
        let mut tabs = BrowseTabs::default();
        tabs.tabs.push(browse);
        let mut state = TableState::default();
        let mut terminal =
            Terminal::new(TestBackend::new(60, 8)).expect("backend");
        terminal
            .draw(|frame| {
                render_browse_pane(
                    frame,
                    frame.area(),
                    &tabs,
                    &mut state,
                    0,
                    downloads,
                );
            })
            .expect("draw");
        terminal.backend().to_string()
    }

    #[test]
    fn a_downloading_file_carries_its_transfer_glyph() {
        let screen = screen_with(&[
            download(
                "active.mp3",
                DownloadStatus::InProgress {
                    bytes_downloaded: 2048,
                    total_bytes: 4096,
                    speed_bytes_per_sec: 1.0,
                },
            ),
            download("done.mp3", DownloadStatus::Completed),
        ]);
        assert!(
            screen.contains(&format!("{GLYPH_ACTIVE} active.mp3")),
            "{screen}"
        );
        assert!(
            screen.contains(&format!("{GLYPH_DONE} done.mp3")),
            "{screen}"
        );
        assert!(
            screen.contains("50%"),
            "the size cell tells the progress: {screen}"
        );
        assert!(
            screen.contains("    plain.mp3"),
            "an unqueued row keeps the two plain spaces: {screen}"
        );
    }

    #[test]
    fn a_folder_marks_with_its_files_download() {
        // One file moving, one done: the folder row reports the job with
        // the loudest status below it, and the bytes' percent beside it.
        let screen = screen_with(&[
            download(
                "active.mp3",
                DownloadStatus::InProgress {
                    bytes_downloaded: 1024,
                    total_bytes: 4096,
                    speed_bytes_per_sec: 1.0,
                },
            ),
            download("done.mp3", DownloadStatus::Completed),
        ]);
        assert!(
            screen.contains(&format!("{GLYPH_ACTIVE} ▾ share")),
            "{screen}"
        );
        // 1024 of the 8192 bytes the two files hold has moved — the done
        // one brings its own 4096 — so the folder sits at 62%.
        assert!(screen.contains("62%"), "{screen}");
    }

    #[test]
    fn a_folder_is_done_when_every_file_is() {
        let screen = screen_with(&[
            download("active.mp3", DownloadStatus::Completed),
            download("done.mp3", DownloadStatus::Completed),
        ]);
        assert!(
            screen.contains(&format!("{GLYPH_DONE} ▾ share")),
            "{screen}"
        );
        assert!(!screen.contains(GLYPH_ACTIVE), "{screen}");
    }

    #[test]
    fn another_user_s_transfers_mark_nothing() {
        let mut theirs = download("active.mp3", DownloadStatus::Completed);
        theirs.download.username = "carol".to_string();
        let screen = screen_with(&[theirs]);
        assert!(!screen.contains(GLYPH_ACTIVE), "{screen}");
        assert!(!screen.contains(GLYPH_DONE), "{screen}");
    }
}
