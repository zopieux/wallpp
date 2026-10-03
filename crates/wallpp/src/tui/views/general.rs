use crossterm::event::{KeyCode, KeyEvent};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Borders, Widget};

use crate::config::{ByteSize, ManagerConfig, RefreshStrategy, SourceStrategy};
use crate::tui::field::FieldKind;

/// General manager configuration form view.
pub struct GeneralView {
    pub selected_field: usize,
    pub cache_max_size: FieldKind,
    pub history_length: FieldKind,
    pub prefetch_count: FieldKind,
    pub refresh_interval: FieldKind,
    pub refresh_at_boot: FieldKind,
    pub refresh_strategy: FieldKind,
    pub source_strategy: FieldKind,
    pub min_display_percentage: FieldKind,
    pub validation_error: Option<String>,
}

impl GeneralView {
    pub fn new(cfg: &ManagerConfig) -> Self {
        let mut view = Self {
            selected_field: 0,
            cache_max_size: FieldKind::ByteSize {
                value: cfg.cache_max_size.to_string(),
                cursor: cfg.cache_max_size.to_string().len(),
            },
            history_length: FieldKind::Integer {
                value: cfg.history_length.to_string(),
                min: Some(1),
                max: Some(100_000),
                cursor: cfg.history_length.to_string().len(),
            },
            prefetch_count: FieldKind::Integer {
                value: cfg.prefetch_count.to_string(),
                min: Some(0),
                max: Some(100),
                cursor: cfg.prefetch_count.to_string().len(),
            },
            refresh_interval: FieldKind::Duration {
                value: cfg
                    .refresh_interval
                    .map(|d| humantime::format_duration(d).to_string())
                    .unwrap_or_else(|| "none".to_string()),
                cursor: 0,
            },
            refresh_at_boot: FieldKind::Boolean {
                value: cfg.refresh_at_boot,
            },
            refresh_strategy: FieldKind::Choice {
                choices: vec!["from_boot".to_string(), "from_last_changed".to_string()],
                selected: match cfg.refresh_strategy {
                    RefreshStrategy::FromBoot => 0,
                    RefreshStrategy::FromLastChanged => 1,
                },
            },
            source_strategy: FieldKind::Choice {
                choices: vec!["random".to_string(), "round_robin".to_string()],
                selected: match cfg.source_strategy {
                    SourceStrategy::Random => 0,
                    SourceStrategy::RoundRobin => 1,
                },
            },
            min_display_percentage: FieldKind::Integer {
                value: cfg.min_display_percentage.to_string(),
                min: Some(1),
                max: Some(500),
                cursor: cfg.min_display_percentage.to_string().len(),
            },
            validation_error: None,
        };
        if let FieldKind::Duration { value, cursor } = &mut view.refresh_interval {
            *cursor = value.len();
        }
        view
    }

    /// Validates and applies form inputs to the in-memory ManagerConfig.
    pub fn apply_to_config(&mut self, cfg: &mut ManagerConfig) -> Result<(), String> {
        // Cache Max Size.
        if let FieldKind::ByteSize { value, .. } = &self.cache_max_size {
            let parsed: ByteSize = toml::from_str(&format!("v = \"{}\"", value.trim()))
                .map(|m: std::collections::HashMap<String, ByteSize>| m["v"])
                .map_err(|e| format!("Invalid cache size '{}': {}", value, e))?;
            cfg.cache_max_size = parsed;
        }

        // History Length.
        if let FieldKind::Integer { value, .. } = &self.history_length {
            let parsed: usize = value
                .trim()
                .parse()
                .map_err(|e| format!("Invalid history length '{}': {}", value, e))?;
            if parsed == 0 {
                return Err("History length must be at least 1".to_string());
            }
            cfg.history_length = parsed;
        }

        // Prefetch Count.
        if let FieldKind::Integer { value, .. } = &self.prefetch_count {
            let parsed: usize = value
                .trim()
                .parse()
                .map_err(|e| format!("Invalid prefetch count '{}': {}", value, e))?;
            cfg.prefetch_count = parsed;
        }

        // Refresh Interval.
        if let FieldKind::Duration { value, .. } = &self.refresh_interval {
            let trimmed = value.trim();
            if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("none") {
                cfg.refresh_interval = None;
            } else {
                let parsed = humantime::parse_duration(trimmed)
                    .map_err(|e| format!("Invalid duration '{}': {}", trimmed, e))?;
                cfg.refresh_interval = Some(parsed);
            }
        }

        // Refresh at Boot.
        if let FieldKind::Boolean { value } = &self.refresh_at_boot {
            cfg.refresh_at_boot = *value;
        }

        // Refresh Strategy.
        if let FieldKind::Choice { selected, .. } = &self.refresh_strategy {
            cfg.refresh_strategy = if *selected == 0 {
                RefreshStrategy::FromBoot
            } else {
                RefreshStrategy::FromLastChanged
            };
        }

        // Source Strategy.
        if let FieldKind::Choice { selected, .. } = &self.source_strategy {
            cfg.source_strategy = if *selected == 0 {
                SourceStrategy::Random
            } else {
                SourceStrategy::RoundRobin
            };
        }

        // Min Display Percentage.
        if let FieldKind::Integer { value, .. } = &self.min_display_percentage {
            let parsed: u32 = value
                .trim()
                .parse()
                .map_err(|e| format!("Invalid percentage '{}': {}", value, e))?;
            cfg.min_display_percentage = parsed;
        }

        self.validation_error = None;
        Ok(())
    }

