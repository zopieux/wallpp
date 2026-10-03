use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Tabs, Widget, Wrap};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc::UnboundedSender;

use crate::cache::CacheManager;
use crate::config::AppConfig;
use crate::provider::ProviderManager;
use crate::state::State;
use crate::tui::preview::PreviewTaskResult;
use crate::tui::views::{GeneralView, HistoryView, SourceEditView, SourcesAction, SourcesView};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppTab {
    General = 0,
    Sources = 1,
    History = 2,
}

/// Root TUI Application state managing navigation, tabs, draft config, and sub-views.
pub struct App {
    pub active_tab: AppTab,
    pub draft_config: AppConfig,
    pub saved_config: AppConfig,
    pub provider_mgr: Arc<ProviderManager>,
    pub general_view: GeneralView,
    pub sources_view: SourcesView,
    pub source_edit_view: Option<SourceEditView>,
    pub history_view: HistoryView,
    pub notification: Option<(String, Instant)>,
    pub show_quit_confirm: bool,
    pub should_quit: bool,
    pub preview_tx: UnboundedSender<PreviewTaskResult>,
    pub needs_clear: bool,
}

impl App {
    pub fn new(
        loaded_config: AppConfig,
        provider_mgr: Arc<ProviderManager>,
        preview_tx: UnboundedSender<PreviewTaskResult>,
    ) -> Self {
        let general_view = GeneralView::new(&loaded_config.manager);
        let sources_view = SourcesView::new();
        let history_view = HistoryView::new();

        Self {
            active_tab: AppTab::General,
            draft_config: loaded_config.clone(),
            saved_config: loaded_config,
            provider_mgr,
            general_view,
            sources_view,
            source_edit_view: None,
            history_view,
            notification: None,
            show_quit_confirm: false,
            should_quit: false,
            preview_tx,
            needs_clear: false,
        }
    }

    /// Checks whether the user has uncommitted changes in the configuration.
    pub fn is_dirty(&self) -> bool {
        self.draft_config != self.saved_config
    }

    /// Displays a temporary banner notification in the status bar.
    pub fn show_notification(&mut self, msg: impl Into<String>) {
        self.notification = Some((msg.into(), Instant::now()));
    }

    /// Handles keyboard events dispatching to modals, sub-views, or top-level navigation.
    pub fn handle_key(&mut self, key: KeyEvent, cache_mgr: &CacheManager, state: &mut State) {
        // Unsaved changes exit confirmation modal.
        if self.show_quit_confirm {
            match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => {
                    if let Err(e) = self.draft_config.save() {
                        self.show_notification(format!("Failed to save config: {}", e));
                        self.show_quit_confirm = false;
                    } else {
                        self.saved_config = self.draft_config.clone();
                        self.should_quit = true;
                    }
                }
                KeyCode::Char('n') | KeyCode::Char('N') => {
                    self.should_quit = true;
                }
                KeyCode::Esc => {
                    self.show_quit_confirm = false;
                }
                _ => {}
            }
            return;
        }

        // Global Save (^S).
        if key.modifiers.contains(KeyModifiers::CONTROL)
            && (key.code == KeyCode::Char('s') || key.code == KeyCode::Char('S'))
        {
            match self.draft_config.save() {
                Ok(_) => {
                    self.saved_config = self.draft_config.clone();
                    self.show_notification("Configuration saved to disk.");
                }
                Err(e) => {
                    self.show_notification(format!("Error saving config: {}", e));
                }
            }
            return;
        }

        // Source Edit View.
        if let Some(ref mut edit_view) = self.source_edit_view {
            if key.code == KeyCode::Esc {
                // Done editing source, commit changes in memory and return.
                if edit_view.source_index < self.draft_config.source.len() {
                    edit_view.apply_to_source_config(
                        &mut self.draft_config.source[edit_view.source_index],
                    );
                }
                self.source_edit_view = None;
                self.needs_clear = true;
                return;
            }

            edit_view.handle_key(
                key,
                Arc::clone(&self.provider_mgr),
                &mut self.draft_config,
                &self.preview_tx,
            );
            return;
        }

