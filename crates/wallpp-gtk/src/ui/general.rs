//! General page: manager settings (cache, history, queue, refresh,
//! strategy), auto-saved on change. Persistence happens in the backend
//! via `Action::UpdateConfig`; this page only validates locally.

use gtk4::prelude::*;
use gtk4::{
    Align, Box, CheckButton, DropDown, Entry, EventControllerFocus, Label, Orientation,
    ScrolledWindow, SpinButton,
};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use wallpp::config::{AppConfig, ByteSize, RefreshStrategy, SourceStrategy};

use super::widgets::{page_box, sub_label, title_label};
use crate::backend::{Action, BackendHandle};

pub struct GeneralView {
    pub container: Box,
    cfg: Rc<RefCell<AppConfig>>,
    backend: BackendHandle,
    dirty: Rc<Cell<bool>>,
    cache: Entry,
    history: SpinButton,
    prefetch: SpinButton,
    interval: Entry,
    boot: CheckButton,
    refresh_strategy: DropDown,
    source_strategy: DropDown,
    display_percent: SpinButton,
    feedback: Label,
}

/// Two-column form grid: text in column 0 (expands), controls in column 1.
/// A shared grid sizes the control column to the widest widget, so every
/// control aligns on both edges.
fn form_grid(rows: &[(&str, &str, gtk4::Widget)]) -> gtk4::Grid {
    let grid = gtk4::Grid::new();
    grid.set_column_spacing(16);
    grid.set_row_spacing(16);
    for (i, (title, desc, control)) in rows.iter().enumerate() {
        let text = Box::new(Orientation::Vertical, 2);
        text.set_hexpand(true);
        text.set_valign(gtk4::Align::Center);
        text.append(&title_label(title));
        text.append(&sub_label(desc));
        control.set_halign(Align::Fill);
        control.set_valign(gtk4::Align::Center);
        grid.attach(&text, 0, i as i32, 1, 1);
        grid.attach(control, 1, i as i32, 1, 1);
    }
    grid
}

macro_rules! bind {
    ($view:expr, $widget:expr, $signal:ident) => {{
        let weak = Rc::downgrade(&$view);
        $widget.$signal(move |_| {
            if let Some(v) = weak.upgrade() {
                v.save();
            }
        });
    }};
}

fn bind_entry(entry: &Entry, view: &Rc<GeneralView>) {
    bind!(view, entry, connect_activate);
    let weak = Rc::downgrade(view);
    let focus = EventControllerFocus::new();
    focus.connect_leave(move |_| {
        if let Some(v) = weak.upgrade() {
            v.save();
        }
    });
    entry.add_controller(focus);
}

