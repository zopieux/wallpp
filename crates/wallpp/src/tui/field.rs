use crossterm::event::{KeyCode, KeyEvent};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};

use crate::config::ByteSize;
use crate::wallpp::provider::types::{ConfigValue, IntBounds, OptionSpec, ScalarType, ScalarValue};

/// Represents an editable field in form editors.
#[derive(Debug, Clone)]
pub enum FieldKind {
    Text {
        value: String,
        cursor: usize,
    },
    Secret {
        value: String,
        cursor: usize,
    },
    Integer {
        value: String,
        min: Option<i64>,
        max: Option<i64>,
        cursor: usize,
    },
    Boolean {
        value: bool,
    },
    Choice {
        choices: Vec<String>,
        selected: usize,
    },
    Duration {
        value: String,
        cursor: usize,
    },
    ByteSize {
        value: String,
        cursor: usize,
    },
    ManyText {
        value: String,
        cursor: usize,
        scroll_offset: usize,
    },
}

impl FieldKind {
    /// Returns true if this field is a multiline textarea.
    pub fn is_multiline(&self) -> bool {
        matches!(self, FieldKind::ManyText { .. })
    }

    /// Constructs a field matching a WIT OptionSpec and existing TOML value.
    pub fn from_option_spec(spec: &OptionSpec, existing: Option<&toml::Value>) -> Self {
        if spec.multiple {
            let initial = match existing {
                Some(toml::Value::Array(arr)) => arr
                    .iter()
                    .filter_map(|v| match v {
                        toml::Value::String(s) => Some(s.clone()),
                        toml::Value::Integer(i) => Some(i.to_string()),
                        toml::Value::Boolean(b) => Some(b.to_string()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n"),
                Some(toml::Value::String(s)) => s.replace(',', "\n"),
                _ => match &spec.default {
                    Some(ConfigValue::Many(items)) => items
                        .iter()
                        .filter_map(|item| match item {
                            ScalarValue::Text(s) | ScalarValue::Choice(s) => Some(s.clone()),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                        .join("\n"),
                    _ => String::new(),
                },
            };
            let cursor = initial.len();
            FieldKind::ManyText {
                value: initial,
                cursor,
                scroll_offset: 0,
            }
        } else {
            match &spec.ty {
                ScalarType::Text => {
                    let val = match existing {
                        Some(toml::Value::String(s)) => s.clone(),
                        _ => String::new(),
                    };
                    let cursor = val.len();
                    FieldKind::Text { value: val, cursor }
                }
                ScalarType::Secret => {
                    let val = match existing {
                        Some(toml::Value::String(s)) => s.clone(),
                        _ => String::new(),
                    };
                    let cursor = val.len();
                    FieldKind::Secret { value: val, cursor }
                }
                ScalarType::Integer(IntBounds { min, max }) => {
                    let val = match existing {
                        Some(toml::Value::Integer(i)) => i.to_string(),
                        _ => min.unwrap_or(0).to_string(),
                    };
                    let cursor = val.len();
                    FieldKind::Integer {
                        value: val,
                        min: *min,
                        max: *max,
                        cursor,
                    }
                }
                ScalarType::Boolean => {
                    let val = match existing {
                        Some(toml::Value::Boolean(b)) => *b,
                        _ => false,
                    };
                    FieldKind::Boolean { value: val }
                }
                ScalarType::Choice(choices) => {
                    let selected = match existing {
                        Some(toml::Value::String(s)) => {
                            choices.iter().position(|c| c == s).unwrap_or(0)
                        }
                        _ => 0,
                    };
                    FieldKind::Choice {
                        choices: choices.clone(),
                        selected,
                    }
                }
            }
        }
    }

    /// Handles a keyboard event when this field is focused.
    /// Returns true if the field value changed.
    pub fn handle_key(&mut self, key: KeyEvent) -> bool {
        match self {
            FieldKind::Text { value, cursor }
            | FieldKind::Secret { value, cursor }
            | FieldKind::Duration { value, cursor }
            | FieldKind::ByteSize { value, cursor } => match key.code {
                KeyCode::Char(c) => {
                    value.insert(*cursor, c);
                    *cursor += c.len_utf8();
                    true
                }
                KeyCode::Backspace => {
                    if *cursor > 0 {
                        let prev_idx = value[..*cursor]
                            .char_indices()
                            .next_back()
                            .map(|(i, _)| i)
                            .unwrap_or(0);
                        value.remove(prev_idx);
                        *cursor = prev_idx;
                        true
                    } else {
                        false
                    }
                }
                KeyCode::Delete => {
                    if *cursor < value.len() {
                        value.remove(*cursor);
                        true
                    } else {
                        false
                    }
                }
                KeyCode::Left => {
                    if *cursor > 0 {
                        *cursor = value[..*cursor]
                            .char_indices()
                            .next_back()
                            .map(|(i, _)| i)
                            .unwrap_or(0);
                    }
                    false
                }
                KeyCode::Right => {
                    if *cursor < value.len() {
                        *cursor = value[*cursor..]
                            .char_indices()
                            .nth(1)
                            .map(|(i, _)| *cursor + i)
                            .unwrap_or(value.len());
                    }
                    false
                }
                KeyCode::Home => {
                    *cursor = 0;
                    false
                }
                KeyCode::End => {
                    *cursor = value.len();
                    false
                }
                _ => false,
            },
            FieldKind::ManyText {
                value,
                cursor,
                scroll_offset,
            } => match key.code {
                KeyCode::Char(c) if c == ',' || c == '+' => {
                    value.insert(*cursor, '\n');
                    *cursor += 1;
                    adjust_multiline_scroll(value, *cursor, scroll_offset, 4);
                    true
                }
                KeyCode::Char(c) => {
                    value.insert(*cursor, c);
                    *cursor += c.len_utf8();
                    adjust_multiline_scroll(value, *cursor, scroll_offset, 4);
                    true
                }
                KeyCode::Enter => {
                    value.insert(*cursor, '\n');
                    *cursor += 1;
                    adjust_multiline_scroll(value, *cursor, scroll_offset, 4);
                    true
                }
                KeyCode::Backspace => {
                    if *cursor > 0 {
                        let prev_idx = value[..*cursor]
                            .char_indices()
                            .next_back()
                            .map(|(i, _)| i)
                            .unwrap_or(0);
                        value.remove(prev_idx);
                        *cursor = prev_idx;
                        adjust_multiline_scroll(value, *cursor, scroll_offset, 4);
                        true
                    } else {
                        false
                    }
                }
                KeyCode::Delete => {
                    if *cursor < value.len() {
                        value.remove(*cursor);
                        adjust_multiline_scroll(value, *cursor, scroll_offset, 4);
                        true
                    } else {
                        false
                    }
                }
                KeyCode::Left => {
                    if *cursor > 0 {
                        *cursor = value[..*cursor]
                            .char_indices()
                            .next_back()
                            .map(|(i, _)| i)
                            .unwrap_or(0);
                        adjust_multiline_scroll(value, *cursor, scroll_offset, 4);
                    }
                    false
                }
                KeyCode::Right => {
                    if *cursor < value.len() {
                        *cursor = value[*cursor..]
                            .char_indices()
                            .nth(1)
                            .map(|(i, _)| *cursor + i)
                            .unwrap_or(value.len());
                        adjust_multiline_scroll(value, *cursor, scroll_offset, 4);
                    }
                    false
                }
                KeyCode::Up => {
                    if let Some(new_cursor) = move_multiline_up(value, *cursor) {
                        *cursor = new_cursor;
                        adjust_multiline_scroll(value, *cursor, scroll_offset, 4);
                        false
                    } else {
                        // At top line, let container navigate to previous field.
                        false
                    }
                }
                KeyCode::Down => {
                    if let Some(new_cursor) = move_multiline_down(value, *cursor) {
                        *cursor = new_cursor;
                        adjust_multiline_scroll(value, *cursor, scroll_offset, 4);
                        false
                    } else {
                        // At bottom line, let container navigate to next field.
                        false
                    }
                }
                KeyCode::Home => {
                    let start_of_line = value[..*cursor].rfind('\n').map(|i| i + 1).unwrap_or(0);
                    *cursor = start_of_line;
                    false
                }
                KeyCode::End => {
                    let end_of_line = value[*cursor..]
                        .find('\n')
                        .map(|i| *cursor + i)
                        .unwrap_or(value.len());
                    *cursor = end_of_line;
                    false
                }
                _ => false,
            },
            FieldKind::Integer {
                value,
                min,
                max,
                cursor,
            } => match key.code {
                KeyCode::Char(c) if c.is_ascii_digit() || (c == '-' && *cursor == 0) => {
                    value.insert(*cursor, c);
                    *cursor += 1;
                    true
                }
                KeyCode::Backspace => {
                    if *cursor > 0 {
                        value.remove(*cursor - 1);
                        *cursor -= 1;
                        true
                    } else {
                        false
                    }
                }
                KeyCode::Delete => {
                    if *cursor < value.len() {
                        value.remove(*cursor);
                        true
                    } else {
                        false
                    }
                }
                KeyCode::Left => {
                    if *cursor > 0 {
                        *cursor -= 1;
                    }
                    false
                }
                KeyCode::Right => {
                    if *cursor < value.len() {
                        *cursor += 1;
                    }
                    false
                }
                KeyCode::Up => {
                    let mut curr = value.parse::<i64>().unwrap_or(0);
                    curr = curr.saturating_add(1);
                    if let Some(m) = max {
                        curr = curr.min(*m);
                    }
                    *value = curr.to_string();
                    *cursor = value.len();
                    true
                }
                KeyCode::Down => {
                    let mut curr = value.parse::<i64>().unwrap_or(0);
                    curr = curr.saturating_sub(1);
                    if let Some(m) = min {
                        curr = curr.max(*m);
                    }
                    *value = curr.to_string();
                    *cursor = value.len();
                    true
                }
                _ => false,
            },
            FieldKind::Boolean { value } => match key.code {
                KeyCode::Char(' ') | KeyCode::Enter => {
                    *value = !*value;
                    true
                }
                KeyCode::Left => {
                    let changed = *value;
                    *value = false;
                    changed
                }
                KeyCode::Right => {
                    let changed = !*value;
                    *value = true;
                    changed
                }
                _ => false,
            },
            FieldKind::Choice { choices, selected } => match key.code {
                KeyCode::Left | KeyCode::Up if !choices.is_empty() => {
                    if *selected == 0 {
                        *selected = choices.len() - 1;
                    } else {
                        *selected -= 1;
                    }
                    true
                }
                KeyCode::Right | KeyCode::Down | KeyCode::Char(' ') | KeyCode::Enter
                    if !choices.is_empty() =>
                {
                    *selected = (*selected + 1) % choices.len();
                    true
                }
                _ => false,
            },
        }
    }

    /// Handles pasted text into the field.
    pub fn handle_paste(&mut self, text: &str) -> bool {
        match self {
            FieldKind::ManyText {
                value,
                cursor,
                scroll_offset,
            } => {
                let normalized = normalize_multiline_paste(text);
                if normalized.is_empty() {
                    return false;
                }
                value.insert_str(*cursor, &normalized);
                *cursor += normalized.len();
                adjust_multiline_scroll(value, *cursor, scroll_offset, 4);
                true
            }
            FieldKind::Text { value, cursor }
            | FieldKind::Secret { value, cursor }
            | FieldKind::Duration { value, cursor }
            | FieldKind::ByteSize { value, cursor } => {
                let sanitized: String = text.chars().filter(|&c| c != '\r' && c != '\n').collect();
                if sanitized.is_empty() {
                    return false;
                }
                value.insert_str(*cursor, &sanitized);
                *cursor += sanitized.len();
                true
            }
            FieldKind::Integer { value, cursor, .. } => {
                let sanitized: String = text
                    .chars()
                    .filter(|c| c.is_ascii_digit() || *c == '-')
                    .collect();
                if sanitized.is_empty() {
                    return false;
                }
                value.insert_str(*cursor, &sanitized);
                *cursor += sanitized.len();
                true
            }
            FieldKind::Boolean { .. } | FieldKind::Choice { .. } => false,
        }
    }

    /// Converts the current editor field content to a TOML value.
    pub fn to_toml_value(&self, multiple: bool) -> Result<Option<toml::Value>, String> {
        if multiple {
            match self {
                FieldKind::ManyText { value, .. } => {
                    let items: Vec<toml::Value> = value
                        .lines()
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .map(toml::Value::String)
                        .collect();
                    if items.is_empty() {
                        Ok(None)
                    } else {
                        Ok(Some(toml::Value::Array(items)))
                    }
                }
                _ => Ok(None),
            }
        } else {
            match self {
                FieldKind::Text { value, .. } => {
                    if value.trim().is_empty() {
                        Ok(None)
                    } else {
                        Ok(Some(toml::Value::String(value.clone())))
                    }
                }
                FieldKind::Secret { value, .. } => {
                    if value.trim().is_empty() {
                        Ok(None)
                    } else {
                        Ok(Some(toml::Value::String(value.clone())))
                    }
                }
                FieldKind::Integer {
                    value, min, max, ..
                } => {
                    if value.trim().is_empty() {
                        return Ok(None);
                    }
                    let parsed: i64 = value
                        .trim()
                        .parse()
                        .map_err(|e| format!("Invalid integer '{}': {}", value, e))?;
                    if let Some(m) = min {
                        if parsed < *m {
                            return Err(format!("Value {} is less than minimum {}", parsed, m));
                        }
                    }
                    if let Some(m) = max {
                        if parsed > *m {
                            return Err(format!("Value {} is greater than maximum {}", parsed, m));
                        }
                    }
                    Ok(Some(toml::Value::Integer(parsed)))
                }
                FieldKind::Boolean { value } => Ok(Some(toml::Value::Boolean(*value))),
                FieldKind::Choice { choices, selected } => {
                    if choices.is_empty() {
                        Ok(None)
                    } else {
                        Ok(Some(toml::Value::String(choices[*selected].clone())))
                    }
                }
                FieldKind::Duration { value, .. } => {
                    let trimmed = value.trim();
                    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("none") {
                        Ok(None)
                    } else {
                        humantime::parse_duration(trimmed)
                            .map_err(|e| format!("Invalid duration '{}': {}", trimmed, e))?;
                        Ok(Some(toml::Value::String(trimmed.to_string())))
                    }
                }
                FieldKind::ByteSize { value, .. } => {
                    let trimmed = value.trim();
                    if trimmed.is_empty() {
                        Ok(None)
                    } else {
                        let _parsed: ByteSize = toml::from_str(&format!("val = \"{}\"", trimmed))
                            .map(|v: std::collections::HashMap<String, ByteSize>| v["val"])
                            .map_err(|e| format!("Invalid byte size '{}': {}", trimmed, e))?;
                        Ok(Some(toml::Value::String(trimmed.to_string())))
                    }
                }
                FieldKind::ManyText { .. } => Ok(None),
            }
        }
    }

    /// Renders the editable field within the allocated area.
    pub fn render(&self, area: Rect, buf: &mut Buffer, focused: bool) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        let base_style = if focused {
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };

        match self {
            FieldKind::Text { value, cursor }
            | FieldKind::Duration { value, cursor }
            | FieldKind::ByteSize { value, cursor } => {
                render_text_with_cursor(value, *cursor, area, buf, focused, base_style);
            }
            FieldKind::ManyText {
                value,
                cursor,
                scroll_offset,
            } => {
                render_multiline_with_cursor(
                    value,
                    *cursor,
                    *scroll_offset,
                    area,
                    buf,
                    focused,
                    base_style,
                );
            }
            FieldKind::Secret { value, cursor } => {
                let masked = "•".repeat(value.chars().count());
                render_text_with_cursor(&masked, *cursor, area, buf, focused, base_style);
            }
            FieldKind::Integer { value, cursor, .. } => {
                render_text_with_cursor(value, *cursor, area, buf, focused, base_style);
            }
            FieldKind::Boolean { value } => {
                let symbol = if *value { "[x]" } else { "[ ]" };
                let style = if focused {
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD)
                } else if *value {
                    Style::default().fg(Color::Green)
                } else {
                    Style::default().fg(Color::DarkGray)
                };
                buf.set_string(area.x, area.y, symbol, style);
            }
            FieldKind::Choice { choices, selected } => {
                let text = if choices.is_empty() {
                    "< none >".to_string()
                } else {
                    format!("< {} >", choices[*selected])
                };
                let style = if focused {
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::Green)
                };
                buf.set_string(area.x, area.y, text, style);
            }
        }
    }
}

fn move_multiline_up(text: &str, cursor: usize) -> Option<usize> {
    let line_start = text[..cursor].rfind('\n').map(|i| i + 1).unwrap_or(0);
    if line_start == 0 {
        return None;
    }
    let col = cursor - line_start;
    let prev_line_end = line_start - 1;
    let prev_line_start = text[..prev_line_end]
        .rfind('\n')
        .map(|i| i + 1)
        .unwrap_or(0);
    let prev_line_len = prev_line_end - prev_line_start;
    Some(prev_line_start + col.min(prev_line_len))
}

fn move_multiline_down(text: &str, cursor: usize) -> Option<usize> {
    let line_start = text[..cursor].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let col = cursor - line_start;
    let next_line_start = cursor + text[cursor..].find('\n')? + 1;
    let next_line_len = text[next_line_start..]
        .find('\n')
        .unwrap_or(text.len() - next_line_start);
    Some(next_line_start + col.min(next_line_len))
}

fn get_cursor_line_and_col(text: &str, cursor: usize) -> (usize, usize) {
    let line_idx = text[..cursor].chars().filter(|&c| c == '\n').count();
    let line_start = text[..cursor].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let col = cursor - line_start;
    (line_idx, col)
}

fn adjust_multiline_scroll(
    text: &str,
    cursor: usize,
    scroll_offset: &mut usize,
    visible_rows: usize,
) {
    let (line_idx, _) = get_cursor_line_and_col(text, cursor);
    if line_idx < *scroll_offset {
        *scroll_offset = line_idx;
    } else if line_idx >= *scroll_offset + visible_rows {
        *scroll_offset = line_idx - visible_rows + 1;
    }
}

fn render_multiline_with_cursor(
    text: &str,
    cursor: usize,
    scroll_offset: usize,
    area: Rect,
    buf: &mut Buffer,
    focused: bool,
    style: Style,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let bg_style = if focused {
        Style::default().bg(Color::Rgb(25, 25, 38))
    } else {
        Style::default().bg(Color::Rgb(18, 18, 26))
    };

    // Fill background area.
    for y in area.y..area.y + area.height {
        for x in area.x..area.x + area.width {
            buf.set_string(x, y, " ", bg_style);
        }
    }

    let lines: Vec<&str> = text.split('\n').collect();
    let (cursor_line, cursor_col) = get_cursor_line_and_col(text, cursor);
    let visible_rows = area.height as usize;

    for row in 0..visible_rows {
        let line_idx = scroll_offset + row;
        let line_y = area.y + row as u16;

        if line_idx < lines.len() {
            let line_content = lines[line_idx];
            let is_cursor_line = focused && line_idx == cursor_line;

            let mut x = area.x + 1;
            let max_x = area.x + area.width.saturating_sub(1);

            if line_content.is_empty() && is_cursor_line && x < max_x {
                buf.set_string(
                    x,
                    line_y,
                    " ",
                    Style::default().bg(Color::White).fg(Color::Black),
                );
            } else {
                for (idx, ch) in line_content.char_indices() {
                    if x >= max_x {
                        break;
                    }
                    let is_cursor = is_cursor_line && idx == cursor_col;
                    let char_style = if is_cursor {
                        Style::default().bg(Color::White).fg(Color::Black)
                    } else {
                        style.bg(bg_style.bg.unwrap_or(Color::Reset))
                    };
                    buf.set_string(x, line_y, ch.to_string(), char_style);
                    x += 1;
                }
                if is_cursor_line && cursor_col >= line_content.len() && x < max_x {
                    buf.set_string(
                        x,
                        line_y,
                        " ",
                        Style::default().bg(Color::White).fg(Color::Black),
                    );
                }
            }
        }
    }

    // Scroll indicator if total lines exceed visible area.
    if lines.len() > visible_rows {
        let indicator_x = area.x + area.width.saturating_sub(1);
        if scroll_offset > 0 {
            buf.set_string(indicator_x, area.y, "▲", Style::default().fg(Color::Cyan));
        }
        if scroll_offset + visible_rows < lines.len() {
            buf.set_string(
                indicator_x,
                area.y + area.height.saturating_sub(1),
                "▼",
                Style::default().fg(Color::Cyan),
            );
        }
    }
}

fn render_text_with_cursor(
    text: &str,
    cursor: usize,
    area: Rect,
    buf: &mut Buffer,
    focused: bool,
    style: Style,
) {
    let mut x = area.x;
    let max_x = area.x + area.width;

    if text.is_empty() {
        if focused && x < max_x {
            buf.set_string(
                x,
                area.y,
                " ",
                Style::default().bg(Color::White).fg(Color::Black),
            );
        }
        return;
    }

    for (idx, ch) in text.char_indices() {
        if x >= max_x {
            break;
        }
        let is_cursor = focused && idx == cursor;
        let char_style = if is_cursor {
            Style::default().bg(Color::White).fg(Color::Black)
        } else {
            style
        };
        buf.set_string(x, area.y, ch.to_string(), char_style);
        x += 1;
    }

    if focused && cursor >= text.len() && x < max_x {
        buf.set_string(
            x,
            area.y,
            " ",
            Style::default().bg(Color::White).fg(Color::Black),
        );
    }
}

/// Normalizes a pasted list into one-item-per-line (converting spaces, commas, pluses, newlines to \n).
fn normalize_multiline_paste(text: &str) -> String {
    let tokens: Vec<&str> = text
        .split(|c: char| c == ',' || c == '+' || c.is_whitespace())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();
    tokens.join("\n")
}
