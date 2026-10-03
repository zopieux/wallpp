pub mod app;
pub mod field;
pub mod preview;
pub mod views;

use anyhow::{Context, Result};
use crossterm::event::{
    DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
    EventStream,
};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use futures::StreamExt;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use std::io::stdout;
use std::sync::Arc;

use crate::cache::CacheManager;
use crate::config::AppConfig;
use crate::provider::ProviderManager;
use crate::state::State;
use app::App;
use preview::PreviewTaskResult;

struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(
            stdout(),
            crossterm::style::Print("\x1b_Ga=d,d=A\x1b\\"),
            LeaveAlternateScreen,
            DisableMouseCapture,
            DisableBracketedPaste
        );
    }
}

/// Runs the interactive TUI configuration editor.
pub async fn run_tui(
    cache_mgr: &CacheManager,
    provider_mgr: Arc<ProviderManager>,
    state: &mut State,
) -> Result<()> {
    // Enter alternate screen, enable raw mode and bracketed paste.
    enable_raw_mode().context("Failed to enable crossterm raw mode")?;
    let mut stdout = stdout();
    execute!(
        stdout,
        EnterAlternateScreen,
        EnableMouseCapture,
        EnableBracketedPaste
    )
    .context("Failed to enter alternate screen")?;
    let _guard = TerminalGuard;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).context("Failed to initialize terminal")?;
    terminal.clear()?;

    // Load file-only config (ignoring transient WALLPP_* env vars).
    let loaded_config = AppConfig::load_file_only()?;

    // Channel for async preview image results.
    let (preview_tx, mut preview_rx) = tokio::sync::mpsc::unbounded_channel::<PreviewTaskResult>();

    let mut app = App::new(loaded_config, Arc::clone(&provider_mgr), preview_tx);
    let mut event_stream = EventStream::new();

    loop {
        if app.needs_clear {
            app.needs_clear = false;
            let _ = execute!(
                std::io::stdout(),
                crossterm::style::Print("\x1b_Ga=d,d=A\x1b\\")
            );
            terminal.clear()?;
        }

        terminal.draw(|f| {
            app.render(f.area(), f.buffer_mut(), state);
        })?;

        if app.should_quit {
            break;
        }

        tokio::select! {
            event_opt = event_stream.next() => {
                match event_opt {
                    Some(Ok(crossterm::event::Event::Key(key))) => {
                        if key.kind == crossterm::event::KeyEventKind::Press {
                            app.handle_key(key, cache_mgr, state);
                        }
                    }
                    Some(Ok(crossterm::event::Event::Paste(pasted))) => {
                        app.handle_paste(&pasted);
                    }
                    _ => {}
                }
            }
            Some(result) = preview_rx.recv() => {
                app.handle_preview_result(result);
            }
        }
    }

    Ok(())
}
