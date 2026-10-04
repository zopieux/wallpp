use gtk4::{gdk, CssProvider, STYLE_PROVIDER_PRIORITY_APPLICATION};

pub fn init_css() {
    let provider = CssProvider::new();
    provider.load_from_data(
        "
        .timeline-card {
            border-radius: 8px;
            padding: 3px;
        }
        .future-card {
            background-color: alpha(#7852c0, 0.22);
            border: 2px solid #8a63d2;
        }
        .future-card:hover {
            background-color: alpha(#7852c0, 0.35);
            border-color: #9d75eb;
        }
        .active-card {
            background-color: alpha(#3584e4, 0.25);
            border: 3px solid #3584e4;
        }
        .active-card:hover {
            background-color: alpha(#3584e4, 0.38);
            border-color: #599cf0;
        }
        .history-card {
            background-color: alpha(currentColor, 0.05);
            border: 1px solid alpha(currentColor, 0.16);
        }
        .history-card:hover {
            background-color: alpha(currentColor, 0.12);
            border-color: alpha(currentColor, 0.3);
        }
        .badge-future {
            font-size: 0.75em;
            font-weight: bold;
            padding: 2px 6px;
            border-radius: 4px;
            background-color: #7852c0;
            color: white;
        }
        .badge-active {
            font-size: 0.75em;
            font-weight: bold;
            padding: 2px 6px;
            border-radius: 4px;
            background-color: #3584e4;
            color: white;
        }
        .badge-history {
            font-size: 0.75em;
            font-weight: bold;
            padding: 2px 6px;
            border-radius: 4px;
            background-color: alpha(#000000, 0.65);
            color: white;
        }
        .badge-source {
            font-size: 0.72em;
            font-weight: 600;
            padding: 2px 6px;
            border-radius: 4px;
            background-color: alpha(#000000, 0.75);
            color: #f0f0f0;
        }
        .card-title {
            font-weight: bold;
            font-size: 1.1em;
        }
        .card-sub {
            font-size: 0.85em;
            opacity: 0.75;
        }
        .status-bar {
            padding: 6px 14px;
            background-color: alpha(currentColor, 0.04);
            border-top: 1px solid alpha(currentColor, 0.12);
        }
        ",
    );
    if let Some(display) = gdk::Display::default() {
        gtk4::style_context_add_provider_for_display(
            &display,
            &provider,
            STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}
