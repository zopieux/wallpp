use crossterm::event::{KeyCode, KeyEvent};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Borders, Clear, Widget};
use std::collections::HashMap;

use crate::config::{AppConfig, SourceConfig};
use crate::provider::DiscoveredProvider;
use crate::sources::resolve_sources;
use crate::tui::field::FieldKind;

/// State for the "Add New Source" modal popup.
pub struct AddSourceModal {
    pub providers: Vec<(String, String)>,
    pub selected_provider_idx: usize,
    pub custom_name_field: FieldKind,
    pub focused_field: usize, // 0 = provider choice, 1 = custom name.
}

impl AddSourceModal {
    pub fn new(discovered: &HashMap<String, DiscoveredProvider>) -> Self {
        let mut providers: Vec<(String, String)> = discovered
            .values()
            .map(|p| (p.name.clone(), p.info.label.clone()))
            .collect();
        providers.sort_by(|a, b| a.1.cmp(&b.1));
        Self {
            providers,
            selected_provider_idx: 0,
            custom_name_field: FieldKind::Text {
                value: String::new(),
                cursor: 0,
            },
            focused_field: 0,
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> Option<Option<(String, Option<String>)>> {
        match key.code {
            KeyCode::Esc => Some(None), // Cancelled.
            KeyCode::Tab | KeyCode::Down if self.focused_field == 0 => {
                self.focused_field = 1;
                None
            }
            KeyCode::BackTab | KeyCode::Up if self.focused_field == 1 => {
                self.focused_field = 0;
                None
            }
            KeyCode::Left if self.focused_field == 0 => {
                if !self.providers.is_empty() {
                    if self.selected_provider_idx == 0 {
                        self.selected_provider_idx = self.providers.len() - 1;
                    } else {
                        self.selected_provider_idx -= 1;
                    }
                }
                None
            }
            KeyCode::Right if self.focused_field == 0 => {
                if !self.providers.is_empty() {
                    self.selected_provider_idx =
                        (self.selected_provider_idx + 1) % self.providers.len();
                }
                None
            }
            KeyCode::Enter => {
                if self.providers.is_empty() {
                    Some(None)
                } else {
                    let prov = self.providers[self.selected_provider_idx].0.clone();
                    let name = match &self.custom_name_field {
                        FieldKind::Text { value, .. } => {
                            let trimmed = value.trim();
                            if trimmed.is_empty() {
                                None
                            } else {
                                Some(trimmed.to_string())
                            }
                        }
                        _ => None,
                    };
                    Some(Some((prov, name)))
                }
            }
            _ if self.focused_field == 1 => {
                self.custom_name_field.handle_key(key);
                None
            }
            _ => None,
        }
    }

    pub fn render(&self, area: Rect, buf: &mut Buffer) {
        let popup_width = 54.min(area.width.saturating_sub(4));
        let popup_height = 12.min(area.height.saturating_sub(4));
        let popup_area = Rect {
            x: area.x + (area.width.saturating_sub(popup_width)) / 2,
            y: area.y + (area.height.saturating_sub(popup_height)) / 2,
            width: popup_width,
            height: popup_height,
        };

        Clear.render(popup_area, buf);
        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Add New Source ")
            .style(Style::default().fg(Color::Cyan));
        let inner = block.inner(popup_area);
        block.render(popup_area, buf);

        if inner.height < 6 {
            return;
        }

        // Provider Selection.
        let prov_style = if self.focused_field == 0 {
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Gray)
        };
        buf.set_string(inner.x + 2, inner.y + 1, "Provider:", prov_style);
        let prov_name = if self.providers.is_empty() {
            "None available".to_string()
        } else {
            format!("< {} >", self.providers[self.selected_provider_idx].1)
        };
        buf.set_string(inner.x + 16, inner.y + 1, prov_name, prov_style);

        // Custom Name Field.
        let name_style = if self.focused_field == 1 {
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Gray)
        };
        buf.set_string(inner.x + 2, inner.y + 3, "Custom Name:", name_style);
        buf.set_string(
            inner.x + 2,
            inner.y + 4,
            "(optional - computed name used if blank)",
            Style::default().fg(Color::DarkGray),
        );
        let input_area = Rect {
            x: inner.x + 16,
            y: inner.y + 3,
            width: inner.width.saturating_sub(18),
            height: 1,
        };
        self.custom_name_field
            .render(input_area, buf, self.focused_field == 1);

        // Help actions.
        buf.set_string(
            inner.x + 2,
            inner.y + 7,
            "[Enter] Create   [Tab] Switch Field   [Esc] Cancel",
            Style::default().fg(Color::DarkGray),
        );
    }
}

