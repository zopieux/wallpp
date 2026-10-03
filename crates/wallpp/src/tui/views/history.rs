use crossterm::event::{KeyCode, KeyEvent};
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Borders, Widget};
use ratatui_image::picker::Picker;
use ratatui_image::protocol::StatefulProtocol;
use ratatui_image::StatefulImage;
use std::path::PathBuf;

use crate::cache::CacheManager;
use crate::engine::collect_keep_paths;
use crate::state::State;
use crate::tui::preview::{create_picker, load_protocol_from_file};
use crate::wallpaper;

/// History viewer with cached image previews and immediate wallpaper application.
pub struct HistoryView {
    pub selected_index: usize,
    pub preview_cache: Option<(PathBuf, StatefulProtocol)>,
    picker: Picker,
}

impl HistoryView {
    pub fn new() -> Self {
        Self {
            selected_index: 0,
            preview_cache: None,
            picker: create_picker(),
        }
    }

    /// Handles keyboard events in the history view.
    /// Returns Some(status_message) if a wallpaper was immediately applied.
    pub fn handle_key(
        &mut self,
        key: KeyEvent,
        state: &mut State,
        cache_mgr: &CacheManager,
        max_history: usize,
        max_cache_size: u64,
    ) -> Option<String> {
        let count = state.history.len();
        if count == 0 {
            return None;
        }

        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                if self.selected_index == 0 {
                    self.selected_index = count - 1;
                } else {
                    self.selected_index -= 1;
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.selected_index = (self.selected_index + 1) % count;
            }
            KeyCode::PageUp => {
                self.selected_index = self.selected_index.saturating_sub(10);
            }
            KeyCode::PageDown => {
                self.selected_index = (self.selected_index + 10).min(count - 1);
            }
            KeyCode::Home => {
                self.selected_index = 0;
            }
            KeyCode::End => {
                self.selected_index = count - 1;
            }
            KeyCode::Enter => {
                if let Some(entry) = state.history.get(self.selected_index).cloned() {
                    if !entry.cache_path.exists() {
                        return Some(format!(
                            "Cached image missing from disk: {:?}",
                            entry.cache_path
                        ));
                    }

                    // Apply immediately to desktop.
                    match wallpaper::set_wallpaper(&entry.cache_path) {
                        Ok(method) => {
                            // Promote selected entry to new head entry in history.
                            state.promote(self.selected_index, max_history);
                            let _ = state.save();
                            self.selected_index = 0;

                            // Run cache maintenance.
                            let keep_paths = collect_keep_paths(state);
                            let _ = cache_mgr.enforce_max_size(max_cache_size, &keep_paths);

                            return Some(format!(
                                "Wallpaper applied via {} and promoted to head!",
                                method
                            ));
                        }
                        Err(e) => {
                            return Some(format!("Failed to set wallpaper: {}", e));
                        }
                    }
                }
            }
            _ => {}
        }

