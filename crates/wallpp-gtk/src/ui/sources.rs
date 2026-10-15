use gtk4::prelude::*;
use gtk4::{
    gdk, glib, Align, Box, Button, CheckButton, DropDown, Entry, EventControllerFocus,
    EventControllerKey, Label, ListBox, ListBoxRow, Orientation, Overlay, PasswordEntry, Picture,
    Popover, ScrolledWindow, SpinButton, Spinner, TextView, WrapMode,
};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use wallpp::config::{AppConfig, SourceConfig};
use wallpp::provider::DiscoveredProvider;
use wallpp::wallpp::provider::types::{
    ConfigValue, IntBounds, OptionSpec, ScalarType, ScalarValue,
};

use super::timeline_grid::TimelineGrid;
use crate::backend::{Action, BackendHandle, SourcePreviewItem};

enum FormField {
    Text(Entry),
    Secret(PasswordEntry),
    Integer(SpinButton),
    Boolean(CheckButton),
    Choice {
        dropdown: DropDown,
        choices: Vec<String>,
    },
    MultipleChoice(Vec<(String, CheckButton)>),
    MultipleText(TextView),
}

impl FormField {
    fn apply_to_source(&self, key: &str, source: &mut SourceConfig) {
        match self {
            FormField::Text(entry) => {
                source.set_option_string(key, entry.text().as_str());
            }
            FormField::Secret(entry) => {
                source.set_option_string(key, entry.text().as_str());
            }
            FormField::Integer(spin) => {
                source.set_option_int(key, spin.value() as i64);
            }
            FormField::Boolean(chk) => {
                source.set_option_bool(key, chk.is_active());
            }
            FormField::Choice { dropdown, choices } => {
                let idx = dropdown.selected() as usize;
                if let Some(c) = choices.get(idx) {
                    source.set_option_string(key, c);
                }
            }
            FormField::MultipleChoice(checks) => {
                let selected: Vec<String> = checks
                    .iter()
                    .filter(|(_, chk)| chk.is_active())
                    .map(|(s, _)| s.clone())
                    .collect();
                source.set_option_string_list(key, &selected);
            }
            FormField::MultipleText(tv) => {
                let buf = tv.buffer();
                let (start, end) = buf.bounds();
                let text = buf.text(&start, &end, false);
                let items: Vec<String> = text
                    .lines()
                    .flat_map(|line| line.split([',', '\t']))
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
                source.set_option_string_list(key, &items);
            }
        }
    }
}

pub struct SourcesView {
    pub container: Box,
    config: RefCell<AppConfig>,
    providers: RefCell<HashMap<String, DiscoveredProvider>>,
    selected_idx: Cell<Option<usize>>,
    source_list_box: ListBox,
    editor_container: Box,
    empty_placeholder: Label,
    custom_name_entry: Entry,
    provider_info_label: Label,
    dynamic_options_box: Box,
    current_fields: RefCell<Vec<(String, FormField)>>,
    preview_grid: TimelineGrid,
    preview_spinner: Spinner,
    preview_status: Label,
    dirty: Rc<Cell<bool>>,
    self_weak: RefCell<Weak<SourcesView>>,
    backend: Rc<BackendHandle>,
}

