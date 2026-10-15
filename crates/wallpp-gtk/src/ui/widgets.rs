//! Small shared GTK helpers: fixed-aspect cover thumbnails and dynamic
//! flow grids, built from stock widgets (no custom GObject subclasses).

use gtk4::prelude::*;
use gtk4::{Box, ContentFit, Label, Orientation, Overlay, Picture, Widget};
use std::path::Path;

/// Aspect ratio (width / height) of the biggest connected monitor, so
/// cover thumbnails preview wallpapers exactly as they will be applied
/// to the screen. Falls back to 16:9 when no monitor is detected.
pub fn screen_aspect() -> f32 {
    match wallpp::monitor::get_biggest_monitor() {
        Some(m) if m.width > 0 && m.height > 0 => m.width as f32 / m.height as f32,
        _ => 16.0 / 9.0,
    }
}

pub fn cover_picture(path: &Path) -> Widget {
    if path.exists() {
        let pic = Picture::for_filename(path);
        pic.set_can_shrink(true);
        pic.set_content_fit(ContentFit::Cover);
        pic.set_hexpand(true);
        pic.set_vexpand(true);
        pic.upcast()
    } else {
        let label = Label::new(Some("No image"));
        label.set_hexpand(true);
        label.set_vexpand(true);
        label.upcast()
    }
}

/// Build a box with children in one expression.
#[macro_export]
macro_rules! stack {
    ($orientation:expr, $spacing:expr, $($child:expr),+ $(,)?) => {{
        let bx = gtk4::Box::new($orientation, $spacing);
        $(bx.append(&$child);)+
        bx
    }};
}

/// Remove all children through the container's own `remove`, which
/// containers with internal child bookkeeping (FlowBox, ListBox)
/// require — raw `unparent()` leaves their child lists stale and
/// corrupts later insertions on some GTK versions.
#[macro_export]
macro_rules! clear {
    ($container:expr) => {
        while let Some(child) = $container.first_child() {
            $container.remove(&child);
        }
    };
}

pub fn badge(text: &str, css_class: &str, halign: gtk4::Align, valign: gtk4::Align) -> Label {
    let label = Label::new(Some(text));
    label.add_css_class(css_class);
    label.set_halign(halign);
    label.set_valign(valign);
    label.set_margin_top(6);
    label.set_margin_bottom(6);
    label.set_margin_start(6);
    label.set_margin_end(6);
    label
}

/// One cover-thumbnail card: image filling its cell, an optional corner
/// badge and an optional bottom-left source label.
pub fn cover_card(
    path: &Path,
    css_class: &str,
    top_badge: Option<(&str, &str)>,
    source: Option<&str>,
    tooltip: &str,
) -> Overlay {
    cover_overlay(cover_picture(path), css_class, top_badge, source, tooltip)
}

/// Same card as `cover_card`, but around a caller-supplied (possibly
/// cached and reused) image widget.
pub fn cover_overlay(
    child: Widget,
    css_class: &str,
    top_badge: Option<(&str, &str)>,
    source: Option<&str>,
    tooltip: &str,
) -> Overlay {
    let overlay = Overlay::new();
    overlay.add_css_class("timeline-card");
    overlay.add_css_class(css_class);
    overlay.set_overflow(gtk4::Overflow::Hidden);
    overlay.set_child(Some(&child));
    overlay.set_tooltip_text(Some(tooltip));

    if let Some((text, class)) = top_badge {
        overlay.add_overlay(&badge(text, class, gtk4::Align::End, gtk4::Align::Start));
    }
    if let Some(src) = source {
        let src_badge = badge(src, "badge-source", gtk4::Align::Start, gtk4::Align::End);
        src_badge.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        overlay.add_overlay(&src_badge);
    }
    overlay
}

/// Vertical page padding wrapper used by the form pages.
pub fn page_box(spacing: i32) -> Box {
    let bx = Box::new(Orientation::Vertical, spacing);
    bx.set_margin_start(20);
    bx.set_margin_end(20);
    bx.set_margin_top(16);
    bx.set_margin_bottom(24);
    bx
}

pub fn title_label(text: &str) -> Label {
    let label = Label::new(Some(text));
    label.add_css_class("card-title");
    label.set_halign(gtk4::Align::Start);
    label
}

pub fn sub_label(text: &str) -> Label {
    let label = Label::new(Some(text));
    label.add_css_class("card-sub");
    label.set_halign(gtk4::Align::Start);
    label
}
