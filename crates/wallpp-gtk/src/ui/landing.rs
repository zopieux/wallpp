//! Wallpapers page: large cover preview + inspector for the selected
//! wallpaper, and the planned/active/history thumbnail timeline below.

use gtk4::prelude::*;
use gtk4::{
    Box, Button, GestureClick, Label, Orientation, Overlay, Picture, ScrolledWindow, Spinner,
    Widget,
};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::{Rc, Weak};
use wallpp::state::{PrefetchedWallpaper, State, WallpaperMetadata};

use super::aspect_bin::AspectBin;
use super::timeline_grid::TimelineGrid;
use super::widgets::{badge, cover_overlay, cover_picture, sub_label, title_label};
use crate::backend::{Action, BackendHandle};
use crate::stack;

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Planned(usize),
    Active(usize),
    Past(usize),
}

impl Kind {
    /// (card badge, inspector badge, css class, apply-button label or disabled)
    fn labels(self) -> (String, String, &'static str, Option<&'static str>) {
        match self {
            Kind::Planned(i) => (
                format!("Planned #{}", i + 1),
                format!("Planned #{}", i + 1),
                "planned-card",
                Some("Use now"),
            ),
            Kind::Active(_) => ("★ ACTIVE".into(), "★ Current".into(), "active-card", None),
            Kind::Past(i) => (
                format!("Past #{}", i + 1),
                format!("Past #{}", i + 1),
                "history-card",
                Some("Use now"),
            ),
        }
    }

    fn badge_class(self) -> &'static str {
        match self {
            Kind::Planned(_) => "badge-planned",
            Kind::Active(_) => "badge-active",
            Kind::Past(_) => "badge-history",
        }
    }

    fn action(self) -> Action {
        match self {
            Kind::Planned(i) => Action::ApplyPlanned(i),
            Kind::Active(i) | Kind::Past(i) => Action::ApplyHistory(i),
        }
    }
}

#[derive(Clone, PartialEq)]
struct Item {
    kind: Kind,
    title: Option<String>,
    author: Option<String>,
    provider: String,
    source: Option<String>,
    path: PathBuf,
    thumbnail_path: PathBuf,
    width: Option<u32>,
    height: Option<u32>,
}

impl Item {
    fn tooltip(&self) -> String {
        match &self.author {
            Some(a) => format!("{}\n{}", self.title.as_deref().unwrap_or("Untitled"), a),
            None => self.title.as_deref().unwrap_or("Untitled").to_string(),
        }
    }
}

impl Item {
    #[allow(clippy::too_many_arguments)]
    fn new(
        kind: Kind,
        title: &Option<String>,
        author: &Option<String>,
        provider: &str,
        source: &Option<String>,
        path: &std::path::Path,
        width: Option<u32>,
        height: Option<u32>,
    ) -> Self {
        Self {
            kind,
            title: title.clone(),
            author: author.clone(),
            provider: provider.to_string(),
            source: source.clone(),
            path: path.to_path_buf(),
            thumbnail_path: wallpp::cache::thumbnail_path_for_original(path),
            width,
            height,
        }
    }
}

impl From<&PrefetchedWallpaper> for Item {
    fn from(p: &PrefetchedWallpaper) -> Self {
        // Kind is patched in by the caller, which knows the queue index.
        Self::new(
            Kind::Planned(0),
            &p.title,
            &p.author,
            &p.provider,
            &p.source_name,
            &p.cache_path,
            p.width,
            p.height,
        )
    }
}

impl From<&WallpaperMetadata> for Item {
    fn from(m: &WallpaperMetadata) -> Self {
        Self::new(
            Kind::Past(0),
            &m.title,
            &m.author,
            &m.provider,
            &m.source_name,
            &m.cache_path,
            m.width,
            m.height,
        )
    }
}

/// Set a badge label's text and kind class, clearing the other kind classes.
fn set_badge(label: &Label, text: &str, class: &str) {
    for c in ["badge-planned", "badge-active", "badge-history"] {
        label.remove_css_class(c);
    }
    label.add_css_class(class);
    label.set_text(text);
    label.set_visible(true);
}

pub struct LandingView {
    pub container: Box,
    preview: Overlay,
    preview_bin: AspectBin,
    badge_label: Label,
    title: Label,
    author: Label,
    source: Label,
    resolution: Label,
    apply_btn: Button,
    grid: TimelineGrid,
    spinner: Spinner,
    /// Decoded image widgets keyed by (cache path, inspector vs grid) —
    /// a widget can only live in one parent, so the two uses cache
    /// separately. Reused across grid rebuilds so progress updates
    /// don't re-decode every thumbnail.
    pictures: RefCell<HashMap<(PathBuf, bool), Picture>>,
    shown_path: RefCell<Option<PathBuf>>,
    rendered_items: RefCell<Vec<Item>>,
    selected: RefCell<Option<Kind>>,
    weak_self: RefCell<Weak<LandingView>>,
    backend: BackendHandle,
}