impl SourcesView {
    pub fn new(
        backend: Rc<BackendHandle>,
        initial_config: &AppConfig,
        dirty: Rc<Cell<bool>>,
    ) -> Rc<Self> {
        let container = Box::new(Orientation::Horizontal, 0);
        container.set_vexpand(true);
        container.set_hexpand(true);

        let aspect_ratio = match wallpp::monitor::get_biggest_monitor() {
            Some(m) if m.width > 0 && m.height > 0 => m.width as f64 / m.height as f64,
            _ => 16.0 / 9.0,
        };

        // Left pane: Source list
        let left_pane = Box::new(Orientation::Vertical, 8);
        left_pane.set_size_request(260, -1);
        left_pane.set_margin_start(16);
        left_pane.set_margin_end(12);
        left_pane.set_margin_top(16);
        left_pane.set_margin_bottom(12);

        let list_header = Box::new(Orientation::Horizontal, 8);
        let list_title = Label::new(Some("Sources"));
        list_title.add_css_class("card-title");
        list_title.set_hexpand(true);
        list_title.set_halign(Align::Start);
        list_header.append(&list_title);
        left_pane.append(&list_header);

        let list_scroll = ScrolledWindow::builder()
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .vscrollbar_policy(gtk4::PolicyType::Automatic)
            .vexpand(true)
            .build();

        let source_list_box = ListBox::new();
        source_list_box.add_css_class("navigation-sidebar");
        list_scroll.set_child(Some(&source_list_box));
        left_pane.append(&list_scroll);

        // List action buttons
        let list_actions = Box::new(Orientation::Horizontal, 6);
        let add_btn = Button::with_label("+ Add");
        let remove_btn = Button::with_label("Remove");
        let up_btn = Button::with_label("▲");
        let down_btn = Button::with_label("▼");

        list_actions.append(&add_btn);
        list_actions.append(&remove_btn);
        list_actions.append(&up_btn);
        list_actions.append(&down_btn);
        left_pane.append(&list_actions);

        container.append(&left_pane);

        let sep = gtk4::Separator::new(Orientation::Vertical);
        container.append(&sep);

        // Right pane: Source editor
        let right_pane = Box::new(Orientation::Vertical, 0);
        right_pane.set_hexpand(true);
        right_pane.set_vexpand(true);

        let editor_scroll = ScrolledWindow::builder()
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .vscrollbar_policy(gtk4::PolicyType::Automatic)
            .vexpand(true)
            .hexpand(true)
            .build();

        let editor_content = Box::new(Orientation::Vertical, 16);
        editor_content.set_margin_start(20);
        editor_content.set_margin_end(20);
        editor_content.set_margin_top(16);
        editor_content.set_margin_bottom(24);

        let empty_placeholder = Label::new(Some(
            "Select a source to edit or click '+ Add' to create one.",
        ));
        empty_placeholder.set_valign(Align::Center);
        empty_placeholder.set_vexpand(true);
        empty_placeholder.add_css_class("card-sub");
        editor_content.append(&empty_placeholder);

        let editor_container = Box::new(Orientation::Vertical, 16);
        editor_container.set_visible(false);

        // Header
        let editor_header = Box::new(Orientation::Vertical, 4);
        let editor_title = Label::new(Some("Source Configuration"));
        editor_title.add_css_class("card-title");
        editor_title.set_halign(Align::Start);

        let provider_info_label = Label::new(None);
        provider_info_label.add_css_class("card-sub");
        provider_info_label.set_halign(Align::Start);

        editor_header.append(&editor_title);
        editor_header.append(&provider_info_label);
        editor_container.append(&editor_header);

        // Custom name
        let custom_name_box = Box::new(Orientation::Vertical, 4);
        let custom_name_lbl = Label::new(Some("Custom Display Name (optional)"));
        custom_name_lbl.set_halign(Align::Start);
        let custom_name_entry = Entry::new();
        custom_name_entry.set_placeholder_text(Some("e.g. My Wallpaper Feed"));
        custom_name_box.append(&custom_name_lbl);
        custom_name_box.append(&custom_name_entry);
        editor_container.append(&custom_name_box);

        // Dynamic options box
        let dynamic_options_box = Box::new(Orientation::Vertical, 12);
        editor_container.append(&dynamic_options_box);

        // Preview section
        let preview_section = Box::new(Orientation::Vertical, 10);
        preview_section.set_margin_top(12);

        let preview_header = Box::new(Orientation::Horizontal, 10);
        let preview_title = Label::new(Some("Source Previews"));
        preview_title.add_css_class("card-title");
        preview_title.set_halign(Align::Start);

        let fetch_preview_btn = Button::with_label("Fetch Previews (5)");
        let preview_spinner = Spinner::new();
        let preview_status = Label::new(None);
        preview_status.add_css_class("card-sub");
        preview_status.set_halign(Align::Start);

        preview_header.append(&preview_title);
        preview_header.append(&fetch_preview_btn);
        preview_header.append(&preview_spinner);
        preview_header.append(&preview_status);
        preview_section.append(&preview_header);

        let preview_grid = TimelineGrid::new(aspect_ratio);
        preview_grid.set_valign(Align::Start);
        preview_grid.set_margin_top(8);
        preview_section.append(&preview_grid);

        editor_container.append(&preview_section);
        editor_content.append(&editor_container);

        editor_scroll.set_child(Some(&editor_content));
        right_pane.append(&editor_scroll);
        container.append(&right_pane);

        let view = Rc::new(Self {
            container,
            config: RefCell::new(initial_config.clone()),
            providers: RefCell::new(HashMap::new()),
            selected_idx: Cell::new(None),
            source_list_box,
            editor_container,
            empty_placeholder,
            custom_name_entry,
            provider_info_label,
            dynamic_options_box,
            current_fields: RefCell::new(Vec::new()),
            preview_grid,
            preview_spinner,
            preview_status,
            dirty,
            self_weak: RefCell::new(Weak::new()),
            backend,
        });

        *view.self_weak.borrow_mut() = Rc::downgrade(&view);

        // Selection change handler
        let weak = Rc::downgrade(&view);
        view.source_list_box.connect_row_selected(move |_, row| {
            if let Some(v) = weak.upgrade() {
                if let Some(r) = row {
                    v.select_source(r.index() as usize);
                } else {
                    v.clear_selection();
                }
            }
        });

        // Add button popover
        let weak = Rc::downgrade(&view);
        let add_popover = Popover::new();
        let popover_box = Box::new(Orientation::Vertical, 4);
        popover_box.set_margin_start(8);
        popover_box.set_margin_end(8);
        popover_box.set_margin_top(8);
        popover_box.set_margin_bottom(8);
        add_popover.set_child(Some(&popover_box));
        add_popover.set_parent(&add_btn);

        let popover_weak = add_popover.clone();
        add_btn.connect_clicked(move |_| {
            if let Some(v) = weak.upgrade() {
                v.populate_add_popover(&popover_box, &popover_weak);
                popover_weak.popup();
            }
        });

        // Remove button
        let weak = Rc::downgrade(&view);
        remove_btn.connect_clicked(move |_| {
            if let Some(v) = weak.upgrade() {
                v.remove_selected();
            }
        });

        // Move Up button
        let weak = Rc::downgrade(&view);
        up_btn.connect_clicked(move |_| {
            if let Some(v) = weak.upgrade() {
                v.move_selected(-1);
            }
        });

        // Move Down button
        let weak = Rc::downgrade(&view);
        down_btn.connect_clicked(move |_| {
            if let Some(v) = weak.upgrade() {
                v.move_selected(1);
            }
        });

        // Custom name auto-save
        let weak = Rc::downgrade(&view);
        view.custom_name_entry.connect_activate(move |_| {
            if let Some(v) = weak.upgrade() {
                v.save_current_source();
            }
        });
        let fc = EventControllerFocus::new();
        let weak = Rc::downgrade(&view);
        fc.connect_leave(move |_| {
            if let Some(v) = weak.upgrade() {
                v.save_current_source();
            }
        });
        view.custom_name_entry.add_controller(fc);

        // Fetch previews button
        let weak = Rc::downgrade(&view);
        fetch_preview_btn.connect_clicked(move |_| {
            if let Some(v) = weak.upgrade() {
                v.fetch_previews_for_current();
            }
        });

        view.refresh_list();
        view
    }

