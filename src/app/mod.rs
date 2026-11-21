//! Interactive terminal application.
//!
//! [`run`] owns the terminal lifecycle, the Tokio event loop and the background
//! download manager. All visual state lives in [`state::App`]; the UI modules
//! render it without mutating it, and [`events`] translates keystrokes into
//! state transitions.

pub mod events;
pub mod state;

#[cfg(test)]
pub(crate) mod test_support;

pub use state::{App, Screen};

use std::io::{self, IsTerminal, Stdout};
use std::time::Duration;

use anyhow::{Context, Result};
use crossterm::cursor::{Hide, Show};
use crossterm::event::EventStream;
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use futures_util::StreamExt;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use crate::catalog::Catalog;
use crate::config::{AppPaths, Config};
use crate::download::{spawn, ManagerConfig};

/// Launch the interactive browser.
///
/// Fails with a helpful message when standard input or output is not a
/// terminal, so `pu-get < file` and CI logs never emit escape sequences.
pub async fn run(paths: AppPaths, config: Config, color: bool) -> Result<()> {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        anyhow::bail!(
            "the interactive browser needs a terminal; use `pu-get list` or \
             `pu-get get` for scripted use"
        );
    }

    let catalog = Catalog::load(&paths).context("could not load the catalogs")?;
    let manager_config = ManagerConfig {
        concurrency: config.concurrency,
        retries: config.retries,
        request_timeout: Duration::from_secs(config.request_timeout_secs),
    };
    let (downloads, mut download_events) =
        spawn(manager_config).context("could not start the download manager")?;

    let mut app = App::new(catalog, config, paths, color);
    app.attach_downloads(downloads.clone());

    let mut guard = TerminalGuard::enter().context("could not initialise the terminal")?;
    install_panic_hook();

    let mut terminal_events = EventStream::new();
    let mut ticker = tokio::time::interval(Duration::from_millis(120));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    let result: Result<()> = loop {
        if let Err(error) = guard.terminal.draw(|frame| crate::ui::draw(frame, &app)) {
            break Err(error).context("could not draw the terminal");
        }

        tokio::select! {
            maybe_event = terminal_events.next() => {
                match maybe_event {
                    Some(Ok(event)) => events::handle_event(&mut app, event),
                    Some(Err(error)) => app.set_error(format!("terminal input error: {error}")),
                    None => break Ok(()),
                }
            }
            Some(event) = download_events.recv() => {
                app.handle_download_event(event);
                while let Ok(event) = download_events.try_recv() {
                    app.handle_download_event(event);
                }
            }
            _ = ticker.tick() => app.on_tick(),
        }

        if app.should_quit {
            break Ok(());
        }
    };

    let _ = downloads.shutdown();
    drop(guard);
    result
}

/// Terminal setup that is always undone, including on early returns.
struct TerminalGuard {
    terminal: Terminal<CrosstermBackend<Stdout>>,
}

impl TerminalGuard {
    fn enter() -> io::Result<Self> {
        enable_raw_mode()?;

        let mut stdout = io::stdout();
        if let Err(error) = execute!(stdout, EnterAlternateScreen, Hide) {
            let _ = disable_raw_mode();
            return Err(error);
        }

        match Terminal::new(CrosstermBackend::new(stdout)) {
            Ok(terminal) => Ok(Self { terminal }),
            Err(error) => {
                restore_screen();
                Err(error)
            }
        }
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        restore_screen();
        let _ = self.terminal.show_cursor();
    }
}

/// Restore the terminal after normal exits and panics.
fn restore_screen() {
    let _ = disable_raw_mode();
    let _ = execute!(io::stdout(), LeaveAlternateScreen, Show);
}

fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore_screen();
        previous(info);
    }));
}