/// Action resulting from user input in the SourcesView.
pub enum SourcesAction {
    None,
    OpenEdit(usize),
}

/// View listing configured sources with management operations (Add, Edit, Delete).
pub struct SourcesView {
    pub selected_index: usize,
    pub add_modal: Option<AddSourceModal>,
}

impl SourcesView {
    pub fn new() -> Self {
        Self {
            selected_index: 0,
            add_modal: None,
        }
    }

    pub fn handle_key(
        &mut self,
        key: KeyEvent,
        app_cfg: &mut AppConfig,
        discovered: &HashMap<String, DiscoveredProvider>,
    ) -> SourcesAction {
        if let Some(ref mut modal) = self.add_modal {
            if let Some(res) = modal.handle_key(key) {
                self.add_modal = None;
                if let Some((prov, custom_name)) = res {
                    // Populate explicit sources if switching from implicit default mode.
                    if app_cfg.source.is_empty() {
                        let mut sorted_keys: Vec<_> = discovered.keys().collect();
                        sorted_keys.sort();
                        for k in sorted_keys {
                            app_cfg.source.push(SourceConfig {
                                name: None,
                                provider: k.clone(),
                                options: HashMap::new(),
                            });
                        }
                    }

                    // Add new source.
                    let new_src = SourceConfig {
                        name: custom_name,
                        provider: prov,
                        options: HashMap::new(),
                    };
                    app_cfg.source.push(new_src);
                    let new_idx = app_cfg.source.len() - 1;
                    self.selected_index = new_idx;
                    return SourcesAction::OpenEdit(new_idx);
                }
            }
            return SourcesAction::None;
        }

        let resolved = resolve_sources(app_cfg, None, discovered);
        let count = resolved.len();

        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                if count > 0 {
                    if self.selected_index == 0 {
                        self.selected_index = count - 1;
                    } else {
                        self.selected_index -= 1;
                    }
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if count > 0 {
                    self.selected_index = (self.selected_index + 1) % count;
                }
            }
            KeyCode::Enter => {
                if count > 0 && self.selected_index < count {
                    // If opening an implicit source, make sources explicit first.
                    if app_cfg.source.is_empty() {
                        let mut sorted_keys: Vec<_> = discovered.keys().collect();
                        sorted_keys.sort();
                        for k in sorted_keys {
                            app_cfg.source.push(SourceConfig {
                                name: None,
                                provider: k.clone(),
                                options: HashMap::new(),
                            });
                        }
                    }
                    return SourcesAction::OpenEdit(self.selected_index);
                }
            }
            KeyCode::Char('a') => {
                self.add_modal = Some(AddSourceModal::new(discovered));
            }
            KeyCode::Char('d') if count > 0 && self.selected_index < count => {
                if app_cfg.source.is_empty() {
                    // To disable a provider from default implicit sources,
                    // expand all providers except the selected one into explicit sources.
                    let mut sorted_keys: Vec<_> = discovered.keys().collect();
                    sorted_keys.sort();
                    for (i, k) in sorted_keys.into_iter().enumerate() {
                        if i != self.selected_index {
                            app_cfg.source.push(SourceConfig {
                                name: None,
                                provider: k.clone(),
                                options: HashMap::new(),
                            });
                        }
                    }
                } else if self.selected_index < app_cfg.source.len() {
                    app_cfg.source.remove(self.selected_index);
                }
                let new_count = app_cfg.source.len();
                if self.selected_index >= new_count && new_count > 0 {
                    self.selected_index = new_count - 1;
                }
            }
            _ => {}
        }

