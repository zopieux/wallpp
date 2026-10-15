//! Sources page: configured-source list (add/remove/reorder) and a
//! per-provider typed option editor with auto-save, validation, and
//! cover-thumbnail previews.

use gtk4::prelude::*;
use gtk4::{
    gdk, glib, Align, Box, Button, CheckButton, DropDown, Entry, EventControllerFocus,
    EventControllerKey, Label, ListBox, ListBoxRow, Orientation, PasswordEntry, Popover,
    ScrolledWindow, SpinButton, Spinner, TextView, WrapMode,
};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use wallpp::config::{AppConfig, SourceConfig};
use wallpp::provider::DiscoveredProvider;
use wallpp::wallpp::provider::types::{
    ConfigEntry, ConfigValue, IntBounds, OptionSpec, ScalarType, ScalarValue,
};

use super::timeline_grid::TimelineGrid;
use super::widgets::{cover_card, sub_label, title_label};
use crate::backend::{Action, BackendHandle, PreviewItem};
use crate::{clear, stack};

/// One typed editor widget bound to a provider option key.
enum Field {
    Text(Entry),
    Secret(PasswordEntry),
    Integer(SpinButton),
    Boolean(CheckButton),
    Choice {
        drop: DropDown,
        choices: Vec<String>,
    },
    MultiChoice(Vec<(String, CheckButton)>),
    MultiText(TextView),
}

impl Field {
    fn write(&self, key: &str, source: &mut SourceConfig) {
        match self {
            Field::Text(e) => source.set_option_string(key, e.text().as_str()),
            Field::Secret(e) => source.set_option_string(key, e.text().as_str()),
            Field::Integer(s) => source.set_option_int(key, s.value() as i64),
            Field::Boolean(c) => source.set_option_bool(key, c.is_active()),
            Field::Choice { drop, choices } => {
                if let Some(c) = choices.get(drop.selected() as usize) {
                    source.set_option_string(key, c);
                }
            }
            Field::MultiChoice(checks) => {
                let selected: Vec<String> = checks
                    .iter()
                    .filter(|(_, c)| c.is_active())
                    .map(|(s, _)| s.clone())
                    .collect();
                source.set_option_string_list(key, &selected);
            }
            Field::MultiText(tv) => {
                let buf = tv.buffer();
                let (start, end) = buf.bounds();
                let items: Vec<String> = buf
                    .text(&start, &end, false)
                    .lines()
                    .flat_map(|l| l.split([',', '\t']))
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
                source.set_option_string_list(key, &items);
            }
        }
    }
}

macro_rules! on_save {
    ($weak:expr) => {{
        let weak = $weak.clone();
        move |_| {
            if let Some(v) = weak.upgrade() {
                v.save_current();
            }
        }
    }};
}

macro_rules! on_validate {
    ($weak:expr) => {{
        let weak = $weak.clone();
        move |_| {
            if let Some(v) = weak.upgrade() {
                v.validate_current();
            }
        }
    }};
}

/// Run save (or validate) when keyboard focus leaves a field widget.
/// The work is deferred to an idle callback: a focus-leave fires in the
/// middle of the *next* widget's button-press dispatch, and saving
/// synchronously there can rebuild the editor (via the source list)
/// while that press is still being handled — destroying the widget
/// being clicked and corrupting the event sequence.
fn connect_leave(widget: &impl IsA<gtk4::Widget>, weak: &Weak<SourcesView>, validate: bool) {
    let focus = EventControllerFocus::new();
    let weak = weak.clone();
    focus.connect_leave(move |_| {
        let weak = weak.clone();
        glib::idle_add_local_once(move || {
            if let Some(view) = weak.upgrade() {
                if validate {
                    view.validate_current();
                } else {
                    view.save_current();
                }
            }
        });
    });
    widget.add_controller(focus);
}

/// Current string value for a field, falling back to the spec default.
fn initial_string(source: &SourceConfig, spec: &OptionSpec) -> String {
    source
        .get_option_string(&spec.key)
        .unwrap_or_else(|| default_strings(spec).into_iter().next().unwrap_or_default())
}