        None
    }

    pub fn render(&mut self, area: Rect, buf: &mut Buffer, state: &State) {
        let block = Block::default()
            .borders(Borders::ALL)
            .title(format!(" Wallpaper History ({}) ", state.history.len()))
            .style(Style::default().fg(Color::White));
        let inner = block.inner(area);
        block.render(area, buf);

        if inner.width < 20 || inner.height < 6 {
            return;
        }

        if state.history.is_empty() {
            buf.set_string(
                inner.x + 2,
                inner.y + 2,
                "No wallpaper history recorded yet.",
                Style::default().fg(Color::DarkGray),
            );
            return;
        }

        // Split into history list (left 55%) and image preview & details (right 45%).
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
            .split(inner);

        self.render_list(chunks[0], buf, state);
        self.render_preview(chunks[1], buf, state);
    }

    fn render_list(&self, area: Rect, buf: &mut Buffer, state: &State) {
        let block = Block::default().borders(Borders::NONE);
        let inner = block.inner(area);

        let header_style = Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD);
        buf.set_string(inner.x + 2, inner.y, "CURRENT", header_style);
        buf.set_string(inner.x + 15, inner.y, "SOURCE", header_style);
        buf.set_string(inner.x + 32, inner.y, "TITLE / ID", header_style);

        let mut y = inner.y + 1;
        for cell_x in inner.x..(inner.x + inner.width.saturating_sub(1)) {
            buf.set_string(cell_x, y, "─", Style::default().fg(Color::DarkGray));
        }
        y += 1;

        let visible_lines = (inner.y + inner.height).saturating_sub(y + 2) as usize;
        let start_idx = if self.selected_index >= visible_lines {
            self.selected_index - visible_lines + 1
        } else {
            0
        };

        for (idx, entry) in state.history.iter().enumerate().skip(start_idx) {
            if y >= inner.y + inner.height.saturating_sub(2) {
                break;
            }
            let is_selected = idx == self.selected_index;
            let is_head = idx == 0;

            let row_style = if is_selected {
                Style::default()
                    .fg(Color::Yellow)
                    .bg(Color::Rgb(30, 30, 45))
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };

            let head_marker = if is_head { "★ Current" } else { "" };
            let pointer = if is_selected { "▶ " } else { "  " };
            buf.set_string(inner.x, y, pointer, row_style);

            let head_style = if is_head {
                if is_selected {
                    Style::default()
                        .fg(Color::Green)
                        .bg(Color::Rgb(30, 30, 45))
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::Green)
                }
            } else {
                row_style
            };
            buf.set_string(inner.x + 2, y, format!("{:<13}", head_marker), head_style);

            let src = entry.source_name.as_deref().unwrap_or(&entry.provider);
            let truncated_src = if src.len() > 15 {
                format!("{:<17}", format!("{:.14}…", src))
            } else {
                format!("{:<17}", src)
            };
            buf.set_string(inner.x + 15, y, truncated_src, row_style);

            let title = entry.title.as_deref().unwrap_or(&entry.opaque_id);
            let max_title_w = inner.width.saturating_sub(33) as usize;
            let truncated_title = if title.len() > max_title_w {
                format!("{:.w$}…", title, w = max_title_w.saturating_sub(1))
            } else if is_selected {
                format!("{:<w$}", title, w = max_title_w)
            } else {
                title.to_string()
            };
            buf.set_string(inner.x + 32, y, truncated_title, row_style);

            y += 1;
        }

        let hint_y = inner.y + inner.height.saturating_sub(1);
        buf.set_string(
            inner.x + 2,
            hint_y,
            "[Enter] Pick Wallpaper   [↑/↓/PgUp/PgDn] Navigate",
            Style::default().fg(Color::DarkGray),
        );
    }

    fn render_preview(&mut self, area: Rect, buf: &mut Buffer, state: &State) {
        let block = Block::default()
            .borders(Borders::LEFT)
            .title(" Wallpaper Preview ")
            .style(Style::default().fg(Color::White));
        let inner = block.inner(area);
        block.render(area, buf);

        let Some(entry) = state.history.get(self.selected_index) else {
            return;
        };

        if inner.height < 6 || inner.width < 10 {
            return;
        }

        // Split into image area (top 65%) and metadata (bottom 35%).
        let img_height = (inner.height * 65) / 100;
        let img_area = Rect {
            x: inner.x + 1,
            y: inner.y + 1,
            width: inner.width.saturating_sub(2),
            height: img_height.saturating_sub(1),
        };

        let meta_area = Rect {
            x: inner.x + 1,
            y: inner.y + img_height + 1,
            width: inner.width.saturating_sub(2),
            height: inner.height.saturating_sub(img_height + 1),
        };

        // Load preview protocol from cache or disk.
        if entry.cache_path.exists() {
            let need_load = match &self.preview_cache {
                Some((path, _)) => path != &entry.cache_path,
                None => true,
            };

            if need_load {
                if let Ok(protocol) = load_protocol_from_file(&self.picker, &entry.cache_path) {
                    self.preview_cache = Some((entry.cache_path.clone(), protocol));
                } else {
                    self.preview_cache = None;
                }
            }

            if let Some((_, ref mut protocol)) = self.preview_cache {
                let widget = StatefulImage::default();
                ratatui::widgets::StatefulWidget::render(widget, img_area, buf, protocol);
            } else {
                buf.set_string(
                    img_area.x + 2,
                    img_area.y + 2,
                    "Failed to decode image preview",
                    Style::default().fg(Color::Red),
                );
            }
        } else {
            buf.set_string(
                img_area.x + 2,
                img_area.y + 2,
                "Cached image file missing from disk",
                Style::default().fg(Color::DarkGray),
            );
        }

        // Render metadata text.
        let mut my = meta_area.y;
        let label_style = Style::default().fg(Color::Cyan);
        let val_x = meta_area.x + 13;
        let max_val_w = (meta_area.x + meta_area.width).saturating_sub(val_x + 1) as usize;

        let title = entry.title.as_deref().unwrap_or(&entry.opaque_id);
        let trunc_title = if title.len() > max_val_w && max_val_w > 0 {
            format!("{:.w$}…", title, w = max_val_w.saturating_sub(1))
        } else {
            title.to_string()
        };
        buf.set_string(meta_area.x + 1, my, "Title:", label_style);
        buf.set_string(
            val_x,
            my,
            trunc_title,
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        );
        my += 1;

        if let Some(ref author) = entry.author {
            let trunc_author = if author.len() > max_val_w && max_val_w > 0 {
                format!("{:.w$}…", author, w = max_val_w.saturating_sub(1))
            } else {
                author.clone()
            };
            buf.set_string(meta_area.x + 1, my, "Author:", label_style);
            buf.set_string(val_x, my, trunc_author, Style::default().fg(Color::Gray));
            my += 1;
        }

        let dims_str = match (entry.width, entry.height) {
            (Some(w), Some(h)) => format!("{}x{}", w, h),
            _ => "Unknown".to_string(),
        };
        buf.set_string(meta_area.x + 1, my, "Dimensions:", label_style);
        buf.set_string(val_x, my, dims_str, Style::default().fg(Color::DarkGray));
        my += 1;

        let src = entry.source_name.as_deref().unwrap_or(&entry.provider);
        let trunc_src = if src.len() > max_val_w && max_val_w > 0 {
            format!("{:.w$}…", src, w = max_val_w.saturating_sub(1))
        } else {
            src.to_string()
        };
        buf.set_string(meta_area.x + 1, my, "Source:", label_style);
        buf.set_string(val_x, my, trunc_src, Style::default().fg(Color::Gray));
        my += 1;

        if let Some(ref url) = entry.source_url {
            let trunc_url = if url.len() > max_val_w && max_val_w > 0 {
                format!("{:.w$}…", url, w = max_val_w.saturating_sub(1))
            } else {
                url.clone()
            };
            buf.set_string(meta_area.x + 1, my, "URL:", label_style);
            buf.set_string(val_x, my, trunc_url, Style::default().fg(Color::DarkGray));
        }
    }
}