        SourcesAction::None
    }

    pub fn render(
        &self,
        area: Rect,
        buf: &mut Buffer,
        app_cfg: &AppConfig,
        discovered: &HashMap<String, DiscoveredProvider>,
    ) {
        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Configured Wallpaper Sources ")
            .style(Style::default().fg(Color::White));
        let inner = block.inner(area);
        block.render(area, buf);

        if inner.width < 10 || inner.height < 6 {
            return;
        }

        let is_implicit = app_cfg.source.is_empty();
        let resolved = resolve_sources(app_cfg, None, discovered);

        let mut y = inner.y + 1;

        if is_implicit {
            buf.set_string(
                inner.x + 2,
                y,
                "ℹ Default Mode: All discovered providers enabled with standard options.",
                Style::default().fg(Color::Yellow),
            );
            buf.set_string(
                inner.x + 4,
                y + 1,
                "Press 'a' to add a custom source or 'd' to disable a provider.",
                Style::default().fg(Color::DarkGray),
            );
            y += 3;
        }

        // Header.
        let header_style = Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD);
        buf.set_string(inner.x + 4, y, "SOURCE LABEL", header_style);
        buf.set_string(inner.x + 28, y, "PROVIDER", header_style);
        buf.set_string(inner.x + 46, y, "OPTIONS SUMMARY", header_style);
        y += 1;

        for cell_x in (inner.x + 2)..(inner.x + inner.width.saturating_sub(2)) {
            buf.set_string(cell_x, y, "─", Style::default().fg(Color::DarkGray));
        }
        y += 1;

        if resolved.is_empty() {
            buf.set_string(
                inner.x + 4,
                y + 1,
                "No active wallpaper sources available.",
                Style::default().fg(Color::Red),
            );
        } else {
            for (idx, src) in resolved.iter().enumerate() {
                if y >= inner.y + inner.height.saturating_sub(3) {
                    break;
                }
                let is_selected = idx == self.selected_index;
                let row_style = if is_selected {
                    Style::default()
                        .fg(Color::Yellow)
                        .bg(Color::Rgb(30, 30, 45))
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::White)
                };

                let marker = if is_selected { "▶ " } else { "  " };
                buf.set_string(inner.x + 2, y, marker, row_style);
                buf.set_string(inner.x + 4, y, &src.display_label, row_style);
                buf.set_string(inner.x + 28, y, &src.provider.info.label, row_style);

                let opt_summary = if src.options.is_empty() {
                    "(defaults)".to_string()
                } else {
                    let mut pairs: Vec<_> = src
                        .options
                        .iter()
                        .map(|(k, v)| format!("{}={}", k, v))
                        .collect();
                    pairs.sort();
                    pairs.join(", ")
                };
                buf.set_string(
                    inner.x + 46,
                    y,
                    opt_summary,
                    Style::default().fg(Color::Gray),
                );

                y += 1;
            }
        }

        // Bottom action hints.
        let hint_y = inner.y + inner.height.saturating_sub(1);
        buf.set_string(
            inner.x + 2,
            hint_y,
            "[Enter] Edit Options   [a] Add Source   [d] Delete/Disable   [↑/↓] Navigate",
            Style::default().fg(Color::DarkGray),
        );

        // Render modal if open.
        if let Some(ref modal) = self.add_modal {
            modal.render(area, buf);
        }
    }
}

impl Default for SourcesView {
    fn default() -> Self {
        Self::new()
    }
}