pub struct SourcesView {
    pub container: Box,
    cfg: Rc<RefCell<AppConfig>>,
    providers: RefCell<HashMap<String, DiscoveredProvider>>,
    selected: Cell<Option<usize>>,
    list: ListBox,
    editor: Box,
    empty: Label,
    name_entry: Entry,
    info_label: Label,
    options_box: Box,
    fields: RefCell<Vec<(String, Field)>>,
    preview_grid: TimelineGrid,
    aspect_ratio: Cell<f32>,
    preview_spinner: Spinner,
    preview_status: Label,
    dirty: Rc<Cell<bool>>,
    /// True while `refresh_list` rebuilds rows; its programmatic
    /// re-selection must not re-enter `select()` (which rebuilds the
    /// editor fields).
    rebuilding_list: Cell<bool>,
    weak_self: RefCell<Weak<SourcesView>>,
    backend: BackendHandle,
}

fn scrolled(child: &impl IsA<gtk4::Widget>) -> ScrolledWindow {
    ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .vscrollbar_policy(gtk4::PolicyType::Automatic)
        .vexpand(true)
        .hexpand(true)
        .child(child)
        .build()
}

impl SourcesView {
    pub fn new(
        backend: BackendHandle,
        cfg: Rc<RefCell<AppConfig>>,
        dirty: Rc<Cell<bool>>,
    ) -> Rc<Self> {
        // Left: source list + list actions.
        let list = ListBox::new();
        list.add_css_class("navigation-sidebar");
        let add_btn = Button::with_label("+ Add");
        let remove_btn = Button::with_label("Remove");
        let up_btn = Button::with_label("▲");
        let down_btn = Button::with_label("▼");
        let list_actions = stack!(
            Orientation::Horizontal,
            6,
            add_btn,
            remove_btn,
            up_btn,
            down_btn
        );
        let left = stack!(
            Orientation::Vertical,
            8,
            title_label("Sources"),
            scrolled(&list),
            list_actions
        );
        left.set_size_request(260, -1);
        left.set_margin_start(16);
        left.set_margin_end(12);
        left.set_margin_top(16);
        left.set_margin_bottom(12);

        // Right: editor for the selected source.
        let empty = Label::new(Some(
            "Select a source to edit or click '+ Add' to create one.",
        ));
        empty.add_css_class("card-sub");
        empty.set_vexpand(true);
        empty.set_valign(Align::Center);

        let name_entry = Entry::new();
        name_entry.set_placeholder_text(Some("e.g. My Wallpaper Feed"));
        let name_box = stack!(
            Orientation::Vertical,
            4,
            Label::new(Some("Custom Display Name (optional)")),
            name_entry
        );

        let info_label = sub_label("");
        let options_box = Box::new(Orientation::Vertical, 12);

        let preview_grid = TimelineGrid::new(16.0 / 9.0);
        let preview_spinner = Spinner::new();
        let preview_status = sub_label("");
        let fetch_btn = Button::with_label("Fetch Previews (5)");
        let preview_header = stack!(
            Orientation::Horizontal,
            10,
            title_label("Source Previews"),
            fetch_btn,
            preview_spinner,
            preview_status
        );
        let preview_section = stack!(Orientation::Vertical, 10, preview_header, preview_grid);
        preview_section.set_margin_top(12);

        let editor_header = stack!(
            Orientation::Vertical,
            4,
            title_label("Source Configuration"),
            info_label
        );
        let editor = stack!(
            Orientation::Vertical,
            16,
            editor_header,
            name_box,
            options_box,
            preview_section
        );
        editor.set_visible(false);

        let editor_content = stack!(Orientation::Vertical, 16, empty, editor);
        editor_content.set_margin_start(20);
        editor_content.set_margin_end(20);
        editor_content.set_margin_top(16);
        editor_content.set_margin_bottom(24);

        let right = stack!(Orientation::Vertical, 0, scrolled(&editor_content));
        right.set_hexpand(true);

        let container = stack!(
            Orientation::Horizontal,
            0,
            left,
            gtk4::Separator::new(Orientation::Vertical),
            right
        );

        let view = Rc::new(Self {
            container,
            cfg,
            providers: RefCell::new(HashMap::new()),
            selected: Cell::new(None),
            list,
            editor,
            empty,
            name_entry,
            info_label,
            options_box,
            fields: RefCell::new(Vec::new()),
            preview_grid,
            aspect_ratio: Cell::new(16.0 / 9.0),
            preview_spinner,
            preview_status,
            dirty,
            rebuilding_list: Cell::new(false),
            weak_self: RefCell::new(Weak::new()),
            backend,
        });
        *view.weak_self.borrow_mut() = Rc::downgrade(&view);
        let weak = Rc::downgrade(&view);

        view.list.connect_row_selected({
            let weak = weak.clone();
            move |_, row| {
                if let Some(v) = weak.upgrade() {
                    // Programmatic re-selection during a list rebuild is
                    // not a user selection change; the editor is already
                    // showing this source and must not be rebuilt (in
                    // particular not from a save triggered by focus
                    // leaving an editor field).
                    if v.rebuilding_list.get() {
                        return;
                    }
                    match row {
                        Some(r) => v.select(r.index() as usize),
                        None => v.clear_selection(),
                    }
                }
            }
        });
        remove_btn.connect_clicked({
            let weak = weak.clone();
            move |_| {
                if let Some(v) = weak.upgrade() {
                    v.remove_selected();
                }
            }
        });
        up_btn.connect_clicked({
            let weak = weak.clone();
            move |_| {
                if let Some(v) = weak.upgrade() {
                    v.move_selected(-1);
                }
            }
        });
        down_btn.connect_clicked({
            let weak = weak.clone();
            move |_| {
                if let Some(v) = weak.upgrade() {
                    v.move_selected(1);
                }
            }
        });
        fetch_btn.connect_clicked({
            let weak = weak.clone();
            move |_| {
                if let Some(v) = weak.upgrade() {
                    v.fetch_previews();
                }
            }
        });

        // Add-source popover, populated from discovered providers.
        let popover = Popover::new();
        let popover_box = Box::new(Orientation::Vertical, 4);
        popover_box.set_margin_start(8);
        popover_box.set_margin_end(8);
        popover_box.set_margin_top(8);
        popover_box.set_margin_bottom(8);
        popover.set_child(Some(&popover_box));
        popover.set_parent(&add_btn);
        add_btn.connect_clicked({
            let weak = weak.clone();
            let popover = popover.clone();
            move |_| {
                if let Some(v) = weak.upgrade() {
                    v.populate_add_popover(&popover_box, &popover);
                    popover.popup();
                }
            }
        });

        view.name_entry.connect_activate(on_save!(weak));
        connect_leave(&view.name_entry, &weak, false);

        view.refresh_list();
        view
    }

