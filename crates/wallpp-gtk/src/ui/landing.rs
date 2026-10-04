use gtk4::prelude::*;
use gtk4::{Box, Button, GestureClick, Grid, Label, Orientation, Overlay, Picture, ScrolledWindow, Widget};
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};
use wallpp::state::State;

use super::aspect_bin::AspectBin;
use super::timeline_grid::TimelineGrid;
use crate::backend::{Action, BackendHandle};

#[derive(Clone, Debug)]
pub enum WallpaperKind {
    Future(usize),
    Active,
    History(usize),
}

#[derive(Clone, Debug)]
pub struct WallpaperItem {
    pub title: Option<String>,
    pub author: Option<String>,
    pub provider: String,
    pub source_name: Option<String>,
    pub cache_path: PathBuf,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub kind: WallpaperKind,
    pub action: Action,
}

pub struct LandingView {
    pub container: Box,
    #[allow(dead_code)]
    preview_bin: AspectBin,
    preview_overlay: Overlay,
    inspector_title: Label,
    inspector_author: Label,
    inspector_source: Label,
    inspector_res: Label,
    inspector_badge: Label,
    inspector_apply_btn: Button,
    timeline: TimelineGrid,
    scroll: ScrolledWindow,
    selected_action: RefCell<Option<Action>>,
    pending_scroll: Cell<Option<usize>>,
    self_weak: RefCell<Weak<LandingView>>,
    backend: Rc<BackendHandle>,
}

impl LandingView {
    pub fn new(backend: Rc<BackendHandle>) -> Rc<Self> {
        let container = Box::new(Orientation::Vertical, 12);
        container.set_margin_start(16);
        container.set_margin_end(16);
        container.set_margin_top(12);
        container.set_margin_bottom(8);

        let aspect_ratio = match wallpp::monitor::get_biggest_monitor() {
            Some(m) if m.width > 0 && m.height > 0 => m.width as f64 / m.height as f64,
            _ => 16.0 / 9.0,
        };

        let inspector_grid = Grid::new();
        inspector_grid.set_column_homogeneous(true);
        inspector_grid.set_column_spacing(24);
        inspector_grid.set_vexpand(false);
        inspector_grid.set_valign(gtk4::Align::Start);

        let preview_bin = AspectBin::new(aspect_ratio);
        preview_bin.set_hexpand(true);
        preview_bin.set_valign(gtk4::Align::Center);

        let preview_overlay = Overlay::new();
        preview_overlay.add_css_class("timeline-card");
        preview_overlay.add_css_class("preview-card");
        preview_overlay.set_overflow(gtk4::Overflow::Hidden);
        preview_overlay.set_hexpand(true);
        preview_overlay.set_vexpand(true);

        preview_bin.set_child(Some(&preview_overlay));
        inspector_grid.attach(&preview_bin, 0, 0, 1, 1);

        let info_col = Box::new(Orientation::Vertical, 6);
        info_col.set_hexpand(true);
        info_col.set_valign(gtk4::Align::Center);

        let inspector_badge = Label::new(None);
        inspector_badge.add_css_class("badge-active");
        inspector_badge.set_halign(gtk4::Align::Start);
        info_col.append(&inspector_badge);

        let inspector_title = Label::new(None);
        inspector_title.add_css_class("card-title");
        inspector_title.set_halign(gtk4::Align::Start);
        inspector_title.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        info_col.append(&inspector_title);

        let inspector_author = Label::new(None);
        inspector_author.add_css_class("card-sub");
        inspector_author.set_halign(gtk4::Align::Start);
        inspector_author.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        info_col.append(&inspector_author);

        let inspector_source = Label::new(None);
        inspector_source.add_css_class("card-sub");
        inspector_source.set_halign(gtk4::Align::Start);
        inspector_source.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        info_col.append(&inspector_source);

        let inspector_res = Label::new(None);
        inspector_res.add_css_class("card-sub");
        inspector_res.set_halign(gtk4::Align::Start);
        inspector_res.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        info_col.append(&inspector_res);

        let inspector_apply_btn = Button::with_label("Use now");
        inspector_apply_btn.add_css_class("suggested-action");
        inspector_apply_btn.set_halign(gtk4::Align::Start);
        inspector_apply_btn.set_margin_top(6);
        info_col.append(&inspector_apply_btn);

        inspector_grid.attach(&info_col, 1, 0, 1, 1);
        container.append(&inspector_grid);

        let scroll = ScrolledWindow::builder()
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .vscrollbar_policy(gtk4::PolicyType::Automatic)
            .vexpand(true)
            .hexpand(true)
            .build();

        let timeline = TimelineGrid::new(aspect_ratio);
        timeline.set_valign(gtk4::Align::Start);
        timeline.set_margin_top(12);
        timeline.set_margin_bottom(16);

        scroll.set_child(Some(&timeline));
        container.append(&scroll);

        let landing = Rc::new(Self {
            container,
            preview_bin,
            preview_overlay,
            inspector_title,
            inspector_author,
            inspector_source,
            inspector_res,
            inspector_badge,
            inspector_apply_btn,
            timeline,
            scroll,
            selected_action: RefCell::new(None),
            pending_scroll: Cell::new(None),
            self_weak: RefCell::new(Weak::new()),
            backend,
        });

        *landing.self_weak.borrow_mut() = Rc::downgrade(&landing);

        let weak = Rc::downgrade(&landing);
        landing.inspector_apply_btn.connect_clicked(move |_| {
            if let Some(view) = weak.upgrade() {
                if let Some(action) = view.selected_action.borrow().clone() {
                    view.backend.send(action);
                }
            }
        });

        let vadj = landing.scroll.vadjustment();

        let weak = Rc::downgrade(&landing);
        vadj.connect_notify_local(Some("page-size"), move |adj, _| {
            if let Some(view) = weak.upgrade() {
                view.timeline.set_viewport_height(adj.page_size() as i32);
                view.apply_pending_scroll();
            }
        });

        let weak = Rc::downgrade(&landing);
        vadj.connect_notify_local(Some("upper"), move |_, _| {
            if let Some(view) = weak.upgrade() {
                view.apply_pending_scroll();
            }
        });

        landing
    }

