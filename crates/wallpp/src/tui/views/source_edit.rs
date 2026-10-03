use crossterm::event::{KeyCode, KeyEvent};
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Borders, Widget};
use ratatui_image::StatefulImage;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc::UnboundedSender;

use crate::config::{toml_to_wit_config, AppConfig, SourceConfig};
use crate::monitor;
use crate::provider::{DiscoveredProvider, ProviderManager};
use crate::tui::field::FieldKind;
use crate::tui::preview::{
    create_picker, load_protocol_from_memory, PreviewItem, PreviewTaskResult,
};
use crate::wallpp::provider::types::{ConfigEntry, ConfigValue, OptionSpec, ScalarValue};

/// Holds form fields and preview state for editing a specific source.
pub struct SourceEditView {
    pub source_index: usize,
    pub provider_name: String,
    pub provider_label: String,
    pub custom_name_field: FieldKind,
    pub option_specs: Vec<OptionSpec>,
    pub option_fields: Vec<(String, FieldKind)>,
    pub focused_field: usize, // 0 = custom name, 1.. = option fields.
    pub form_scroll_offset: usize,
    pub previews: Vec<PreviewItem>,
    pub is_loading_previews: bool,
    pub preview_error: Option<String>,
    pub preview_generation: u64,
}

impl SourceEditView {
    pub fn new(source_index: usize, src: &SourceConfig, provider: &DiscoveredProvider) -> Self {
        let custom_name_val = src.name.clone().unwrap_or_default();
        let custom_name_cursor = custom_name_val.len();

        let mut option_fields = Vec::new();
        for spec in &provider.info.options {
            let existing_val = src.options.get(&spec.key);
            let field = FieldKind::from_option_spec(spec, existing_val);
            option_fields.push((spec.key.clone(), field));
        }

        Self {
            source_index,
            provider_name: provider.name.clone(),
            provider_label: provider.info.label.clone(),
            custom_name_field: FieldKind::Text {
                value: custom_name_val,
                cursor: custom_name_cursor,
            },
            option_specs: provider.info.options.clone(),
            option_fields,
            focused_field: 0,
            form_scroll_offset: 0,
            previews: Vec::new(),
            is_loading_previews: false,
            preview_error: None,
            preview_generation: 0,
        }
    }

    /// Triggers an asynchronous fetch of the next 3 preview images for this source.
    pub fn trigger_preview_fetch(
        &mut self,
        provider_mgr: Arc<ProviderManager>,
        app_cfg: &AppConfig,
        tx: UnboundedSender<PreviewTaskResult>,
    ) {
        self.preview_generation += 1;
        let gen = self.preview_generation;
        self.is_loading_previews = true;
        self.preview_error = None;

        let Some(prov) = provider_mgr.providers.get(&self.provider_name) else {
            self.is_loading_previews = false;
            self.preview_error = Some("Provider not found".to_string());
            return;
        };

        // Extract current options from form fields.
        let mut options_map = HashMap::new();
        for (i, (key, field)) in self.option_fields.iter().enumerate() {
            let spec = &self.option_specs[i];
            if let Ok(Some(val)) = field.to_toml_value(spec.multiple) {
                options_map.insert(key.clone(), val);
            }
        }

        let wit_cfg =
            toml_to_wit_config(&options_map, &prov.info.options, &prov.info.default_config);

        let provider_name = self.provider_name.clone();
        let min_percent = app_cfg.manager.min_display_percentage;
        let mgr = Arc::clone(&provider_mgr);

        tokio::spawn(async move {
            let filter = monitor::compute_filter_criteria(min_percent);

            // Validate and normalize configuration via provider wasm hook.
            let valid_cfg = match mgr.validate_config(&provider_name, wit_cfg).await {
                Ok(c) => c,
                Err(e) => {
                    let _ = tx.send(PreviewTaskResult::Error {
                        generation: gen,
                        message: format!("Config error: {}", e),
                    });
                    return;
                }
            };

            let page_res = mgr
                .query_list(&provider_name, valid_cfg.clone(), 3, None, filter)
                .await;

            let page = match page_res {
                Ok(p) => p,
                Err(e) => {
                    let _ = tx.send(PreviewTaskResult::Error {
                        generation: gen,
                        message: format!("Failed to query provider: {}", e),
                    });
                    return;
                }
            };

            let picker = create_picker();
            let mut items = Vec::new();

            for item in page.items.into_iter().take(3) {
                match mgr
                    .query_download(&provider_name, valid_cfg.clone(), &item.id)
                    .await
                {
                    Ok(img) => match load_protocol_from_memory(&picker, &img.data) {
                        Ok(protocol) => {
                            items.push(PreviewItem {
                                id: item.id,
                                title: item.title,
                                author: item.author,
                                width: item.width,
                                height: item.height,
                                protocol,
                            });
                        }
                        Err(e) => {
                            eprintln!("Warning: failed to decode image: {}", e);
                        }
                    },
                    Err(e) => {
                        eprintln!("Warning: failed to download preview image: {}", e);
                    }
                }
            }

            if items.is_empty() {
                let _ = tx.send(PreviewTaskResult::Error {
                    generation: gen,
                    message: "No wallpapers returned matching criteria".to_string(),
                });
            } else {
                let _ = tx.send(PreviewTaskResult::Success {
                    generation: gen,
                    items,
                    normalized_config: Some(valid_cfg),
                });
            }
        });
    }

