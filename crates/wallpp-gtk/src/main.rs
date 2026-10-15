use gtk4::prelude::*;
use gtk4::Application;
use std::rc::Rc;
use wallpp::config::AppConfig;
use wallpp::state::State;

mod backend;
mod style;
mod ui;

use ui::MainWindow;

const APP_ID: &str = "org.wallpp.WallppGtk";

fn main() {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("Failed to initialize Tokio runtime");
    let _guard = rt.enter();

    let app = Application::builder().application_id(APP_ID).build();
    app.connect_startup(|_| style::init_css());
    app.connect_activate(build_ui);
    app.run();
}

fn build_ui(app: &Application) {
    // Two small local file reads; everything expensive (provider
    // discovery, network, downloads, saves) runs in the backend task.
    let app_cfg = AppConfig::load_file_only().unwrap_or_default();
    let state = State::load_or_default();

    let (ui_tx, ui_rx) = async_channel::unbounded();
    let backend = backend::start_backend(app_cfg.clone(), state.clone(), ui_tx);
    let main_window = Rc::new(MainWindow::new(app, backend, &app_cfg, &state));
    let win = main_window.clone();
    glib::spawn_future_local(async move {
        while let Ok(update) = ui_rx.recv().await {
            win.handle_update(update);
        }
    });

    // Thumbnails start at 16:9 and switch to the real monitor aspect
    // (so cover previews match how wallpapers will be applied) as soon
    // as the probe finishes off the GTK thread.
    let win = main_window.clone();
    glib::spawn_future_local(async move {
        if let Ok(ratio) = gtk4::gio::spawn_blocking(ui::screen_aspect).await {
            win.set_aspect(ratio);
        }
    });

    main_window.window.present();
}
