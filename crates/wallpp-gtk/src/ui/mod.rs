pub mod landing;
mod aspect_bin;
mod timeline_grid;

use gtk4::prelude::*;
use gtk4::{Application, ApplicationWindow, Box, HeaderBar, Label, Orientation, Spinner, Stack, StackSwitcher};
use std::rc::Rc;
use wallpp::state::State;

use crate::backend::{BackendHandle, UiUpdate};
use landing::LandingView;

pub struct MainWindow {
    pub window: ApplicationWindow,
    pub landing_view: Rc<LandingView>,
    #[allow(dead_code)]
    pub stack: Stack,
    status_spinner: Spinner,
    status_label: Label,
    summary_label: Label,
}

impl MainWindow {
    pub fn new(app: &Application, backend: Rc<BackendHandle>, initial_state: &State) -> Self {
        let window = ApplicationWindow::builder()
            .application(app)
            .title("wallpp")
            .default_width(860)
            .default_height(680)
            .build();

        let header = HeaderBar::new();
        let stack = Stack::new();
        stack.set_vexpand(true);
        stack.set_transition_type(gtk4::StackTransitionType::Crossfade);

        let switcher = StackSwitcher::new();
        switcher.set_stack(Some(&stack));
        header.set_title_widget(Some(&switcher));
        window.set_titlebar(Some(&header));

        let landing_view = LandingView::new(backend);
        landing_view.update(initial_state);

        stack.add_titled(&landing_view.container, Some("landing"), "Wallpapers");

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
            UiUpdate::StatusMessage(msg) => {
                self.set_status(&msg, true);
            }
            UiUpdate::Error(err) => {
                self.set_status(&format!("Error: {}", err), false);
            }
        }
    }
}