    pub fn set_providers(&self, provs: Vec<DiscoveredProvider>) {
        *self.providers.borrow_mut() = provs.into_iter().map(|p| (p.name.clone(), p)).collect();
        self.refresh_list();
        if let Some(idx) = self.selected.get() {
            self.select(idx);
        }
    }

    fn provider_label(&self, name: &str) -> String {
        self.providers
            .borrow()
            .get(name)
            .map(|p| p.info.label.clone())
            .unwrap_or_else(|| name.to_string())
    }

    fn refresh_list(&self) {
        self.rebuilding_list.set(true);
        clear!(self.list);
        for (i, src) in self.cfg.borrow().source.iter().enumerate() {
            let label = self.provider_label(&src.provider);
            let text = Box::new(Orientation::Vertical, 2);
            text.set_margin_start(8);
            text.set_margin_end(8);
            text.set_margin_top(6);
            text.set_margin_bottom(6);
            text.append(&title_label(src.name.as_deref().unwrap_or(&label)));
            text.append(&sub_label(&format!("Provider: {label}")));
            let row = ListBoxRow::new();
            row.set_child(Some(&text));
            self.list.append(&row);
            if Some(i) == self.selected.get() {
                self.list.select_row(Some(&row));
            }
        }
        self.rebuilding_list.set(false);
    }