    pub fn show_details(&self, item: &WallpaperItem) {
        self.preview_overlay
            .set_child(Some(&make_preview_thumbnail(&item.cache_path)));

        self.preview_overlay.remove_css_class("future-card");
        self.preview_overlay.remove_css_class("active-card");
        self.preview_overlay.remove_css_class("history-card");

        match item.kind {
            WallpaperKind::Future(_) => self.preview_overlay.add_css_class("future-card"),
            WallpaperKind::Active => self.preview_overlay.add_css_class("active-card"),
            WallpaperKind::History(_) => self.preview_overlay.add_css_class("history-card"),
        }

        self.inspector_title
            .set_text(item.title.as_deref().unwrap_or("Untitled"));

        if let Some(ref a) = item.author {
            self.inspector_author.set_text(&format!("by {}", a));
            self.inspector_author.set_visible(true);
        } else {
            self.inspector_author.set_visible(false);
        }

        let src = item.source_name.as_deref().unwrap_or(&item.provider);
        self.inspector_source
            .set_text(&format!("Source: {} [{}]", src, item.provider));

        let dim = match (item.width, item.height) {
            (Some(w), Some(h)) => format!("Resolution: {}x{}", w, h),
            _ => "Resolution: Unknown".to_string(),
        };
        self.inspector_res.set_text(&dim);

        self.inspector_badge.remove_css_class("badge-future");
        self.inspector_badge.remove_css_class("badge-active");
        self.inspector_badge.remove_css_class("badge-history");

        match item.kind {
            WallpaperKind::Future(idx) => {
                self.inspector_badge
                    .set_text(&format!("Planned #{}", idx + 1));
                self.inspector_badge.add_css_class("badge-future");
                self.inspector_apply_btn.set_sensitive(true);
                self.inspector_apply_btn.set_label("Use now");
            }
            WallpaperKind::Active => {
                self.inspector_badge.set_text("★ Current");
                self.inspector_badge.add_css_class("badge-active");
                self.inspector_apply_btn.set_sensitive(false);
                self.inspector_apply_btn.set_label("Current wallpaper");
            }
            WallpaperKind::History(idx) => {
                self.inspector_badge.set_text(&format!("Past #{}", idx + 1));
                self.inspector_badge.add_css_class("badge-history");
                self.inspector_apply_btn.set_sensitive(true);
                self.inspector_apply_btn.set_label("Use now");
            }
        }

        *self.selected_action.borrow_mut() = Some(item.action.clone());
    }

    /// Centers the pending cell in the viewport once the grid has been allocated.
    fn apply_pending_scroll(&self) {
        let Some(index) = self.pending_scroll.get() else {
            return;
        };
        let Some((y, h)) = self.timeline.cell_extent(index) else {
            return;
        };
        let vadj = self.scroll.vadjustment();
        let page = vadj.page_size();
        let y = (y + self.timeline.margin_top()) as f64;
        if page <= 0.0 || vadj.upper() < y + h as f64 {
            return;
        }
        let max = (vadj.upper() - page).max(0.0);
        vadj.set_value((y - (page - h as f64) / 2.0).clamp(0.0, max));
        self.pending_scroll.set(None);
    }