    pub fn handle_key(&mut self, key: KeyEvent, cfg: &mut ManagerConfig) -> bool {
        match key.code {
            KeyCode::Up => {
                if self.selected_field == 0 {
                    self.selected_field = 7;
                } else {
                    self.selected_field -= 1;
                }
                false
            }
            KeyCode::Down => {
                self.selected_field = (self.selected_field + 1) % 8;
                false
            }
            KeyCode::Tab => {
                self.selected_field = (self.selected_field + 1) % 8;
                false
            }
            KeyCode::BackTab => {
                if self.selected_field == 0 {
                    self.selected_field = 7;
                } else {
                    self.selected_field -= 1;
                }
                false
            }
            _ => {
                let field = match self.selected_field {
                    0 => &mut self.cache_max_size,
                    1 => &mut self.history_length,
                    2 => &mut self.prefetch_count,
                    3 => &mut self.refresh_interval,
                    4 => &mut self.refresh_at_boot,
                    5 => &mut self.refresh_strategy,
                    6 => &mut self.source_strategy,
                    7 => &mut self.min_display_percentage,
                    _ => return false,
                };
                let changed = field.handle_key(key);
                if changed {
                    if let Err(e) = self.apply_to_config(cfg) {
                        self.validation_error = Some(e);
                    } else {
                        self.validation_error = None;
                    }
                }
                changed
            }
        }
    }

    pub fn handle_paste(&mut self, text: &str, cfg: &mut ManagerConfig) -> bool {
        let field = match self.selected_field {
            0 => &mut self.cache_max_size,
            1 => &mut self.history_length,
            2 => &mut self.prefetch_count,
            3 => &mut self.refresh_interval,
            4 => &mut self.refresh_at_boot,
            5 => &mut self.refresh_strategy,
            6 => &mut self.source_strategy,
            7 => &mut self.min_display_percentage,
            _ => return false,
        };
        let changed = field.handle_paste(text);
        if changed {
            if let Err(e) = self.apply_to_config(cfg) {
                self.validation_error = Some(e);
            } else {
                self.validation_error = None;
            }
        }
        changed
    }

    pub fn render(&self, area: Rect, buf: &mut Buffer) {
        let block = Block::default()
            .borders(Borders::ALL)
            .title(" General Settings ")
            .style(Style::default().fg(Color::White));
        let inner = block.inner(area);
        block.render(area, buf);

        if inner.width < 10 || inner.height < 10 {
            return;
        }

        let field_names = [
            (
                "Cache Max Size",
                "Maximum local cache quota (e.g. 1G, 500M)",
            ),
            ("History Length", "Number of past wallpapers recorded"),
            (
                "Prefetch Count",
                "Number of future wallpapers to cache in queue",
            ),
            (
                "Refresh Interval",
                "Periodic timer duration (e.g. 30m, 2h, or none)",
            ),
            (
                "Refresh at Boot",
                "Whether to refresh the wallpaper on daemon startup",
            ),
            (
                "Refresh Strategy",
                "Base time for periodic interval refreshes",
            ),
            (
                "Source Strategy",
                "Selection strategy among configured sources",
            ),
            (
                "Min Display %",
                "Minimum percentage of largest display required (1-500%)",
            ),
        ];

        let mut y = inner.y + 1;
        for (i, (label, desc)) in field_names.iter().enumerate() {
            if y + 2 >= inner.y + inner.height {
                break;
            }
            let is_focused = self.selected_field == i;
            let label_style = if is_focused {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Gray)
            };

            let pointer = if is_focused { "▶ " } else { "  " };
            buf.set_string(inner.x + 1, y, format!("{}{}", pointer, label), label_style);

            let input_area = Rect {
                x: inner.x + 28,
                y,
                width: inner.width.saturating_sub(30).min(35),
                height: 1,
            };

            let field = match i {
                0 => &self.cache_max_size,
                1 => &self.history_length,
                2 => &self.prefetch_count,
                3 => &self.refresh_interval,
                4 => &self.refresh_at_boot,
                5 => &self.refresh_strategy,
                6 => &self.source_strategy,
                7 => &self.min_display_percentage,
                _ => unreachable!(),
            };

            // Subtle input box background when focused.
            if is_focused {
                let bg_style = Style::default().bg(Color::Rgb(25, 25, 35));
                for cell_x in input_area.x..input_area.x + input_area.width {
                    buf.set_string(cell_x, input_area.y, " ", bg_style);
                }
            }

            field.render(input_area, buf, is_focused);

            // Description hint line aligned with label text (no newline between label/input and description).
            let desc_area_w = inner.width.saturating_sub(5) as usize;
            let truncated_desc = if desc.len() > desc_area_w {
                format!("{:.w$}…", desc, w = desc_area_w.saturating_sub(1))
            } else {
                desc.to_string()
            };
            buf.set_string(
                inner.x + 3,
                y + 1,
                truncated_desc,
                Style::default().fg(Color::DarkGray),
            );

            // Advance by 3 rows: row y (label/input), row y+1 (desc), row y+2 (blank line between fields).
            y += 3;
        }

        if let Some(ref err) = self.validation_error {
            let err_y = (inner.y + inner.height).saturating_sub(2);
            buf.set_string(
                inner.x + 2,
                err_y,
                format!("⚠ {}", err),
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            );
        }
    }
}