    fn clear_selection(&self) {
        self.selected.set(None);
        self.empty.set_visible(true);
        self.editor.set_visible(false);
    }

    fn select(&self, index: usize) {
        let cfg = self.cfg.borrow();
        let Some(src) = cfg.source.get(index) else {
            drop(cfg);
            self.clear_selection();
            return;
        };
        let provider = src.provider.clone();
        let name = src.name.clone();
        // Clone specs so the config borrow ends before touching widgets.
        let specs: Vec<OptionSpec> = self
            .providers
            .borrow()
            .get(&provider)
            .map(|p| p.info.options.clone())
            .unwrap_or_default();
        let info = match self.providers.borrow().get(&provider) {
            Some(p) => format!("{} (version {})", p.info.label, p.info.version),
            None => format!("{provider} [uninstalled or loading]"),
        };
        let src = src.clone();
        drop(cfg);

        self.selected.set(Some(index));
        self.empty.set_visible(false);
        self.editor.set_visible(true);
        self.preview_grid.clear();
        self.preview_status.set_text("");
        self.name_entry.set_text(name.as_deref().unwrap_or(""));
        self.info_label.set_text(&format!("Provider: {info}"));

        clear!(self.options_box);
        let mut fields = Vec::new();
        for spec in &specs {
            let (widget, field) = build_field(spec, &src, &self.weak_self);
            self.options_box.append(&widget);
            fields.push((spec.key.clone(), field));
        }
        *self.fields.borrow_mut() = fields;
    }

    fn populate_add_popover(&self, bx: &Box, popover: &Popover) {
        clear!(bx);
        let providers = self.providers.borrow();
        if providers.is_empty() {
            bx.append(&Label::new(Some("No providers found")));
            return;
        }
        let mut sorted: Vec<&DiscoveredProvider> = providers.values().collect();
        sorted.sort_by_key(|p| &p.info.label);
        for prov in sorted {
            let btn = Button::with_label(&prov.info.label);
            btn.set_halign(Align::Fill);
            let name = prov.name.clone();
            let pop = popover.clone();
            let weak = self.weak_self.borrow().clone();
            btn.connect_clicked(move |_| {
                pop.popdown();
                if let Some(v) = weak.upgrade() {
                    v.add_source(&name);
                }
            });
            bx.append(&btn);
        }
    }

    fn persist(&self, cfg: AppConfig) {
        *self.cfg.borrow_mut() = cfg.clone();
        self.dirty.set(true);
        self.backend.send(Action::UpdateConfig(cfg));
    }

    fn add_source(&self, provider_name: &str) {
        let mut cfg = self.cfg.borrow().clone();
        let new_source = match self.providers.borrow().get(provider_name) {
            Some(prov) => SourceConfig::from_default_config(
                provider_name.to_string(),
                &prov.info.default_config,
            ),
            None => SourceConfig::new(provider_name),
        };
        cfg.source.push(new_source);
        let idx = cfg.source.len() - 1;
        self.persist(cfg);
        self.refresh_list();
        self.select(idx);
    }

    fn remove_selected(&self) {
        let Some(idx) = self.selected.get() else {
            return;
        };
        let mut cfg = self.cfg.borrow().clone();
        if idx >= cfg.source.len() {
            return;
        }
        cfg.source.remove(idx);
        let empty = cfg.source.is_empty();
        self.persist(cfg);
        self.clear_selection();
        self.refresh_list();
        if !empty {
            self.select(idx.min(self.cfg.borrow().source.len() - 1));
        }
    }

    fn move_selected(&self, delta: isize) {
        let Some(idx) = self.selected.get() else {
            return;
        };
        let mut cfg = self.cfg.borrow().clone();
        let target = idx as isize + delta;
        if target < 0 || target as usize >= cfg.source.len() {
            return;
        }
        cfg.source.swap(idx, target as usize);
        self.persist(cfg);
        self.refresh_list();
        self.select(target as usize);
    }