    /// Handles incoming preview task results.
    pub fn handle_preview_result(&mut self, result: PreviewTaskResult) {
        match result {
            PreviewTaskResult::Success {
                generation,
                items,
                normalized_config,
            } => {
                if generation == self.preview_generation {
                    self.is_loading_previews = false;
                    self.preview_error = None;
                    self.previews = items;

                    // Update form fields with normalized values if provided.
                    if let Some(cfg) = normalized_config {
                        self.apply_normalized_config(&cfg);
                    }
                }
            }
            PreviewTaskResult::Error {
                generation,
                message,
            } => {
                if generation == self.preview_generation {
                    self.is_loading_previews = false;
                    self.preview_error = Some(message);
                    self.previews.clear();
                }
            }
        }
    }

    /// Updates editor form fields from the validated and normalized config returned by the provider.
    fn apply_normalized_config(&mut self, cfg: &[ConfigEntry]) {
        for (key, field) in &mut self.option_fields {
            if let Some(entry) = cfg.iter().find(|e| e.key == *key) {
                match (field, &entry.value) {
                    (
                        FieldKind::ManyText {
                            value,
                            cursor,
                            scroll_offset,
                        },
                        ConfigValue::Many(items),
                    ) => {
                        let lines: Vec<String> = items
                            .iter()
                            .filter_map(|it| match it {
                                ScalarValue::Text(s) | ScalarValue::Choice(s) => Some(s.clone()),
                                _ => None,
                            })
                            .collect();
                        *value = lines.join("\n");
                        *cursor = (*cursor).min(value.len());
                        *scroll_offset = (*scroll_offset).min(lines.len().saturating_sub(1));
                    }
                    (FieldKind::Text { value, cursor }, ConfigValue::One(ScalarValue::Text(s))) => {
                        *value = s.clone();
                        *cursor = (*cursor).min(value.len());
                    }
                    _ => {}
                }
            }
        }
    }

    /// Handles pasted text into the currently focused field.
    pub fn handle_paste(&mut self, text: &str) -> bool {
        if self.focused_field == 0 {
            self.custom_name_field.handle_paste(text)
        } else {
            let opt_idx = self.focused_field - 1;
            if opt_idx < self.option_fields.len() {
                self.option_fields[opt_idx].1.handle_paste(text)
            } else {
                false
            }
        }
    }

    /// Applies form values to the source configuration in the draft AppConfig.
    pub fn apply_to_source_config(&self, src: &mut SourceConfig) {
        if let FieldKind::Text { value, .. } = &self.custom_name_field {
            let trimmed = value.trim();
            src.name = if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            };
        }