    pub fn set_providers(&self, provs: Vec<DiscoveredProvider>) {
        let mut map = HashMap::new();
        for p in provs {
            map.insert(p.name.clone(), p);
        }
        *self.providers.borrow_mut() = map;
        self.refresh_list();
        if let Some(idx) = self.selected_idx.get() {
            self.select_source(idx);
        }
    }

    #[allow(dead_code)]
    pub fn update_config(&self, new_cfg: &AppConfig) {
        *self.config.borrow_mut() = new_cfg.clone();
        self.refresh_list();
    }

    fn refresh_list(&self) {
        while let Some(row) = self.source_list_box.first_child() {
            self.source_list_box.remove(&row);
        }

        let cfg = self.config.borrow();
        let provs = self.providers.borrow();

        for (i, src) in cfg.source.iter().enumerate() {
            let row = ListBoxRow::new();
            let row_box = Box::new(Orientation::Vertical, 2);
            row_box.set_margin_start(8);
            row_box.set_margin_end(8);
            row_box.set_margin_top(6);
            row_box.set_margin_bottom(6);

            let prov_label = provs
                .get(&src.provider)
                .map(|p| p.info.label.clone())
                .unwrap_or_else(|| src.provider.clone());

            let display_name = src.name.as_deref().unwrap_or(&prov_label);
            let name_lbl = Label::new(Some(display_name));
            name_lbl.set_halign(Align::Start);
            name_lbl.add_css_class("card-title");

            let sub_lbl = Label::new(Some(&format!("Provider: {}", prov_label)));
            sub_lbl.set_halign(Align::Start);
            sub_lbl.add_css_class("card-sub");

            row_box.append(&name_lbl);
            row_box.append(&sub_lbl);
            row.set_child(Some(&row_box));

            self.source_list_box.append(&row);

            if Some(i) == self.selected_idx.get() {
                self.source_list_box.select_row(Some(&row));
            }
        }
    }