    /// Write widget values back into the shared config and persist them.
    pub fn save_current(&self) {
        let Some(idx) = self.selected.get() else {
            return;
        };
        let mut cfg = self.cfg.borrow().clone();
        let Some(src) = cfg.source.get_mut(idx) else {
            return;
        };
        let name = self.name_entry.text().trim().to_string();
        src.name = if name.is_empty() { None } else { Some(name) };
        for (key, field) in self.fields.borrow().iter() {
            field.write(key, src);
        }
        self.persist(cfg);
        self.refresh_list();
    }

    fn validate_current(&self) {
        self.save_current();
        if let Some(src) = self
            .selected
            .get()
            .and_then(|i| self.cfg.borrow().source.get(i).cloned())
        {
            self.backend.send(Action::Validate(src));
        }
    }

    fn fetch_previews(&self) {
        let Some(idx) = self.selected.get() else {
            return;
        };
        let Some(mut src) = self.cfg.borrow().source.get(idx).cloned() else {
            return;
        };
        for (key, field) in self.fields.borrow().iter() {
            field.write(key, &mut src);
        }
        self.preview_spinner.start();
        self.preview_status.set_text("Loading previews...");
        self.preview_grid.clear();
        self.backend.send(Action::Previews(src, 5));
    }

    /// Switch preview thumbnails to the real monitor aspect once probed.
    pub fn set_aspect(&self, ratio: f32) {
        self.aspect_ratio.set(ratio);
        self.preview_grid.set_aspect(ratio as f64);
    }

    pub fn handle_previews(&self, result: Result<Vec<PreviewItem>, String>) {
        self.preview_spinner.stop();
        match result {
            Ok(items) => {
                self.preview_status
                    .set_text(&format!("Showing {} wallpapers", items.len()));
                self.preview_grid.clear();
                for item in &items {
                    let tooltip = match &item.author {
                        Some(a) => {
                            format!("{}\nby {}", item.title.as_deref().unwrap_or("Untitled"), a)
                        }
                        None => item.title.as_deref().unwrap_or("Untitled").to_string(),
                    };
                    self.preview_grid.append(&cover_card(
                        &item.thumbnail_path,
                        "history-card",
                        None,
                        None,
                        &tooltip,
                    ));
                }
            }
            Err(e) => self.preview_status.set_text(&format!("Preview error: {e}")),
        }
    }

    pub fn handle_normalized(&self, entries: &[ConfigEntry]) {
        for (key, field) in self.fields.borrow().iter() {
            let Some(entry) = entries.iter().find(|e| &e.key == key) else {
                continue;
            };
            match (field, &entry.value) {
                (Field::MultiText(tv), ConfigValue::Many(items)) => {
                    let lines: Vec<String> = items
                        .iter()
                        .filter_map(|it| match it {
                            ScalarValue::Text(s) | ScalarValue::Choice(s) => Some(s.clone()),
                            _ => None,
                        })
                        .collect();
                    tv.buffer().set_text(&lines.join("\n"));
                }
                (Field::Text(e), ConfigValue::One(ScalarValue::Text(s))) => e.set_text(s),
                _ => {}
            }
        }
    }
}

/// Wrap a control in a labelled option row.
fn field_row(spec: &OptionSpec, control: &gtk4::Widget) -> Box {
    let row = Box::new(Orientation::Vertical, 4);
    let header = Box::new(Orientation::Horizontal, 8);
    header.append(&title_label(&spec.label));
    if spec.required {
        header.append(&Label::new(Some("*")));
    }
    row.append(&header);
    if let Some(desc) = &spec.description {
        let desc_label = sub_label(desc);
        desc_label.set_wrap(true);
        row.append(&desc_label);
    }
    row.append(control);
    row
}

fn default_strings(spec: &OptionSpec) -> Vec<String> {
    match &spec.default {
        Some(ConfigValue::Many(items)) => items
            .iter()
            .filter_map(|it| match it {
                ScalarValue::Choice(s) | ScalarValue::Text(s) => Some(s.clone()),
                _ => None,
            })
            .collect(),
        Some(ConfigValue::One(ScalarValue::Text(s) | ScalarValue::Choice(s))) => vec![s.clone()],
        _ => Vec::new(),
    }
}