        // Sources Add Modal.
        if self.active_tab == AppTab::Sources && self.sources_view.add_modal.is_some() {
            let action = self.sources_view.handle_key(
                key,
                &mut self.draft_config,
                &self.provider_mgr.providers,
            );
            if let SourcesAction::OpenEdit(idx) = action {
                self.open_source_edit(idx);
            }
            return;
        }

        // Global Quit (q).
        if key.code == KeyCode::Char('q') {
            if self.is_dirty() {
                self.show_quit_confirm = true;
            } else {
                self.should_quit = true;
            }
            return;
        }

        // Direct Tab Switching (F1..F3 or Alt+1..3).
        match key.code {
            KeyCode::F(1) => {
                if self.active_tab != AppTab::General || self.source_edit_view.is_some() {
                    self.active_tab = AppTab::General;
                    self.source_edit_view = None;
                    self.needs_clear = true;
                }
                return;
            }
            KeyCode::F(2) => {
                if self.active_tab != AppTab::Sources || self.source_edit_view.is_some() {
                    self.active_tab = AppTab::Sources;
                    self.source_edit_view = None;
                    self.needs_clear = true;
                }
                return;
            }
            KeyCode::F(3) => {
                if self.active_tab != AppTab::History || self.source_edit_view.is_some() {
                    self.active_tab = AppTab::History;
                    self.source_edit_view = None;
                    self.needs_clear = true;
                }
                return;
            }
            _ => {}
        }

