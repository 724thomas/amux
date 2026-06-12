//! Server-side terminal state machine.
//!
//! Every pane feeds its raw PTY output through an `alacritty_terminal::Term`
//! so the engine can answer `read_screen` headlessly — without a round-trip
//! to the webview and regardless of whether the UI is subscribed. The crate
//! is isolated behind this module so it can be swapped (e.g. for `vt100`)
//! without touching the rest of the engine.

use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{test::TermSize, Config, Term};
use alacritty_terminal::vte::ansi::Processor;
use parking_lot::Mutex;

/// Receives `alacritty_terminal` events. Bell wiring lands in M4.
#[derive(Clone)]
pub struct EventProxy;

impl EventListener for EventProxy {
    fn send_event(&self, _event: Event) {}
}

pub struct TermState {
    term: Mutex<Term<EventProxy>>,
    parser: Mutex<Processor>,
}

impl TermState {
    pub fn new(cols: u16, rows: u16) -> Self {
        let config = Config { scrolling_history: 1_000, ..Config::default() };
        let size = TermSize::new(cols.max(2) as usize, rows.max(2) as usize);
        Self {
            term: Mutex::new(Term::new(config, &size, EventProxy)),
            parser: Mutex::new(Processor::new()),
        }
    }

    pub fn advance(&self, bytes: &[u8]) {
        let mut term = self.term.lock();
        self.parser.lock().advance(&mut *term, bytes);
    }

    pub fn resize(&self, cols: u16, rows: u16) {
        self.term
            .lock()
            .resize(TermSize::new(cols.max(2) as usize, rows.max(2) as usize));
    }

    /// The visible screen as plain text, one line per row, trailing blanks
    /// trimmed. Wide-char spacer cells are skipped so CJK text round-trips.
    pub fn read_screen(&self) -> String {
        let term = self.term.lock();
        let grid = term.grid();
        let mut out = String::new();
        for l in 0..grid.screen_lines() {
            let line = Line(l as i32);
            let mut row = String::new();
            for c in 0..grid.columns() {
                let cell = &grid[line][Column(c)];
                if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                    continue;
                }
                row.push(cell.c);
            }
            let trimmed = row.trim_end();
            out.push_str(trimmed);
            out.push('\n');
        }
        // Collapse the blank tail below the last used row.
        let trimmed_len = out.trim_end_matches('\n').len();
        out.truncate(trimmed_len);
        out.push('\n');
        out
    }
}