impl GeneralView {
    pub fn new(
        backend: BackendHandle,
        cfg: Rc<RefCell<AppConfig>>,
        dirty: Rc<Cell<bool>>,
    ) -> Rc<Self> {
        let mgr = cfg.borrow().manager.clone();

        let cache = Entry::new();
        cache.set_text(&mgr.cache_max_size.to_string());
        cache.set_placeholder_text(Some("e.g. 1G, 500M"));
        let history = SpinButton::with_range(1.0, 100_000.0, 10.0);
        history.set_value(mgr.history_length as f64);
        let prefetch = SpinButton::with_range(1.0, 50.0, 1.0);
        prefetch.set_value(mgr.prefetch_count as f64);
        let interval = Entry::new();
        interval.set_text(&mgr.refresh_interval_str());
        interval.set_placeholder_text(Some("e.g. 1h, 30m (leave empty for manual only)"));
        let boot = CheckButton::new();
        boot.set_active(mgr.refresh_at_boot);
        let refresh_strategy = DropDown::from_strings(&["From Boot", "From Last Changed"]);
        refresh_strategy.set_selected(match mgr.refresh_strategy {
            RefreshStrategy::FromBoot => 0,
            RefreshStrategy::FromLastChanged => 1,
        });
        let source_strategy = DropDown::from_strings(&["Random", "Round-Robin"]);
        source_strategy.set_selected(match mgr.source_strategy {
            SourceStrategy::Random => 0,
            SourceStrategy::RoundRobin => 1,
        });
        let display_percent = SpinButton::with_range(1.0, 200.0, 5.0);
        display_percent.set_value(mgr.min_display_percentage as f64);
        let feedback = Label::new(None);
        feedback.set_halign(Align::Start);
        feedback.set_margin_top(8);

        let content = page_box(16);
        content.set_halign(Align::Center);
        content.set_size_request(540, -1);
        let header = Box::new(Orientation::Vertical, 4);
        header.append(&title_label("General Settings"));
        header.append(&sub_label(
            "Configure wallpaper management, cache, and automatic refresh rules",
        ));
        content.append(&header);
        content.append(&form_grid(&[
            (
                "Cache Max Size",
                "Maximum disk space used for cached wallpapers",
                cache.clone().upcast(),
            ),
            (
                "History Length",
                "Maximum number of past wallpapers to retain in history",
                history.clone().upcast(),
            ),
            (
                "Planned Queue Count",
                "Number of upcoming wallpapers to prefetch and display in queue",
                prefetch.clone().upcast(),
            ),
            (
                "Refresh Interval",
                "Automatic wallpaper rotation interval (e.g. 1h, 30m)",
                interval.clone().upcast(),
            ),
            (
                "Refresh At Boot",
                "Change wallpaper on system startup",
                boot.clone().upcast(),
            ),
            (
                "Refresh Timing Strategy",
                "Calculate interval from system boot or from last wallpaper transition",
                refresh_strategy.clone().upcast(),
            ),
            (
                "Source Selection Strategy",
                "Algorithm for selecting which source to pick the next wallpaper from",
                source_strategy.clone().upcast(),
            ),
            (
                "Minimum Display Percentage",
                "Minimum wallpaper resolution relative to biggest display (%)",
                display_percent.clone().upcast(),
            ),
        ]));
        content.append(&feedback);

        let scroll = ScrolledWindow::builder()
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .vscrollbar_policy(gtk4::PolicyType::Automatic)
            .vexpand(true)
            .hexpand(true)
            .child(&content)
            .build();
        let container = Box::new(Orientation::Vertical, 0);
        container.append(&scroll);

        let view = Rc::new(Self {
            container,
            cfg,
            backend,
            dirty,
            cache,
            history,
            prefetch,
            interval,
            boot,
            refresh_strategy,
            source_strategy,
            display_percent,
            feedback,
        });

        bind!(view, view.history, connect_value_changed);
        bind!(view, view.prefetch, connect_value_changed);
        bind!(view, view.display_percent, connect_value_changed);
        bind!(view, view.boot, connect_toggled);
        bind!(view, view.refresh_strategy, connect_selected_notify);
        bind!(view, view.source_strategy, connect_selected_notify);
        bind_entry(&view.cache, &view);
        bind_entry(&view.interval, &view);
        view
    }

    fn fail(&self, msg: &str) {
        self.feedback.set_text(msg);
    }

    fn save(&self) {
        let cache_size: ByteSize = match self.cache.text().trim().parse() {
            Ok(b) => b,
            Err(e) => return self.fail(&format!("Invalid cache size: {e}")),
        };
        let mut cfg = self.cfg.borrow().clone();
        if let Err(e) = cfg
            .manager
            .set_refresh_interval_from_str(self.interval.text().trim())
        {
            return self.fail(&format!("Invalid refresh interval: {e}"));
        }
        cfg.manager.cache_max_size = cache_size;
        cfg.manager.history_length = self.history.value() as usize;
        cfg.manager.prefetch_count = self.prefetch.value() as usize;
        cfg.manager.refresh_at_boot = self.boot.is_active();
        cfg.manager.refresh_strategy = match self.refresh_strategy.selected() {
            0 => RefreshStrategy::FromBoot,
            _ => RefreshStrategy::FromLastChanged,
        };
        cfg.manager.source_strategy = match self.source_strategy.selected() {
            0 => SourceStrategy::Random,
            _ => SourceStrategy::RoundRobin,
        };
        cfg.manager.min_display_percentage = self.display_percent.value() as u32;

        *self.cfg.borrow_mut() = cfg.clone();
        self.dirty.set(true);
        self.backend.send(Action::UpdateConfig(cfg));
        self.feedback.set_text("");
    }
}