fn build_field(
    spec: &OptionSpec,
    source: &SourceConfig,
    weak_view: &RefCell<Weak<SourcesView>>,
) -> (Box, Field) {
    let weak = weak_view.borrow().clone();

    if spec.multiple {
        let mut existing = source.get_option_string_list(&spec.key);
        if existing.is_empty() {
            existing = default_strings(spec);
        }
        return match &spec.ty {
            ScalarType::Choice(choices) => {
                let checks_box = Box::new(Orientation::Vertical, 4);
                let mut checks = Vec::new();
                for choice in choices {
                    let chk = CheckButton::with_label(choice);
                    chk.set_active(existing.iter().any(|e| e == choice));
                    chk.connect_toggled(on_save!(weak));
                    checks_box.append(&chk);
                    checks.push((choice.clone(), chk));
                }
                (
                    field_row(spec, checks_box.upcast_ref()),
                    Field::MultiChoice(checks),
                )
            }
            _ => {
                let tv = TextView::new();
                tv.set_accepts_tab(false);
                tv.set_wrap_mode(WrapMode::Word);
                tv.set_monospace(true);
                tv.add_css_class("multiline-entry");
                tv.buffer().set_text(&existing.join("\n"));
                connect_leave(&tv, &weak, true);
                let keys = EventControllerKey::new();
                keys.connect_key_pressed({
                    let weak = weak.clone();
                    move |_, keyval, _, state| {
                        if state.contains(gdk::ModifierType::CONTROL_MASK)
                            && keyval == gdk::Key::Return
                        {
                            if let Some(v) = weak.upgrade() {
                                v.validate_current();
                            }
                            glib::Propagation::Stop
                        } else {
                            glib::Propagation::Proceed
                        }
                    }
                });
                tv.add_controller(keys);
                let scroll = ScrolledWindow::new();
                scroll.set_child(Some(&tv));
                scroll.set_min_content_height(100);
                scroll.set_has_frame(true);
                scroll.add_css_class("multiline-scroll");
                (field_row(spec, scroll.upcast_ref()), Field::MultiText(tv))
            }
        };
    }

    match &spec.ty {
        ScalarType::Boolean => {
            let chk = CheckButton::new();
            chk.set_active(source.get_option_bool(&spec.key).unwrap_or(matches!(
                spec.default,
                Some(ConfigValue::One(ScalarValue::Boolean(true)))
            )));
            chk.connect_toggled(on_save!(weak));
            (field_row(spec, chk.upcast_ref()), Field::Boolean(chk))
        }
        ScalarType::Integer(IntBounds { min, max }) => {
            let lo = min.unwrap_or(0) as f64;
            let spin = SpinButton::with_range(lo, max.unwrap_or(10_000) as f64, 1.0);
            spin.set_value(
                source
                    .get_option_int(&spec.key)
                    .map(|i| i as f64)
                    .or(match &spec.default {
                        Some(ConfigValue::One(ScalarValue::Integer(i))) => Some(*i as f64),
                        _ => None,
                    })
                    .unwrap_or(lo),
            );
            spin.connect_value_changed(on_save!(weak));
            (field_row(spec, spin.upcast_ref()), Field::Integer(spin))
        }
        ScalarType::Choice(choices) => {
            let drop =
                DropDown::from_strings(&choices.iter().map(|s| s.as_str()).collect::<Vec<_>>());
            let selected = initial_string(source, spec);
            if let Some(pos) = choices.iter().position(|c| c == &selected) {
                drop.set_selected(pos as u32);
            }
            drop.connect_selected_notify(on_save!(weak));
            (
                field_row(spec, drop.upcast_ref()),
                Field::Choice {
                    drop,
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
            connect_leave(&entry, &weak, false);
            (field_row(spec, entry.upcast_ref()), Field::Secret(entry))
        }
        ScalarType::Text => {
            let entry = Entry::new();
            entry.set_text(&initial_string(source, spec));
            entry.connect_activate(on_validate!(weak));
            connect_leave(&entry, &weak, true);
            (field_row(spec, entry.upcast_ref()), Field::Text(entry))
        }
    }
}