        // Route to Active Tab View.
        match self.active_tab {
            AppTab::General => {
                self.general_view
                    .handle_key(key, &mut self.draft_config.manager);
            }
            AppTab::Sources => {
                let action = self.sources_view.handle_key(
                    key,
                    &mut self.draft_config,
                    &self.provider_mgr.providers,
                );
                if let SourcesAction::OpenEdit(idx) = action {
                    self.open_source_edit(idx);
                }
            }
            AppTab::History => {
                let prev_selected = self.history_view.selected_index;
                let max_hist = self.draft_config.manager.history_length;
                let max_cache = self.draft_config.manager.cache_max_size.as_bytes();
                if let Some(msg) = self
                    .history_view
                    .handle_key(key, state, cache_mgr, max_hist, max_cache)
                {
                    self.show_notification(msg);
                }
                if self.history_view.selected_index != prev_selected {
                    self.needs_clear = true;
                }
            }
        }
    }

    /// Opens the source editor for a given source index and triggers its initial 3-image preview fetch.
    fn open_source_edit(&mut self, source_idx: usize) {
        if let Some(src) = self.draft_config.source.get(source_idx) {
            if let Some(prov) = self.provider_mgr.providers.get(&src.provider) {
                let mut edit_view = SourceEditView::new(source_idx, src, prov);
                edit_view.trigger_preview_fetch(
                    Arc::clone(&self.provider_mgr),
                    &self.draft_config,
                    self.preview_tx.clone(),
                );
                self.source_edit_view = Some(edit_view);
                self.needs_clear = true;
            }
        }
    }

    /// Processes incoming async preview results.
    pub fn handle_preview_result(&mut self, result: PreviewTaskResult) {
        if let Some(ref mut edit_view) = self.source_edit_view {
            edit_view.handle_preview_result(result);
        }
    }

    /// Handles pasted text from terminal bracketed paste events.
    pub fn handle_paste(&mut self, text: &str) {
        if self.show_quit_confirm {
            return;
        }

        if let Some(ref mut edit_view) = self.source_edit_view {
            edit_view.handle_paste(text);
            return;
        }

        match self.active_tab {
            AppTab::General => {
                self.general_view
                    .handle_paste(text, &mut self.draft_config.manager);
            }
            AppTab::Sources => {
                if let Some(ref mut modal) = self.sources_view.add_modal {
                    modal.custom_name_field.handle_paste(text);
                }
            }
            AppTab::History => {}
        }
    }

    /// Renders the complete TUI frame.
    pub fn render(&mut self, area: Rect, buf: &mut Buffer, state: &State) {
        // Clear screen.
        Clear.render(area, buf);

        // Top-level vertical layout: Tab Header (3), Main Body (fill), Status Bar (2).
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(5),
                Constraint::Length(2),
            ])
            .split(area);

        self.render_tab_header(chunks[0], buf);

        // Render body view.
        if let Some(ref mut edit_view) = self.source_edit_view {
            edit_view.render(chunks[1], buf);
        } else {
            match self.active_tab {
                AppTab::General => self.general_view.render(chunks[1], buf),
                AppTab::Sources => self.sources_view.render(
                    chunks[1],
                    buf,
                    &self.draft_config,
                    &self.provider_mgr.providers,
                ),
                AppTab::History => self.history_view.render(chunks[1], buf, state),
            }
        }

        self.render_status_bar(chunks[2], buf);

        // Render quit confirmation modal if active.
        if self.show_quit_confirm {
            self.render_quit_modal(area, buf);
        }
    }

    fn render_tab_header(&self, area: Rect, buf: &mut Buffer) {
        let dirty_indicator = if self.is_dirty() {
            " ● [Modified - Unsaved] "
        } else {
            " "
        };

        let titles = vec!["[F1] Settings", "[F2] Sources", "[F3] History"];

        let selected = match self.active_tab {
            AppTab::General => 0,
            AppTab::Sources => 1,
            AppTab::History => 2,
        };

        let block = Block::default()
            .borders(Borders::BOTTOM)
            .title(format!(" wallpp config editor{} ", dirty_indicator))
            .title_style(if self.is_dirty() {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            });

        let tabs = Tabs::new(titles)
            .block(block)
            .select(selected)
            .style(Style::default().fg(Color::DarkGray))
            .highlight_style(
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            );

        tabs.render(area, buf);
    }

    fn render_status_bar(&self, area: Rect, buf: &mut Buffer) {
        let mut text = String::new();
        let mut style = Style::default().fg(Color::DarkGray);

        if let Some((ref msg, instant)) = self.notification {
            if instant.elapsed() < Duration::from_secs(5) {
                text = format!(" ℹ {}", msg);
                style = Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD);
            }
        }

        if text.is_empty() {
            text = " [^S] Save Config   [F1..F3] Switch Tab   [q] Quit".to_string();
        }

        buf.set_string(area.x + 1, area.y, text, style);
    }

    fn render_quit_modal(&self, area: Rect, buf: &mut Buffer) {
        let popup_width = 58.min(area.width.saturating_sub(4));
        let popup_height = 8.min(area.height.saturating_sub(4));
        let popup_area = Rect {
            x: area.x + (area.width.saturating_sub(popup_width)) / 2,
            y: area.y + (area.height.saturating_sub(popup_height)) / 2,
            width: popup_width,
            height: popup_height,
        };

        Clear.render(popup_area, buf);
        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Unsaved Changes ")
            .style(Style::default().fg(Color::Yellow));
        let inner = block.inner(popup_area);
        block.render(popup_area, buf);

        let content_area = Rect {
            x: inner.x + 2,
            y: inner.y + 1,
            width: inner.width.saturating_sub(4),
            height: inner.height.saturating_sub(2),
        };

        let text = vec![
            Line::from("Save configuration before exiting?").style(
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Line::from(""),
            Line::from(vec![
                Span::styled(
                    "[Y] Save & Exit",
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("   "),
                Span::styled(
                    "[N] Discard & Exit",
                    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                ),
                Span::raw("   "),
                Span::styled("[Esc] Cancel", Style::default().fg(Color::DarkGray)),
            ]),
        ];

        let paragraph = Paragraph::new(text).wrap(Wrap { trim: true });
        paragraph.render(content_area, buf);
    }
}