impl LandingView {
    pub fn new(backend: BackendHandle) -> Rc<Self> {
        // Start at 16:9; `set_aspect` switches to the real monitor
        // aspect as soon as it has been probed off the GTK thread.
        let aspect_ratio = 16.0 / 9.0;
        let container = Box::new(Orientation::Vertical, 12);
        container.set_margin_start(16);
        container.set_margin_end(16);
        container.set_margin_top(12);
        container.set_margin_bottom(8);

        // Inspector: cover preview on the left, details on the right,
        // each occupying half the window width.
        let top = gtk4::Grid::new();
        top.set_column_homogeneous(true);
        top.set_column_spacing(24);
        let preview = Overlay::new();
        preview.add_css_class("timeline-card");
        preview.set_overflow(gtk4::Overflow::Hidden);
        // Half the window width (left column) at the monitor's aspect
        // ratio, exactly like the timeline thumbnails below: the bin
        // derives its height from the allocated width, so the cover
        // picture is cropped to the screen's shape, never stretched.
        let preview_bin = AspectBin::new(aspect_ratio);
        preview_bin.set_child(&preview);
        preview_bin.set_hexpand(true);
        top.attach(&preview_bin, 0, 0, 1, 1);

        let badge_label = badge("", "badge-active", gtk4::Align::Start, gtk4::Align::Start);
        let title = title_label("");
        title.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        let author = sub_label("");
        let source = sub_label("");
        let resolution = sub_label("");
        let apply_btn = Button::with_label("Use now");
        apply_btn.add_css_class("suggested-action");
        apply_btn.set_halign(gtk4::Align::Start);
        apply_btn.set_margin_top(6);
        let info = stack!(
            Orientation::Vertical,
            6,
            badge_label,
            title,
            author,
            source,
            resolution,
            apply_btn
        );
        info.set_hexpand(true);
        info.set_valign(gtk4::Align::Center);
        top.attach(&info, 1, 0, 1, 1);
        container.append(&top);

        let grid = TimelineGrid::new(aspect_ratio);
        grid.set_valign(gtk4::Align::Start);
        grid.set_margin_top(12);
        grid.set_margin_bottom(16);
        let scroll = ScrolledWindow::builder()
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .vscrollbar_policy(gtk4::PolicyType::Automatic)
            .vexpand(true)
            .hexpand(true)
            .child(&grid)
            .build();
        // Spinner overlaid on the timeline while the planned queue
        // refills; the page stays responsive underneath.
        let spinner = Spinner::new();
        spinner.set_size_request(48, 48);
        spinner.set_halign(gtk4::Align::Center);
        spinner.set_valign(gtk4::Align::Center);
        spinner.set_can_target(false);
        spinner.set_visible(false);
        let scroll_overlay = Overlay::new();
        scroll_overlay.set_child(Some(&scroll));
        scroll_overlay.add_overlay(&spinner);
        container.append(&scroll_overlay);

        let view = Rc::new(Self {
            container,
            preview,
            preview_bin,
            badge_label,
            title,
            author,
            source,
            resolution,
            apply_btn,
            grid,
            spinner,
            pictures: RefCell::new(HashMap::new()),
            shown_path: RefCell::new(None),
            rendered_items: RefCell::new(Vec::new()),
            selected: RefCell::new(None),
            weak_self: RefCell::new(Weak::new()),
            backend,
        });
        *view.weak_self.borrow_mut() = Rc::downgrade(&view);

        let weak = Rc::downgrade(&view);
        view.apply_btn.connect_clicked(move |_| {
            if let Some(v) = weak.upgrade() {
                if let Some(kind) = v.selected.borrow().as_ref() {
                    v.backend.send(kind.action());
                }
            }
        });
        view
    }

    /// Switch thumbnails to the real monitor aspect once probed.
    pub fn set_aspect(&self, ratio: f32) {
        self.grid.set_aspect(ratio as f64);
        self.preview_bin.set_aspect(ratio as f64);
    }

    /// Show/hide the refill spinner (the timeline stays usable).
    pub fn set_refreshing(&self, active: bool) {
        self.spinner.set_visible(active);
        if active {
            self.spinner.start();
        } else {
            self.spinner.stop();
        }
    }

    /// The decoded picture for `path`, created once per use and reused.
    fn picture(&self, path: &std::path::Path, inspector: bool) -> Widget {
        let key = (path.to_path_buf(), inspector);
        if let Some(pic) = self.pictures.borrow().get(&key) {
            let pic = pic.clone();
            // A reused widget must not still belong to a previous card.
            if pic.parent().is_some() {
                pic.unparent();
            }
            return pic.upcast();
        }
        let widget = cover_picture(path);
        if let Ok(pic) = widget.clone().downcast::<Picture>() {
            self.pictures.borrow_mut().insert(key, pic);
        }
        widget
    }

