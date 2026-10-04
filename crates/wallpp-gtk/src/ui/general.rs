use gtk4::prelude::*;
use gtk4::{
    Align, Box, CheckButton, DropDown, Entry, EventControllerFocus, Label, Orientation,
    ScrolledWindow, SpinButton,
};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use wallpp::config::{AppConfig, ByteSize, RefreshStrategy, SourceStrategy};

use crate::backend::{Action, BackendHandle};

pub struct GeneralView {
    pub container: Box,
    config: RefCell<AppConfig>,
    backend: Rc<BackendHandle>,
    dirty: Rc<Cell<bool>>,
    cache_entry: Entry,
    history_spin: SpinButton,
    prefetch_spin: SpinButton,
    interval_entry: Entry,
    boot_check: CheckButton,
    refresh_strat_drop: DropDown,
    source_strat_drop: DropDown,
    display_percent_spin: SpinButton,
    feedback_label: Label,
}

impl GeneralView {
    pub fn new(
        backend: Rc<BackendHandle>,
        initial_config: &AppConfig,
        dirty: Rc<Cell<bool>>,
    ) -> Rc<Self> {
        let container = Box::new(Orientation::Vertical, 0);
        container.set_vexpand(true);
        container.set_hexpand(true);

        let scroll = ScrolledWindow::builder()
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .vscrollbar_policy(gtk4::PolicyType::Automatic)
            .vexpand(true)
            .hexpand(true)
            .build();

        let content = Box::new(Orientation::Vertical, 16);
        content.set_margin_start(24);
        content.set_margin_end(24);
        content.set_margin_top(20);
        content.set_margin_bottom(24);
        content.set_halign(Align::Center);
        content.set_size_request(540, -1);

        let title_box = Box::new(Orientation::Vertical, 4);
        let title = Label::new(Some("General Settings"));
        title.add_css_class("card-title");
        title.set_halign(Align::Start);
        let sub = Label::new(Some(
            "Configure wallpaper management, cache, and automatic refresh rules",
        ));
        sub.add_css_class("card-sub");
        sub.set_halign(Align::Start);
        title_box.append(&title);
        title_box.append(&sub);
        content.append(&title_box);

        let mgr = &initial_config.manager;

        // Cache Max Size
        let cache_entry = Entry::new();
        cache_entry.set_text(&mgr.cache_max_size.to_string());
        cache_entry.set_placeholder_text(Some("e.g. 1G, 500M"));
        content.append(&build_form_row(
            "Cache Max Size",
            "Maximum disk space used for cached wallpapers",
            &cache_entry,
        ));

        // History Length
        let history_spin = SpinButton::with_range(1.0, 100_000.0, 10.0);
        history_spin.set_value(mgr.history_length as f64);
        content.append(&build_form_row(
            "History Length",
            "Maximum number of past wallpapers to retain in history",
            &history_spin,
        ));

        // Prefetch Count
        let prefetch_spin = SpinButton::with_range(1.0, 50.0, 1.0);
        prefetch_spin.set_value(mgr.prefetch_count as f64);
        content.append(&build_form_row(
            "Planned Queue Count",
            "Number of upcoming wallpapers to prefetch and display in queue",
            &prefetch_spin,
        ));

        // Refresh Interval
        let interval_entry = Entry::new();
        let interval_str = mgr.refresh_interval_str();
        interval_entry.set_text(&interval_str);
        interval_entry.set_placeholder_text(Some("e.g. 1h, 30m (leave empty for manual only)"));
        content.append(&build_form_row(
            "Refresh Interval",
            "Automatic wallpaper rotation interval (e.g. 1h, 30m)",
            &interval_entry,
        ));

        // Refresh At Boot
        let boot_check = CheckButton::new();
        boot_check.set_active(mgr.refresh_at_boot);
        boot_check.set_valign(Align::Center);
        content.append(&build_form_row(
            "Refresh At Boot",
            "Change wallpaper on system startup",
            &boot_check,
        ));

        // Refresh Strategy
        let refresh_strat_drop = DropDown::from_strings(&["From Boot", "From Last Changed"]);
        refresh_strat_drop.set_selected(match mgr.refresh_strategy {
            RefreshStrategy::FromBoot => 0,
            RefreshStrategy::FromLastChanged => 1,
        });
        content.append(&build_form_row(
            "Refresh Timing Strategy",
            "Calculate interval from system boot or from last wallpaper transition",
            &refresh_strat_drop,
        ));

        // Source Strategy
        let source_strat_drop = DropDown::from_strings(&["Random", "Round-Robin"]);
        source_strat_drop.set_selected(match mgr.source_strategy {
            SourceStrategy::Random => 0,
            SourceStrategy::RoundRobin => 1,
        });
        content.append(&build_form_row(
            "Source Selection Strategy",
            "Algorithm for selecting which source to pick the next wallpaper from",
            &source_strat_drop,
        ));

        // Min Display Percentage
        let display_percent_spin = SpinButton::with_range(1.0, 200.0, 5.0);
        display_percent_spin.set_value(mgr.min_display_percentage as f64);
        content.append(&build_form_row(
            "Minimum Display Percentage",
            "Minimum wallpaper resolution relative to biggest display (%)",
            &display_percent_spin,
        ));

        // Feedback label & Save button
        let feedback_label = Label::new(None);
        feedback_label.set_halign(Align::Start);
        feedback_label.set_margin_top(8);
        content.append(&feedback_label);

        scroll.set_child(Some(&content));
        container.append(&scroll);

        let view = Rc::new(Self {
            container,
            config: RefCell::new(initial_config.clone()),
            backend,
            dirty,
            cache_entry,
            history_spin,
            prefetch_spin,
            interval_entry,
            boot_check,
            refresh_strat_drop,
            source_strat_drop,
            display_percent_spin,
            feedback_label,
        });

        let view_weak = Rc::downgrade(&view);

        // Auto-save on any change
        let vw = view_weak.clone();
        view.history_spin.connect_value_changed(move |_| {
            if let Some(v) = vw.upgrade() {
                v.save_and_apply();
            }
        });
        let vw = view_weak.clone();
        view.prefetch_spin.connect_value_changed(move |_| {
            if let Some(v) = vw.upgrade() {
                v.save_and_apply();
            }
        });
        let vw = view_weak.clone();
        view.display_percent_spin.connect_value_changed(move |_| {
            if let Some(v) = vw.upgrade() {
                v.save_and_apply();
            }
        });
        let vw = view_weak.clone();
        view.boot_check.connect_toggled(move |_| {
            if let Some(v) = vw.upgrade() {
                v.save_and_apply();
            }
        });
        let vw = view_weak.clone();
        view.refresh_strat_drop.connect_selected_notify(move |_| {
            if let Some(v) = vw.upgrade() {
                v.save_and_apply();
            }
        });
        let vw = view_weak.clone();
        view.source_strat_drop.connect_selected_notify(move |_| {
            if let Some(v) = vw.upgrade() {
                v.save_and_apply();
            }
        });

        let vw = view_weak.clone();
        view.cache_entry.connect_activate(move |_| {
            if let Some(v) = vw.upgrade() {
                v.save_and_apply();
            }
        });
        let fc = EventControllerFocus::new();
        let vw = view_weak.clone();
        fc.connect_leave(move |_| {
            if let Some(v) = vw.upgrade() {
                v.save_and_apply();
            }
        });
        view.cache_entry.add_controller(fc);

        let vw = view_weak.clone();
        view.interval_entry.connect_activate(move |_| {
            if let Some(v) = vw.upgrade() {
                v.save_and_apply();
            }
        });
        let fc = EventControllerFocus::new();
        let vw = view_weak.clone();
        fc.connect_leave(move |_| {
            if let Some(v) = vw.upgrade() {
                v.save_and_apply();
            }
        });
        view.interval_entry.add_controller(fc);

        view
    }

