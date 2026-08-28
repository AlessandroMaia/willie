//! What a late client needs to see: the tail of the output, and whether
//! the program is on the alternate screen (where a replay only flashes
//! stale bytes). Pure; the socket code owns the locking.

use std::collections::VecDeque;

/// The tail of the session's output, replayed to a new terminal.
#[derive(Debug)]
pub struct Ring {
    buf: VecDeque<u8>,
    cap: usize,
}

impl Ring {
    #[must_use]
    pub fn new(cap: usize) -> Self {
        Self {
            buf: VecDeque::new(),
            cap,
        }
    }

    pub fn push(&mut self, bytes: &[u8]) {
        self.buf.extend(bytes.iter().copied());
        let excess = self.buf.len().saturating_sub(self.cap);
        if excess > 0 {
            self.buf.drain(..excess);
        }
    }

    #[must_use]
    pub fn snapshot(&self) -> Vec<u8> {
        self.buf.iter().copied().collect()
    }
}

/// Tracks `ESC [ ? 1049|1047|47 h|l`, the switches to and from the
/// alternate screen. A switch may straddle two reads, so the last few
/// bytes of the previous chunk are kept.
#[derive(Debug, Default)]
pub struct AltScreen {
    active: bool,
    tail: Vec<u8>,
}

/// Longest switch sequence: `ESC [ ? 1 0 4 9 h`.
const MAX_SEQ: usize = 8;

impl AltScreen {
    #[must_use]
    pub fn active(&self) -> bool {
        self.active
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        let mut buf = std::mem::take(&mut self.tail);
        buf.extend_from_slice(bytes);
        let mut i = 0;
        while i < buf.len() {
            if let Some((len, on)) = switch_at(&buf[i..]) {
                self.active = on;
                i += len;
            } else {
                i += 1;
            }
        }
        let keep = buf.len().min(MAX_SEQ - 1);
        self.tail = buf[buf.len() - keep..].to_vec();
    }
}

/// If `bytes` starts with a switch sequence, its length and whether it
/// turns the alternate screen on.
fn switch_at(bytes: &[u8]) -> Option<(usize, bool)> {
    for code in [&b"1049"[..], &b"1047"[..], &b"47"[..]] {
        let len = 3 + code.len() + 1;
        if bytes.len() >= len
            && bytes[0] == 0x1b
            && bytes[1] == b'['
            && bytes[2] == b'?'
            && &bytes[3..3 + code.len()] == code
        {
            match bytes[len - 1] {
                b'h' => return Some((len, true)),
                b'l' => return Some((len, false)),
                _ => {}
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ring_keeps_only_the_last_bytes() {
        let mut ring = Ring::new(4);
        ring.push(b"abcdef");
        assert_eq!(ring.snapshot(), b"cdef");
        ring.push(b"gh");
        assert_eq!(ring.snapshot(), b"efgh");
    }

    #[test]
    fn the_alternate_screen_is_tracked_across_chunk_boundaries() {
        let mut alt = AltScreen::default();
        alt.feed(b"hello \x1b[?10");
        assert!(!alt.active());
        alt.feed(b"49h drawing");
        assert!(alt.active());
        alt.feed(b"\x1b[?1049l bye");
        assert!(!alt.active());
        alt.feed(b"\x1b[?47h");
        assert!(alt.active());
        alt.feed(b"\x1b[?1047l");
        assert!(!alt.active());
    }

    #[test]
    fn the_last_switch_in_a_chunk_wins() {
        let mut alt = AltScreen::default();
        alt.feed(b"\x1b[?1049h\x1b[?1049l");
        assert!(!alt.active());
        alt.feed(b"\x1b[?1049l\x1b[?1049h");
        assert!(alt.active());
    }
}
