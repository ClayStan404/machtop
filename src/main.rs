mod app;
mod metrics;
mod ui;

use std::io;
use std::io::IsTerminal;
use std::time::Duration;

use anyhow::Result;
use app::App;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

struct TerminalGuard {
    raw_mode: bool,
    alternate_screen: bool,
}

impl TerminalGuard {
    fn enter() -> Result<(Self, Terminal<CrosstermBackend<io::Stdout>>)> {
        let mut guard = Self {
            raw_mode: false,
            alternate_screen: false,
        };

        enable_raw_mode()?;
        guard.raw_mode = true;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen)?;
        guard.alternate_screen = true;
        let terminal = Terminal::new(CrosstermBackend::new(stdout))?;
        Ok((guard, terminal))
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        if self.alternate_screen {
            let mut stdout = io::stdout();
            let _ = execute!(stdout, LeaveAlternateScreen);
        }
        if self.raw_mode {
            let _ = disable_raw_mode();
        }
    }
}

fn main() -> Result<()> {
    if !io::stdout().is_terminal() {
        eprintln!("machtop requires an interactive terminal.");
        std::process::exit(1);
    }

    let (guard, mut terminal) = TerminalGuard::enter()?;
    let mut app = App::new(Duration::from_secs(1))?;

    loop {
        terminal.draw(|frame| ui::render(frame, &app))?;

        if event::poll(app.poll_timeout())?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            match key.code {
                KeyCode::Char('q') => break,
                KeyCode::Char('c')
                    if key
                        .modifiers
                        .contains(crossterm::event::KeyModifiers::CONTROL) =>
                {
                    break;
                }
                _ => {}
            }
        }

        app.tick_if_needed()?;
    }

    drop(guard);
    Ok(())
}