    pub fn update(&self, state: &State) {
        let mut items: Vec<WallpaperItem> = Vec::new();
        let mut active_idx = None;

        for (i, p) in state.prefetch_queue.iter().enumerate().rev() {
            items.push(WallpaperItem {
                title: p.title.clone(),
                author: p.author.clone(),
                provider: p.provider.clone(),
                source_name: p.source_name.clone(),
                cache_path: p.cache_path.clone(),
                width: p.width,
                height: p.height,
                kind: WallpaperKind::Future(i),
                action: Action::SelectPlanned(i),
            });
        }

        if let Some(active) = state.history.get(state.current_history_index) {
            active_idx = Some(items.len());
            items.push(WallpaperItem {
                title: active.title.clone(),
                author: active.author.clone(),
                provider: active.provider.clone(),
                source_name: active.source_name.clone(),
                cache_path: active.cache_path.clone(),
                width: active.width,
                height: active.height,
                kind: WallpaperKind::Active,
                action: Action::SelectHistory(state.current_history_index),
            });
        }

        for (i, h) in state.history.iter().enumerate() {
            if i == state.current_history_index {
                continue;
            }
            items.push(WallpaperItem {
                title: h.title.clone(),
                author: h.author.clone(),
                provider: h.provider.clone(),
                source_name: h.source_name.clone(),
                cache_path: h.cache_path.clone(),
                width: h.width,
                height: h.height,
                kind: WallpaperKind::History(i),
                action: Action::SelectHistory(i),
            });
        }

        if let Some(item) = active_idx.and_then(|i| items.get(i)).or(items.first()) {
            self.show_details(item);
        }

        self.timeline.clear();
        let weak = self.self_weak.borrow().clone();
        for item in &items {
            self.timeline.append(&build_card(item, &weak, &self.backend));
        }

        self.pending_scroll.set(active_idx);
        let weak = self.self_weak.borrow().clone();
        glib::idle_add_local_once(move || {
            if let Some(view) = weak.upgrade() {
                view.apply_pending_scroll();
            }
        });
    }
}

fn build_card(item: &WallpaperItem, landing_weak: &Weak<LandingView>, backend: &Rc<BackendHandle>) -> Overlay {
    let overlay = Overlay::new();
    overlay.add_css_class("timeline-card");
    overlay.set_overflow(gtk4::Overflow::Hidden);

    match item.kind {
        WallpaperKind::Future(_) => overlay.add_css_class("future-card"),
        WallpaperKind::Active => overlay.add_css_class("active-card"),
        WallpaperKind::History(_) => overlay.add_css_class("history-card"),
    }

    overlay.set_child(Some(&make_grid_thumbnail(&item.cache_path)));

    let (badge_text, badge_class) = match item.kind {
        WallpaperKind::Future(i) => (format!("Future #{}", i + 1), "badge-future"),
        WallpaperKind::Active => ("★ ACTIVE".to_string(), "badge-active"),
        WallpaperKind::History(i) => (format!("Past #{}", i + 1), "badge-history"),
    };

    let badge = Label::new(Some(&badge_text));
    badge.add_css_class(badge_class);
    badge.set_halign(gtk4::Align::End);
    badge.set_valign(gtk4::Align::Start);
    badge.set_margin_top(6);
    badge.set_margin_end(6);
    overlay.add_overlay(&badge);

    let src_label = item.source_name.as_deref().unwrap_or(&item.provider);
    let src_badge = Label::new(Some(src_label));
    src_badge.add_css_class("badge-source");
    src_badge.set_halign(gtk4::Align::Start);
    src_badge.set_valign(gtk4::Align::End);
    src_badge.set_margin_bottom(6);
    src_badge.set_margin_start(6);
    src_badge.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    overlay.add_overlay(&src_badge);

    let title_str = item.title.as_deref().unwrap_or("Untitled");
    let tooltip_str = match &item.author {
        Some(author) => format!("{}\n{}", title_str, author),
        None => title_str.to_string(),
    };
    overlay.set_tooltip_text(Some(&tooltip_str));

    let gesture = GestureClick::new();
    let item_data = item.clone();
    let l_weak = landing_weak.clone();
    let b_handle = backend.clone();
    gesture.connect_pressed(move |_, n_press, _, _| {
        if let Some(view) = l_weak.upgrade() {
            if n_press == 1 {
                view.show_details(&item_data);
            } else if n_press == 2 {
                b_handle.send(item_data.action.clone());
            }
        }
    });
    overlay.add_controller(gesture);

    overlay
}

fn make_grid_thumbnail(path: &Path) -> Widget {
    if path.exists() {
        let pic = Picture::for_filename(path);
        pic.set_can_shrink(true);
        pic.set_content_fit(gtk4::ContentFit::Cover);
        pic.upcast()
    } else {
        Label::new(Some("No image")).upcast()
    }
}

fn make_preview_thumbnail(path: &Path) -> Widget {
    if path.exists() {
        let pic = Picture::for_filename(path);
        pic.set_can_shrink(true);
        pic.set_content_fit(gtk4::ContentFit::Cover);
        pic.set_hexpand(true);
        pic.set_vexpand(true);
        pic.upcast()
    } else {
        let placeholder = Label::new(Some("No preview available"));
        placeholder.set_hexpand(true);
        placeholder.set_vexpand(true);
        placeholder.upcast()
    }
}
