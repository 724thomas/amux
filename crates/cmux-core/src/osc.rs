//! Notification-bearing escape sequence detection.
//!
//! A second incremental `vte::Parser` runs over the same PTY stream as the
//! terminal state machine. Incremental parsing is the only correct way to
//! catch sequences split across read chunks — never string-match on chunks.
//!
//! Detected: bare BEL, OSC 9 (`9;message` — iTerm2/ConEmu style), OSC 777
//! (`777;notify;title;body` — urxvt style), OSC 99 (kitty style, basic).

use alacritty_terminal::vte;

#[derive(Debug, Clone, PartialEq)]
pub enum OscEvent {
    Bell,
    Notify { title: Option<String>, body: Option<String> },
}

#[derive(Default)]
struct Performer {
    events: Vec<OscEvent>,
}

impl vte::Perform for Performer {
    fn execute(&mut self, byte: u8) {
        if byte == 0x07 {
            self.events.push(OscEvent::Bell);
        }
    }

    fn osc_dispatch(&mut self, params: &[&[u8]], _bell_terminated: bool) {
        let text = |idx: usize| -> Option<String> {
            params
                .get(idx)
                .map(|p| String::from_utf8_lossy(p).into_owned())
                .filter(|s| !s.is_empty())
        };
        match params.first().map(|p| *p) {
            Some(b"9") => self.events.push(OscEvent::Notify { title: None, body: text(1) }),
            Some(b"777") if params.get(1).is_some_and(|k| *k == b"notify") => {
                self.events.push(OscEvent::Notify { title: text(2), body: text(3) });
            }
            // kitty: OSC 99 ; metadata ; payload
            Some(b"99") => self.events.push(OscEvent::Notify { title: None, body: text(2) }),
            _ => {}
        }
    }
}

pub struct OscDetector {
    parser: vte::Parser,
    performer: Performer,
}

impl Default for OscDetector {
    fn default() -> Self {
        Self { parser: vte::Parser::new(), performer: Performer::default() }
    }
}

impl OscDetector {
    /// Feed a chunk; returns any notification events it completed.
    pub fn advance(&mut self, bytes: &[u8]) -> Vec<OscEvent> {
        self.parser.advance(&mut self.performer, bytes);
        std::mem::take(&mut self.performer.events)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_bel_and_osc() {
        let mut det = OscDetector::default();
        assert_eq!(det.advance(b"hello\x07"), vec![OscEvent::Bell]);
        assert_eq!(
            det.advance(b"\x1b]9;build done\x07"),
            vec![OscEvent::Notify { title: None, body: Some("build done".into()) }]
        );
        assert_eq!(
            det.advance(b"\x1b]777;notify;Title;Body\x1b\\"),
            vec![OscEvent::Notify { title: Some("Title".into()), body: Some("Body".into()) }]
        );
    }

    #[test]
    fn handles_chunk_split_sequences() {
        let mut det = OscDetector::default();
        // Sequence split across three reads.
        assert!(det.advance(b"\x1b]9;par").is_empty());
        assert!(det.advance(b"tial mess").is_empty());
        assert_eq!(
            det.advance(b"age\x07"),
            vec![OscEvent::Notify { title: None, body: Some("partial message".into()) }]
        );
    }

    #[test]
    fn osc_terminator_bel_is_not_a_bell() {
        let mut det = OscDetector::default();
        let events = det.advance(b"\x1b]9;msg\x07");
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], OscEvent::Notify { .. }));
    }
}