    /// Grid cards use the cached reduced-resolution copy; only the
    /// inspector and actual wallpaper application use the original.
    fn grid_picture(&self, item: &Item) -> Widget {
        let path = if item.thumbnail_path.is_file() {
            &item.thumbnail_path
        } else {
            &item.path
        };
        self.picture(path, false)
    }

    fn show(&self, item: &Item) {
        let (_, inspector_badge, class, apply_label) = item.kind.labels();
        // Reloading the same image again on every progress update is
        // the expensive part; only swap the picture when it changes.
        if self.shown_path.borrow().as_ref() != Some(&item.path) {
            self.preview
                .set_child(Some(&self.picture(&item.path, true)));
            *self.shown_path.borrow_mut() = Some(item.path.clone());
        }
        for c in ["planned-card", "active-card", "history-card"] {
            self.preview.remove_css_class(c);
        }
        self.preview.add_css_class(class);
        set_badge(&self.badge_label, &inspector_badge, item.kind.badge_class());
        self.title
            .set_text(item.title.as_deref().unwrap_or("Untitled"));
        match &item.author {
            Some(a) => {
                self.author.set_text(&format!("by {a}"));
                self.author.set_visible(true);
            }
            None => self.author.set_visible(false),
        }
        self.source.set_text(&format!(
            "Source: {} [{}]",
            item.source.as_deref().unwrap_or(&item.provider),
            item.provider
        ));
        self.resolution.set_text(&match (item.width, item.height) {
            (Some(w), Some(h)) => format!("Resolution: {w}x{h}"),
            _ => "Resolution: Unknown".into(),
        });
        match apply_label {
            Some(label) => {
                self.apply_btn.set_sensitive(true);
                self.apply_btn.set_label(label);
            }
            None => {
                self.apply_btn.set_sensitive(false);
                self.apply_btn.set_label("Current wallpaper");
            }
        }
        *self.selected.borrow_mut() = Some(item.kind);
    }

    fn show_empty(&self) {
        self.preview
            .set_child(Some(&cover_picture(std::path::Path::new(""))));
        *self.shown_path.borrow_mut() = None;
        set_badge(&self.badge_label, "No wallpaper yet", "badge-history");
        self.title.set_text("No wallpaper selected");
        self.author.set_visible(false);
        self.source.set_text("Fetching planned wallpapers…");
        self.resolution.set_text("");
        self.apply_btn.set_sensitive(false);
        self.apply_btn.set_label("No wallpaper");
        *self.selected.borrow_mut() = None;
    }

    pub fn update(&self, state: &State) {
        let mut items: Vec<Item> = state
            .prefetch_queue
            .iter()
            .enumerate()
            .rev()
            .map(|(i, p)| Item {
                kind: Kind::Planned(i),
                ..Item::from(p)
            })
            .collect();
        let active_pos = state.history.get(state.current_history_index).map(|m| {
            let pos = items.len();
            items.push(Item {
                kind: Kind::Active(state.current_history_index),
                ..Item::from(m)
            });
            pos
        });
        for (i, m) in state.history.iter().enumerate() {
            if i != state.current_history_index {
                items.push(Item {
                    kind: Kind::Past(i),
                    ..Item::from(m)
                });
            }
        }

        match active_pos.and_then(|p| items.get(p)).or(items.first()) {
            Some(item) => self.show(item),
            None => self.show_empty(),
        }

        // The backend republishes the same state at the end of a refill;
        // rebuilding identical cards still costs layout and widget churn.
        if *self.rendered_items.borrow() == items {
            return;
        }
        *self.rendered_items.borrow_mut() = items.clone();
        // Do not retain decoded textures for wallpapers that have left
        // the timeline; otherwise the picture cache grows without bound.
        let live_paths: HashSet<&std::path::Path> = items
            .iter()
            .flat_map(|item| [item.path.as_path(), item.thumbnail_path.as_path()])
            .collect();
        self.pictures
            .borrow_mut()
            .retain(|(path, _), _| live_paths.contains(path.as_path()));

        self.grid.clear();
        for item in &items {
            let (card_badge, _, class, _) = item.kind.labels();
            let card = cover_overlay(
                self.grid_picture(item),
                class,
                Some((&card_badge, item.kind.badge_class())),
                Some(item.source.as_deref().unwrap_or(&item.provider)),
                &item.tooltip(),
            );
            let gesture = GestureClick::new();
            let weak = self.weak_self.borrow().clone();
            let clicked = item.clone();
            let backend = self.backend.clone();
            gesture.connect_pressed(move |_, n_press, _, _| {
                if n_press == 1 {
                    if let Some(v) = weak.upgrade() {
                        v.show(&clicked);
                    }
                } else if n_press >= 2 {
                    backend.send(clicked.kind.action());
                }
            });
            card.add_controller(gesture);
            self.grid.append(&card);
        }

        // Keep the current wallpaper in view when the grid is rebuilt.
        if let Some(pos) = active_pos {
            if let Some(child) = self.grid.child_at_index(pos as i32) {
                glib::idle_add_local_once(move || {
                    child.grab_focus();
                });
            }
        }
    }
}