    fn clear_selection(&self) {
        self.selected_idx.set(None);
        self.empty_placeholder.set_visible(true);
        self.editor_container.set_visible(false);
    }

    fn select_source(&self, index: usize) {
        let cfg = self.config.borrow();
        let Some(src) = cfg.source.get(index) else {
            self.clear_selection();
            return;
        };

        self.selected_idx.set(Some(index));
        self.empty_placeholder.set_visible(false);
        self.editor_container.set_visible(true);
        self.preview_grid.clear();
        self.preview_status.set_text("");

        self.custom_name_entry
            .set_text(src.name.as_deref().unwrap_or(""));

        let provs = self.providers.borrow();
        let prov = provs.get(&src.provider);

        let prov_label = prov
            .map(|p| format!("{} (version {})", p.info.label, p.info.version))
            .unwrap_or_else(|| format!("{} [uninstalled or loading]", src.provider));
        self.provider_info_label
            .set_text(&format!("Provider: {}", prov_label));

        // Rebuild dynamic options
        while let Some(child) = self.dynamic_options_box.first_child() {
            self.dynamic_options_box.remove(&child);
        }
        self.current_fields.borrow_mut().clear();

        if let Some(p) = prov {
            let mut fields = Vec::new();
            for spec in &p.info.options {
                let (widget, field) = build_option_field(spec, src, &self.self_weak);
                self.dynamic_options_box.append(&widget);
                fields.push((spec.key.clone(), field));
            }
            *self.current_fields.borrow_mut() = fields;
        }
    }

    fn populate_add_popover(&self, box_widget: &Box, popover: &Popover) {
        while let Some(child) = box_widget.first_child() {
            box_widget.remove(&child);
        }

        let provs = self.providers.borrow();
        if provs.is_empty() {
            let lbl = Label::new(Some("No providers found"));
            box_widget.append(&lbl);
            return;
        }

        let mut sorted: Vec<_> = provs.values().collect();
        sorted.sort_by_key(|p| &p.info.label);

        for prov in sorted {
            let btn = Button::with_label(&format!("{} [{}]", prov.info.label, prov.name));
            btn.set_halign(Align::Fill);
            let prov_name = prov.name.clone();
            let pop = popover.clone();
            let weak = self.self_weak.borrow().clone();
            btn.connect_clicked(move |_| {
                pop.popdown();
                if let Some(v) = weak.upgrade() {
                    v.add_source_for_provider(&prov_name);
                }
            });
            box_widget.append(&btn);
        }
    }

    fn add_source_for_provider(&self, provider_name: &str) {
        let mut cfg = self.config.borrow().clone();
        let provs = self.providers.borrow();

        let new_source = if let Some(prov) = provs.get(provider_name) {
            SourceConfig::from_default_config(provider_name.to_string(), &prov.info.default_config)
        } else {
            SourceConfig::new(provider_name)
        };

        cfg.source.push(new_source);

        let new_idx = cfg.source.len() - 1;
        self.dirty.set(true);
        *self.config.borrow_mut() = cfg.clone();
        self.backend.send(Action::UpdateConfig(cfg));

        self.refresh_list();
        self.select_source(new_idx);
    }