    #[allow(dead_code)]
    pub fn update_config(&self, new_cfg: &AppConfig) {
        *self.config.borrow_mut() = new_cfg.clone();
        let mgr = &new_cfg.manager;
        self.cache_entry.set_text(&mgr.cache_max_size.to_string());
        self.history_spin.set_value(mgr.history_length as f64);
        self.prefetch_spin.set_value(mgr.prefetch_count as f64);
        let interval_str = mgr.refresh_interval_str();
        self.interval_entry.set_text(&interval_str);
        self.boot_check.set_active(mgr.refresh_at_boot);
        self.refresh_strat_drop
            .set_selected(match mgr.refresh_strategy {
                RefreshStrategy::FromBoot => 0,
                RefreshStrategy::FromLastChanged => 1,
            });
        self.source_strat_drop
            .set_selected(match mgr.source_strategy {
                SourceStrategy::Random => 0,
                SourceStrategy::RoundRobin => 1,
            });
        self.display_percent_spin
            .set_value(mgr.min_display_percentage as f64);
    }

    fn save_and_apply(&self) {
        let mut cfg = self.config.borrow().clone();

        // Validate & parse cache max size
        let cache_str = self.cache_entry.text().to_string();
        let cache_parsed: ByteSize = match cache_str.trim().parse() {
            Ok(b) => b,
            Err(e) => {
                self.feedback_label
                    .set_text(&format!("Invalid cache size '{}': {}", cache_str, e));
                self.feedback_label.remove_css_class("badge-active");
                self.feedback_label.add_css_class("badge-history");
                return;
            }
        };

        // Validate & parse refresh interval
        let interval_str = self.interval_entry.text().trim().to_string();
        if let Err(e) = cfg.manager.set_refresh_interval_from_str(&interval_str) {
            self.feedback_label
                .set_text(&format!("Invalid refresh interval: {}", e));
            self.feedback_label.remove_css_class("badge-active");
            self.feedback_label.add_css_class("badge-history");
            return;
        }

        cfg.manager.cache_max_size = cache_parsed;
        cfg.manager.history_length = self.history_spin.value() as usize;
        cfg.manager.prefetch_count = self.prefetch_spin.value() as usize;
        cfg.manager.refresh_at_boot = self.boot_check.is_active();
        cfg.manager.refresh_strategy = match self.refresh_strat_drop.selected() {
            0 => RefreshStrategy::FromBoot,
            _ => RefreshStrategy::FromLastChanged,
        };
        cfg.manager.source_strategy = match self.source_strat_drop.selected() {
            0 => SourceStrategy::Random,
            _ => SourceStrategy::RoundRobin,
        };
        cfg.manager.min_display_percentage = self.display_percent_spin.value() as u32;

        if let Err(e) = cfg.save() {
            self.feedback_label
                .set_text(&format!("Failed to save config: {}", e));
            self.feedback_label.remove_css_class("badge-active");
            self.feedback_label.add_css_class("badge-history");
            return;
        }

        self.dirty.set(true);
        *self.config.borrow_mut() = cfg.clone();
        self.backend.send(Action::UpdateConfig(cfg));
        self.feedback_label.set_text("");
    }
}

fn build_form_row(title: &str, desc: &str, control: &impl IsA<gtk4::Widget>) -> Box {
    let row = Box::new(Orientation::Horizontal, 16);
    row.set_hexpand(true);

    let text_box = Box::new(Orientation::Vertical, 2);
    text_box.set_hexpand(true);
    text_box.set_halign(Align::Start);
    text_box.set_valign(Align::Center);

    let title_lbl = Label::new(Some(title));
    title_lbl.set_halign(Align::Start);
    title_lbl.add_css_class("card-title");

    let desc_lbl = Label::new(Some(desc));
    desc_lbl.set_halign(Align::Start);
    desc_lbl.add_css_class("card-sub");

    text_box.append(&title_lbl);
    text_box.append(&desc_lbl);

    control.set_halign(Align::End);
    control.set_valign(Align::Center);

    row.append(&text_box);
    row.append(control);
    row
}