        src.options.clear();
        for (i, (key, field)) in self.option_fields.iter().enumerate() {
            let spec = &self.option_specs[i];
            if let Ok(Some(val)) = field.to_toml_value(spec.multiple) {
                src.options.insert(key.clone(), val);
            }
        }
    }

    pub fn handle_key(
        &mut self,
        key: KeyEvent,
        provider_mgr: Arc<ProviderManager>,
        app_cfg: &mut AppConfig,
        preview_tx: &UnboundedSender<PreviewTaskResult>,
    ) -> bool {
        let total_fields = 1 + self.option_fields.len();

        match key.code {
            KeyCode::Tab => {
                self.focused_field = (self.focused_field + 1) % total_fields;
                false
            }
            KeyCode::BackTab => {
                if self.focused_field == 0 {
                    self.focused_field = total_fields.saturating_sub(1);
                } else {
                    self.focused_field -= 1;
                }
                false
            }
            KeyCode::Char('r')
                if key
                    .modifiers
                    .contains(crossterm::event::KeyModifiers::CONTROL) =>
            {
                // Ctrl+r refreshes previews with current options.
                self.trigger_preview_fetch(provider_mgr, app_cfg, preview_tx.clone());
                true
            }
            KeyCode::Up => {
                if self.focused_field > 0 {
                    let idx = self.focused_field - 1;
                    if self.option_fields[idx].1.is_multiline() {
                        let is_on_first_line = match &self.option_fields[idx].1 {
                            FieldKind::ManyText { value, cursor, .. } => {
                                !value[..*cursor].contains('\n')
                            }
                            _ => true,
                        };
                        if !is_on_first_line {
                            self.option_fields[idx].1.handle_key(key);
                            return true;
                        }
                    }
                }
                if self.focused_field == 0 {
                    self.focused_field = total_fields.saturating_sub(1);
                } else {
                    self.focused_field -= 1;
                }
                false
            }
            KeyCode::Down => {
                if self.focused_field > 0 {
                    let idx = self.focused_field - 1;
                    if self.option_fields[idx].1.is_multiline() {
                        let is_on_last_line = match &self.option_fields[idx].1 {
                            FieldKind::ManyText { value, cursor, .. } => {
                                !value[*cursor..].contains('\n')
                            }
                            _ => true,
                        };
                        if !is_on_last_line {
                            self.option_fields[idx].1.handle_key(key);
                            return true;
                        }
                    }
                }
                self.focused_field = (self.focused_field + 1) % total_fields;
                false
            }
            _ => {
                let changed = if self.focused_field == 0 {
                    self.custom_name_field.handle_key(key)
                } else {
                    let idx = self.focused_field - 1;
                    if idx < self.option_fields.len() {
                        self.option_fields[idx].1.handle_key(key)
                    } else {
                        false
                    }
                };

                if changed && self.source_index < app_cfg.source.len() {
                    self.apply_to_source_config(&mut app_cfg.source[self.source_index]);
                }
                changed
            }
        }
    }

    pub fn render(&mut self, area: Rect, buf: &mut Buffer) {
        let block = Block::default()
            .borders(Borders::ALL)
            .title(format!(" Configure Source: {} ", self.provider_label))
            .style(Style::default().fg(Color::White));
        let inner = block.inner(area);
        block.render(area, buf);

        if inner.width < 20 || inner.height < 10 {
            return;
        }

        // Split into form options (left 55%) and image previews (right 45%).
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
            .split(inner);

        self.render_form(chunks[0], buf);
        self.render_previews(chunks[1], buf);
    }

    fn render_form(&mut self, area: Rect, buf: &mut Buffer) {
        // Calculate vertical layout metrics for all fields.
        let mut field_layouts = Vec::new();
        let mut curr_y = 0;

        // Field 0: Custom Label.
        let custom_h = 2usize; // 1 input line + 1 blank line between fields.
        field_layouts.push((curr_y, custom_h, 1u16));
        curr_y += custom_h;

        // Dynamic option fields.
        for (i, (_, field)) in self.option_fields.iter().enumerate() {
            let is_multi = field.is_multiline();
            let input_h = if is_multi { 4u16 } else { 1u16 };
            let has_desc = self
                .option_specs
                .get(i)
                .and_then(|s| s.description.as_ref())
                .is_some();
            // Input widget lines + (1 line if description present) + 1 blank line between fields.
            let total_h = input_h as usize + if has_desc { 1 } else { 0 } + 1;
            field_layouts.push((curr_y, total_h, input_h));
            curr_y += total_h;
        }

        let total_form_height = curr_y;
        let viewport_height = area.height.saturating_sub(5) as usize;

        // Ensure focused field is visible inside scroll viewport.
        if let Some(&(field_top, field_h, _)) = field_layouts.get(self.focused_field) {
            if field_top < self.form_scroll_offset {
                self.form_scroll_offset = field_top;
            } else if field_top + field_h > self.form_scroll_offset + viewport_height {
                self.form_scroll_offset = (field_top + field_h).saturating_sub(viewport_height);
            }
        }

        // Fixed header at top of form panel.
        buf.set_string(
            area.x + 2,
            area.y + 1,
            format!("Provider: {}", self.provider_label),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        );
        for cell_x in (area.x + 2)..(area.x + area.width.saturating_sub(2)) {
            buf.set_string(
                cell_x,
                area.y + 2,
                "─",
                Style::default().fg(Color::DarkGray),
            );
        }

        let form_body_top = area.y + 3;
        let form_body_bottom = area.y + area.height.saturating_sub(2);

        // Render each field intersecting the visible viewport.
        for (i, &(field_top, field_h, input_h)) in field_layouts.iter().enumerate() {
            let field_screen_y =
                form_body_top as i32 + field_top as i32 - self.form_scroll_offset as i32;
            let field_screen_bottom = field_screen_y + field_h as i32;

            if field_screen_bottom <= form_body_top as i32
                || field_screen_y >= form_body_bottom as i32
            {
                continue;
            }

            let is_focused = self.focused_field == i;
            let pointer = if is_focused { "▶ " } else { "  " };
            let style = if is_focused {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Gray)
            };

            let (label_text, desc_text, field_ref) = if i == 0 {
                ("Custom Label:".to_string(), None, &self.custom_name_field)
            } else {
                let opt_idx = i - 1;
                let spec = &self.option_specs[opt_idx];
                let (_key, field) = &self.option_fields[opt_idx];
                let req_marker = if spec.required { "*" } else { "" };
                let label = format!("{}{}:", spec.label, req_marker);
                let desc = spec.description.clone();

                (label, desc, field)
            };

            // Draw field label.
            if field_screen_y >= form_body_top as i32 && (field_screen_y as u16) < form_body_bottom
            {
                buf.set_string(
                    area.x + 2,
                    field_screen_y as u16,
                    format!("{}{}", pointer, label_text),
                    style,
                );
            }

            // Draw input widget area.
            let input_y = field_screen_y;
            if input_y >= form_body_top as i32 && (input_y as u16) < form_body_bottom {
                let input_area = Rect {
                    x: area.x + 24,
                    y: input_y as u16,
                    width: area.width.saturating_sub(26).min(30),
                    height: input_h.min(form_body_bottom - input_y as u16),
                };
                field_ref.render(input_area, buf, is_focused);
            }

            // Draw description text aligned with the label, immediately under the widget (no newline).
            if let Some(desc) = desc_text {
                let desc_y = field_screen_y + input_h as i32;
                if desc_y >= form_body_top as i32 && (desc_y as u16) < form_body_bottom {
                    let desc_area_w = area.width.saturating_sub(6) as usize;
                    let truncated_desc = if desc.len() > desc_area_w {
                        format!("{:.w$}…", desc, w = desc_area_w.saturating_sub(1))
                    } else {
                        desc
                    };
                    buf.set_string(
                        area.x + 4,
                        desc_y as u16,
                        truncated_desc,
                        Style::default().fg(Color::DarkGray),
                    );
                }
            }
        }

        // Draw scroll indicators if total form lines exceed viewport.
        if total_form_height > viewport_height {
            let indicator_x = area.x + area.width.saturating_sub(2);
            if self.form_scroll_offset > 0 {
                buf.set_string(
                    indicator_x,
                    form_body_top,
                    "▲",
                    Style::default().fg(Color::Cyan),
                );
            }
            if self.form_scroll_offset + viewport_height < total_form_height {
                buf.set_string(
                    indicator_x,
                    form_body_bottom.saturating_sub(1),
                    "▼",
                    Style::default().fg(Color::Cyan),
                );
            }
        }

        // Form footer keybindings.
        let hint_y = area.y + area.height.saturating_sub(1);
        buf.set_string(
            area.x + 2,
            hint_y,
            "[Esc] Done   [^R] Refresh Previews   [Tab/↑/↓] Navigate",
            Style::default().fg(Color::DarkGray),
        );
    }

    fn render_previews(&mut self, area: Rect, buf: &mut Buffer) {
        let block = Block::default()
            .borders(Borders::LEFT)
            .title(" Next 3 Wallpapers Preview ")
            .style(Style::default().fg(Color::White));
        let inner = block.inner(area);
        block.render(area, buf);

        if self.is_loading_previews {
            buf.set_string(
                inner.x + 2,
                inner.y + 2,
                "Loading 3 preview wallpapers from provider...",
                Style::default().fg(Color::Yellow),
            );
            return;
        }

        if let Some(ref err) = self.preview_error {
            buf.set_string(
                inner.x + 2,
                inner.y + 2,
                format!("⚠ {}", err),
                Style::default().fg(Color::Red),
            );
            return;
        }

        if self.previews.is_empty() {
            buf.set_string(
                inner.x + 2,
                inner.y + 2,
                "Press Ctrl+R to fetch previews for this source.",
                Style::default().fg(Color::DarkGray),
            );
            return;
        }

        // Layout the 3 preview slots vertically.
        let slot_height = inner.height / 3;
        for (idx, item) in self.previews.iter_mut().enumerate() {
            if idx >= 3 {
                break;
            }
            let slot_y = inner.y + (idx as u16 * slot_height);
            let slot_area = Rect {
                x: inner.x + 2,
                y: slot_y,
                width: inner.width.saturating_sub(4),
                height: slot_height.saturating_sub(1),
            };

            if slot_area.height < 3 || slot_area.width < 10 {
                continue;
            }

            // Split slot into image area (left) and metadata (right).
            let image_width = slot_area.width.min(22);
            let img_area = Rect {
                x: slot_area.x,
                y: slot_area.y,
                width: image_width,
                height: slot_area.height,
            };

            let meta_area = Rect {
                x: slot_area.x + image_width + 1,
                y: slot_area.y,
                width: slot_area.width.saturating_sub(image_width + 1),
                height: slot_area.height,
            };

            // Render ratatui-image.
            let widget = StatefulImage::default();
            ratatui::widgets::StatefulWidget::render(widget, img_area, buf, &mut item.protocol);

            // Metadata text.
            let title = item.title.as_deref().unwrap_or(&item.id);
            buf.set_string(
                meta_area.x,
                meta_area.y,
                format!("{}. {}", idx + 1, title),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            );

            if let Some(ref author) = item.author {
                buf.set_string(
                    meta_area.x,
                    meta_area.y + 1,
                    format!("by {}", author),
                    Style::default().fg(Color::Gray),
                );
            }

            if let (Some(w), Some(h)) = (item.width, item.height) {
                buf.set_string(
                    meta_area.x,
                    meta_area.y + 2,
                    format!("{}x{}", w, h),
                    Style::default().fg(Color::DarkGray),
                );
            }
        }
    }
}