    fn remove_selected(&self) {
        let Some(idx) = self.selected_idx.get() else {
            return;
        };
        let mut cfg = self.config.borrow().clone();
        if idx < cfg.source.len() {
            cfg.source.remove(idx);
            self.dirty.set(true);
            *self.config.borrow_mut() = cfg.clone();
            self.backend.send(Action::UpdateConfig(cfg));

            self.clear_selection();
            self.refresh_list();
            if !self.config.borrow().source.is_empty() {
                let next_idx = idx.min(self.config.borrow().source.len() - 1);
                self.select_source(next_idx);
            }
        }
    }

    fn move_selected(&self, delta: isize) {
        let Some(idx) = self.selected_idx.get() else {
            return;
        };
        let mut cfg = self.config.borrow().clone();
        let target = idx as isize + delta;
        if target >= 0 && (target as usize) < cfg.source.len() {
            let target_idx = target as usize;
            cfg.source.swap(idx, target_idx);
            self.dirty.set(true);
            *self.config.borrow_mut() = cfg.clone();
            self.backend.send(Action::UpdateConfig(cfg));

            self.refresh_list();
            self.select_source(target_idx);
        }
    }

    pub fn save_current_source(&self) {
        let Some(idx) = self.selected_idx.get() else {
            return;
        };
        let mut cfg = self.config.borrow().clone();
        let Some(src) = cfg.source.get_mut(idx) else {
            return;
        };

        let name_text = self.custom_name_entry.text().trim().to_string();
        src.name = if name_text.is_empty() {
            None
        } else {
            Some(name_text)
        };

        for (key, field) in self.current_fields.borrow().iter() {
            field.apply_to_source(key, src);
        }

        self.dirty.set(true);
        *self.config.borrow_mut() = cfg.clone();
        self.backend.send(Action::UpdateConfig(cfg));

        self.refresh_list();
    }

    fn fetch_previews_for_current(&self) {
        let Some(idx) = self.selected_idx.get() else {
            return;
        };
        let cfg = self.config.borrow();
        let Some(src) = cfg.source.get(idx) else {
            return;
        };

        let mut preview_src = src.clone();
        for (key, field) in self.current_fields.borrow().iter() {
            field.apply_to_source(key, &mut preview_src);
        }

        self.preview_spinner.start();
        self.preview_status.set_text("Loading previews...");
        self.preview_grid.clear();

        self.backend.send(Action::FetchSourcePreviews {
            source: preview_src,
            count: 5,
        });
    }

    pub fn handle_previews(&self, items: &[SourcePreviewItem]) {
        self.preview_spinner.stop();
        self.preview_status
            .set_text(&format!("Showing {} wallpapers", items.len()));
        self.preview_grid.clear();

        for item in items {
            let overlay = Overlay::new();
            overlay.add_css_class("timeline-card");
            overlay.add_css_class("history-card");
            overlay.set_overflow(gtk4::Overflow::Hidden);

            if item.cache_path.exists() {
                let pic = Picture::for_filename(&item.cache_path);
                pic.set_can_shrink(true);
                pic.set_content_fit(gtk4::ContentFit::Cover);
                overlay.set_child(Some(&pic));
            } else {
                overlay.set_child(Some(&Label::new(Some("No preview"))));
            }

            let title_text = item.title.as_deref().unwrap_or("Untitled");
            let tooltip = match &item.author {
                Some(a) => format!("{}\nby {}", title_text, a),
                None => title_text.to_string(),
            };
            overlay.set_tooltip_text(Some(&tooltip));

            self.preview_grid.append(&overlay);
        }
    }

    pub fn handle_preview_error(&self, err: &str) {
        self.preview_spinner.stop();
        self.preview_status
            .set_text(&format!("Preview error: {}", err));
    }

    pub fn trigger_validation(&self) {
        self.save_current_source();
        let Some(idx) = self.selected_idx.get() else {
            return;
        };
        let cfg = self.config.borrow();
        let Some(src) = cfg.source.get(idx) else {
            return;
        };
        self.backend.send(Action::ValidateSourceConfig {
            source: src.clone(),
        });
    }

