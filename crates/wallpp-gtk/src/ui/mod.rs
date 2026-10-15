pub mod aspect_bin;
pub mod general;
pub mod landing;
pub mod sources;
pub mod timeline_grid;
mod widgets;

use gtk4::prelude::*;
use gtk4::{Application, ApplicationWindow, HeaderBar, Stack, StackSwitcher};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use wallpp::config::AppConfig;
use wallpp::state::State;

use crate::backend::{BackendHandle, UiUpdate};
use general::GeneralView;
use landing::LandingView;
use sources::SourcesView;
pub use widgets::screen_aspect;

pub struct MainWindow {
    pub window: ApplicationWindow,
    landing: Rc<LandingView>,
    sources: Rc<SourcesView>,
}

impl MainWindow {
    /// Apply the probed monitor aspect to all thumbnail views.
    pub fn set_aspect(&self, ratio: f32) {
        self.landing.set_aspect(ratio);
        self.sources.set_aspect(ratio);
    }
}

impl MainWindow {
    pub fn new(
        app: &Application,
        backend: BackendHandle,
        initial_config: &AppConfig,
        initial_state: &State,
    ) -> Self {
        // One shared config owned by the window: both form pages mutate
        // it and hand full clones to the backend, so edits on one page
        // can never silently overwrite the other page's changes.
        let cfg = Rc::new(RefCell::new(initial_config.clone()));
        let dirty = Rc::new(Cell::new(false));

        let landing = LandingView::new(backend.clone());
        landing.update(initial_state);
        let general = GeneralView::new(backend.clone(), cfg.clone(), dirty.clone());
        let sources = SourcesView::new(backend.clone(), cfg, dirty.clone());

        let stack = Stack::new();
        stack.set_vexpand(true);
        stack.set_transition_type(gtk4::StackTransitionType::Crossfade);
        stack.add_titled(&landing.container, Some("landing"), "Wallpapers");
        stack.add_titled(&general.container, Some("general"), "General");
        stack.add_titled(&sources.container, Some("sources"), "Sources");

        // Returning to the wallpapers page after settings changed
        // rebuilds the planned queue.
        stack.connect_visible_child_notify(move |stk| {
            if stk.visible_child_name().as_deref() == Some("landing") && dirty.get() {
                dirty.set(false);
                backend.send(crate::backend::Action::RefreshQueue);
            }
        });

        let switcher = StackSwitcher::new();
        switcher.set_stack(Some(&stack));
        let header = HeaderBar::new();
        header.set_title_widget(Some(&switcher));

        let window = ApplicationWindow::builder()
            .application(app)
            .title("wallpp")
            .default_width(920)
            .default_height(700)
            .titlebar(&header)
            .child(&stack)
            .build();

        Self {
            window,
            landing,
            sources,
        }
    }

    pub fn handle_update(&self, update: UiUpdate) {
        match update {
            UiUpdate::State(state) => self.landing.update(&state),
            UiUpdate::Refreshing(active) => self.landing.set_refreshing(active),
            UiUpdate::Providers(provs) => self.sources.set_providers(provs),
            UiUpdate::Normalized(entries) => self.sources.handle_normalized(&entries),
            UiUpdate::Previews(result) => self.sources.handle_previews(result),
        }
    }
}
