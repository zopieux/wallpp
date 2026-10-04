mod aspect_bin;
pub mod general;
pub mod landing;
pub mod sources;
mod timeline_grid;

use gtk4::prelude::*;
use gtk4::{
    Application, ApplicationWindow, Box, HeaderBar, Label, Orientation, Spinner, Stack,
    StackSwitcher,
};
use std::cell::Cell;
use std::rc::Rc;
use wallpp::config::AppConfig;
use wallpp::state::State;

use crate::backend::{Action, BackendHandle, UiUpdate};
use general::GeneralView;
use landing::LandingView;
use sources::SourcesView;

pub struct MainWindow {
    pub window: ApplicationWindow,
    pub landing_view: Rc<LandingView>,
    #[allow(dead_code)]
    pub general_view: Rc<GeneralView>,
    pub sources_view: Rc<SourcesView>,
    #[allow(dead_code)]
    pub stack: Stack,
    status_spinner: Spinner,
    status_label: Label,
    summary_label: Label,
}

impl MainWindow {
    pub fn new(
        app: &Application,
        backend: Rc<BackendHandle>,
        initial_config: &AppConfig,
        initial_state: &State,
    ) -> Self {
        let window = ApplicationWindow::builder()
            .application(app)
            .title("wallpp")
            .default_width(920)
            .default_height(700)
            .build();

        let header = HeaderBar::new();
        let stack = Stack::new();
        stack.set_vexpand(true);
        stack.set_transition_type(gtk4::StackTransitionType::Crossfade);

        let switcher = StackSwitcher::new();
        switcher.set_stack(Some(&stack));
        header.set_title_widget(Some(&switcher));
        window.set_titlebar(Some(&header));

        let sources_dirty = Rc::new(Cell::new(false));

        let landing_view = LandingView::new(backend.clone());
        landing_view.update(initial_state);

        let general_view = GeneralView::new(backend.clone(), initial_config, sources_dirty.clone());
        let sources_view = SourcesView::new(backend.clone(), initial_config, sources_dirty.clone());

        stack.add_titled(&landing_view.container, Some("landing"), "Wallpapers");
        stack.add_titled(&general_view.container, Some("general"), "General");
        stack.add_titled(&sources_view.container, Some("sources"), "Sources");

        let dirty_clone = sources_dirty.clone();
        let backend_clone = backend.clone();
        stack.connect_visible_child_notify(move |stk| {
            if stk.visible_child_name().as_deref() == Some("landing") && dirty_clone.get() {
                dirty_clone.set(false);
                backend_clone.send(Action::RefillQueueAndRefresh);
            }
        });

        let status_bar = Box::new(Orientation::Horizontal, 8);
        status_bar.add_css_class("status-bar");

        let status_spinner = Spinner::new();
        let status_label = Label::new(Some("Ready"));
        status_label.set_halign(gtk4::Align::Start);
        status_label.set_hexpand(true);

        let summary_label = Label::new(None);
        summary_label.add_css_class("card-sub");
        summary_label.set_halign(gtk4::Align::End);

        let initial_summary = format!(
            "{} planned • 1 active • {} in history",
            initial_state.prefetch_queue.len(),
            initial_state.history.len()
        );
        summary_label.set_text(&initial_summary);

        status_bar.append(&status_spinner);
        status_bar.append(&status_label);
        status_bar.append(&summary_label);

        let root_box = Box::new(Orientation::Vertical, 0);
        root_box.append(&stack);
        root_box.append(&status_bar);

        window.set_child(Some(&root_box));

        Self {
            window,
            landing_view,
            general_view,
            sources_view,
            stack,
            status_spinner,
            status_label,
            summary_label,
        }
    }

    pub fn set_status(&self, text: &str, loading: bool) {
        self.status_label.set_text(text);
        if loading {
            self.status_spinner.start();
        } else {
            self.status_spinner.stop();
        }
    }

    pub fn handle_update(&self, update: UiUpdate) {
        match update {
            UiUpdate::StateUpdated { state, status } => {
                self.landing_view.update(&state);
                self.set_status(&status, false);
                let summary = format!(
                    "{} planned • 1 active • {} in history",
                    state.prefetch_queue.len(),
                    state.history.len()
                );
                self.summary_label.set_text(&summary);
            }
            UiUpdate::ProvidersDiscovered(provs) => {
                self.sources_view.set_providers(provs);
            }
            UiUpdate::SourceConfigNormalized { normalized } => {
                self.sources_view.handle_normalized_config(&normalized);
            }
            UiUpdate::SourcePreviews { items, .. } => {
                self.sources_view.handle_previews(&items);
            }
            UiUpdate::SourcePreviewError(err) => {
                self.sources_view.handle_preview_error(&err);
                self.set_status(&format!("Preview error: {}", err), false);
            }
            UiUpdate::StatusMessage(msg) => {
                self.set_status(&msg, true);
            }
            UiUpdate::Error(err) => {
                self.set_status(&format!("Error: {}", err), false);
            }
        }
    }
}