    pub fn handle_normalized_config(&self, cfg: &[wallpp::wallpp::provider::types::ConfigEntry]) {
        let fields = self.current_fields.borrow();
        for (key, field) in fields.iter() {
            if let Some(entry) = cfg.iter().find(|e| e.key == *key) {
                match (field, &entry.value) {
                    (FormField::MultipleText(tv), ConfigValue::Many(items)) => {
                        let lines: Vec<String> = items
                            .iter()
                            .filter_map(|it| match it {
                                ScalarValue::Text(s) | ScalarValue::Choice(s) => Some(s.clone()),
                                _ => None,
                            })
                            .collect();
                        tv.buffer().set_text(&lines.join("\n"));
                    }
                    (FormField::Text(entry), ConfigValue::One(ScalarValue::Text(s))) => {
                        entry.set_text(s);
                    }
                    _ => {}
                }
            }
        }
    }
}

fn build_option_field(
    spec: &OptionSpec,
    source: &SourceConfig,
    weak_view: &RefCell<Weak<SourcesView>>,
) -> (Box, FormField) {
    let row = Box::new(Orientation::Vertical, 4);

    let label_box = Box::new(Orientation::Horizontal, 8);
    let title_lbl = Label::new(Some(&spec.label));
    title_lbl.add_css_class("card-title");
    title_lbl.set_halign(Align::Start);
    label_box.append(&title_lbl);

    if spec.required {
        let req = Label::new(Some("*"));
        req.set_halign(Align::Start);
        label_box.append(&req);
    }
    row.append(&label_box);

    if let Some(ref desc) = spec.description {
        let desc_lbl = Label::new(Some(desc));
        desc_lbl.add_css_class("card-sub");
        desc_lbl.set_halign(Align::Start);
        desc_lbl.set_wrap(true);
        row.append(&desc_lbl);
    }

    if spec.multiple {
        let mut existing_list = source.get_option_string_list(&spec.key);
        if existing_list.is_empty() {
            if let Some(ConfigValue::Many(items)) = &spec.default {
                existing_list = items
                    .iter()
                    .filter_map(|it| match it {
                        ScalarValue::Choice(s) | ScalarValue::Text(s) => Some(s.clone()),
                        _ => None,
                    })
                    .collect();
            }
        }

        match &spec.ty {
            ScalarType::Choice(choices) => {
                let check_box = Box::new(Orientation::Vertical, 4);
                let mut checks = Vec::new();
                for choice in choices {
                    let chk = CheckButton::with_label(choice);
                    chk.set_active(existing_list.iter().any(|e| e == choice));
                    let weak_view_chk = weak_view.borrow().clone();
                    chk.connect_toggled(move |_| {
                        if let Some(v) = weak_view_chk.upgrade() {
                            v.save_current_source();
                        }
                    });
                    check_box.append(&chk);
                    checks.push((choice.clone(), chk));
                }
                row.append(&check_box);
                (row, FormField::MultipleChoice(checks))
            }
            _ => {
                let text_view = TextView::new();
                text_view.set_accepts_tab(false);
                text_view.set_wrap_mode(WrapMode::Word);
                text_view.set_monospace(true);
                text_view.add_css_class("multiline-entry");

                let buffer = text_view.buffer();
                buffer.set_text(&existing_list.join("\n"));

                let focus_ctrl = EventControllerFocus::new();
                let weak_view_clone = weak_view.borrow().clone();
                focus_ctrl.connect_leave(move |_| {
                    if let Some(v) = weak_view_clone.upgrade() {
                        v.trigger_validation();
                    }
                });
                text_view.add_controller(focus_ctrl);

                let key_ctrl = EventControllerKey::new();
                let weak_view_key = weak_view.borrow().clone();
                key_ctrl.connect_key_pressed(move |_ctrl, keyval, _code, state| {
                    if state.contains(gdk::ModifierType::CONTROL_MASK) && keyval == gdk::Key::Return
                    {
                        if let Some(v) = weak_view_key.upgrade() {
                            v.trigger_validation();
                        }
                        glib::Propagation::Stop
                    } else {
                        glib::Propagation::Proceed
                    }
                });
                text_view.add_controller(key_ctrl);

                let scroll = ScrolledWindow::new();
                scroll.set_child(Some(&text_view));
                scroll.set_min_content_height(100);
                scroll.set_has_frame(true);
                scroll.add_css_class("multiline-scroll");

                row.append(&scroll);
                (row, FormField::MultipleText(text_view))
            }
        }
    } else {
        match &spec.ty {
            ScalarType::Boolean => {
                let chk = CheckButton::new();
                chk.set_halign(Align::Start);
                let initial = source
                    .get_option_bool(&spec.key)
                    .unwrap_or(match &spec.default {
                        Some(ConfigValue::One(ScalarValue::Boolean(b))) => *b,
                        _ => false,
                    });
                chk.set_active(initial);
                let weak_view_chk = weak_view.borrow().clone();
                chk.connect_toggled(move |_| {
                    if let Some(v) = weak_view_chk.upgrade() {
                        v.save_current_source();
                    }
                });
                row.append(&chk);
                (row, FormField::Boolean(chk))
            }
            ScalarType::Integer(IntBounds { min, max }) => {
                let min_val = min.unwrap_or(0) as f64;
                let max_val = max.unwrap_or(10_000) as f64;
                let spin = SpinButton::with_range(min_val, max_val, 1.0);
                spin.set_halign(Align::Start);
                let initial = source
                    .get_option_int(&spec.key)
                    .map(|i| i as f64)
                    .unwrap_or_else(|| match &spec.default {
                        Some(ConfigValue::One(ScalarValue::Integer(i))) => *i as f64,
                        _ => min_val,
                    });
                spin.set_value(initial);
                let weak_view_spin = weak_view.borrow().clone();
                spin.connect_value_changed(move |_| {
                    if let Some(v) = weak_view_spin.upgrade() {
                        v.save_current_source();
                    }
                });
                row.append(&spin);
                (row, FormField::Integer(spin))
            }
            ScalarType::Choice(choices) => {
                let choice_strs: Vec<&str> = choices.iter().map(|s| s.as_str()).collect();
                let dropdown = DropDown::from_strings(&choice_strs);
                dropdown.set_halign(Align::Start);

                let selected_str =
                    source
                        .get_option_string(&spec.key)
                        .unwrap_or_else(|| match &spec.default {
                            Some(ConfigValue::One(ScalarValue::Choice(s))) => s.clone(),
                            _ => choices.first().cloned().unwrap_or_default(),
                        });
                if let Some(pos) = choices.iter().position(|c| c == &selected_str) {
                    dropdown.set_selected(pos as u32);
                }
                let weak_view_drop = weak_view.borrow().clone();
                dropdown.connect_selected_notify(move |_| {
                    if let Some(v) = weak_view_drop.upgrade() {
                        v.save_current_source();
                    }
                });
                row.append(&dropdown);
                (
                    row,
                    FormField::Choice {
                        dropdown,
                        choices: choices.clone(),
                    },
                )
            }
            ScalarType::Secret => {
                let entry = PasswordEntry::new();
                entry.set_show_peek_icon(true);
                if let Some(s) = source.get_option_string(&spec.key) {
                    entry.set_text(&s);
                }
                let weak_view_sec = weak_view.borrow().clone();
                let fc = EventControllerFocus::new();
                fc.connect_leave(move |_| {
                    if let Some(v) = weak_view_sec.upgrade() {
                        v.save_current_source();
                    }
                });
                entry.add_controller(fc);
                row.append(&entry);
                (row, FormField::Secret(entry))
            }
            ScalarType::Text => {
                let entry = Entry::new();
                let initial =
                    source
                        .get_option_string(&spec.key)
                        .unwrap_or_else(|| match &spec.default {
                            Some(ConfigValue::One(ScalarValue::Text(s))) => s.clone(),
                            _ => String::new(),
                        });
                entry.set_text(&initial);

                let weak_view_act = weak_view.borrow().clone();
                entry.connect_activate(move |_| {
                    if let Some(v) = weak_view_act.upgrade() {
                        v.trigger_validation();
                    }
                });
                let focus_ctrl = EventControllerFocus::new();
                let weak_view_leave = weak_view.borrow().clone();
                focus_ctrl.connect_leave(move |_| {
                    if let Some(v) = weak_view_leave.upgrade() {
                        v.trigger_validation();
                    }
                });
                entry.add_controller(focus_ctrl);

                row.append(&entry);
                (row, FormField::Text(entry))
            }
        }
    }
}
