//! Server-side terminal state machine.
//!
//! Every pane feeds its raw PTY output through an `alacritty_terminal::Term`
//! so the engine can answer `read_screen` headlessly — without a round-trip
//! to the webview and regardless of whether the UI is subscribed. The crate
//! is isolated behind this module so it can be swapped (e.g. for `vt100`)
//! without touching the rest of the engine.
//!
//! It also tracks the kitty keyboard protocol (push/pop/query), which
//! xterm.js does not support. Query responses surface as `Event::PtyWrite`
//! and must be written back to the PTY — apps like Claude Code only enable
//! shift+arrow selection etc. once the terminal answers `CSI ? u`.

use std::sync::Arc;

use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{test::TermSize, Config, Term, TermMode};
use alacritty_terminal::vte::ansi::Processor;
use parking_lot::Mutex;

/// Collects terminal-generated replies (e.g. kitty keyboard mode reports)
/// produced while parsing; the pane's read loop drains and writes them back.
#[derive(Clone, Default)]
pub struct EventProxy {
    replies: Arc<Mutex<Vec<String>>>,
}

impl EventListener for EventProxy {
    fn send_event(&self, event: Event) {
        if let Event::PtyWrite(text) = event {
            // Only forward kitty keyboard-mode reports (CSI ? <flags> u).
            // Other reports (DA, DSR, ...) are already answered by xterm.js
            // in the webview; forwarding them too would double-reply.
            if text.starts_with("\x1b[?") && text.ends_with('u') {
                self.replies.lock().push(text);
            }
        }
    }
}

pub struct TermState {
    term: Mutex<Term<EventProxy>>,
    parser: Mutex<Processor>,
    proxy: EventProxy,
}

impl TermState {
    pub fn new(cols: u16, rows: u16) -> Self {
        let config = Config {
            scrolling_history: 1_000,
            kitty_keyboard: true,
            ..Config::default()
        };
        let size = TermSize::new(cols.max(2) as usize, rows.max(2) as usize);
        let proxy = EventProxy::default();
        Self {
            term: Mutex::new(Term::new(config, &size, proxy.clone())),
            parser: Mutex::new(Processor::new()),
            proxy,
        }
    }

    /// Feed a chunk of PTY output. Returns terminal replies that must be
    /// written back to the PTY (kitty keyboard mode reports).
    pub fn advance(&self, bytes: &[u8]) -> Vec<String> {
        {
            let mut term = self.term.lock();
            self.parser.lock().advance(&mut *term, bytes);
        }
        std::mem::take(&mut *self.proxy.replies.lock())
    }

    pub fn resize(&self, cols: u16, rows: u16) {
        self.term
            .lock()
            .resize(TermSize::new(cols.max(2) as usize, rows.max(2) as usize));
    }

    /// Is the kitty keyboard protocol's "disambiguate escape codes" mode on?
    /// The frontend switches key encodings (Esc, modified Enter) on this.
    pub fn kitty_keyboard_active(&self) -> bool {
        self.term
            .lock()
            .mode()
            .contains(TermMode::DISAMBIGUATE_ESC_CODES)
    }

    /// Pop every kitty keyboard mode entry. Used when an app exits without
    /// cleaning up (e.g. killed by Ctrl+C) and the shell is foreground again
    /// — otherwise the stale mode breaks Esc/Enter encodings for the shell.
    pub fn reset_kitty_keyboard(&self) {
        // CSI < 255 u pops up to 255 stack entries — public parse path,
        // no private API needed.
        let _ = self.advance(b"\x1b[<255u");
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kitty_query_gets_answered() {
        let ts = TermState::new(80, 24);
        let replies = ts.advance(b"\x1b[?u");
        assert_eq!(replies, vec!["\x1b[?0u".to_string()], "query must be answered");
        // Push disambiguate mode, then the flag should be visible.
        let _ = ts.advance(b"\x1b[>1u");
        assert!(ts.kitty_keyboard_active(), "disambiguate flag should be on");
        let replies = ts.advance(b"\x1b[?u");
        assert_eq!(replies, vec!["\x1b[?1u".to_string()]);
    }
}
