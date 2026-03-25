use std::io;
use std::sync::atomic::{AtomicBool, Ordering};

use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};

pub type TuiTerminal = Terminal<CrosstermBackend<io::Stdout>>;

static TERMINAL_RESTORED: AtomicBool = AtomicBool::new(false);

pub fn init() -> io::Result<TuiTerminal> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    Terminal::new(backend)
}

pub fn restore() -> io::Result<()> {
    if TERMINAL_RESTORED.swap(true, Ordering::SeqCst) {
        return Ok(());
    }
    disable_raw_mode()?;
    execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture)?;
    Ok(())
}

pub struct PanicGuard;

impl PanicGuard {
    pub fn new() -> Self {
        let prev_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if !TERMINAL_RESTORED.swap(true, Ordering::SeqCst) {
                let _ = disable_raw_mode();
                let _ = execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture);
            }
            prev_hook(info);
        }));
        Self
    }
}

impl Drop for PanicGuard {
    fn drop(&mut self) {
        let _ = restore();
    }
}

impl Default for PanicGuard {
    fn default() -> Self {
        Self::new()
    }
}
