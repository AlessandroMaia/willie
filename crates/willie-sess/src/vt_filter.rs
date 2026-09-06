//! The terminal-output filter: a byte automaton on the harness's output,
//! between the PTY and the attach fan-out.
//!
//! The output is a control language. An escape sequence can write the
//! host clipboard (OSC 52), set the window title (OSC 0/1/2, `CSI > t`),
//! or ask the emulator a question whose reply carries attacker-controlled
//! text back into the input stream (DECRQSS, XTGETTCAP, the title
//! reports). The kernel sandbox cannot help: this channel is the
//! session's whole purpose. So a filter drops the acting and text-echoing
//! sequences and passes everything that draws — including the queries
//! whose reply is a fixed form the harness depends on (DA, DSR, the size
//! reports, background colour).
//!
//! Pure and portable: no I/O, no libc, tested on any host. It owns state
//! across calls, because a sequence can arrive split between reads; it
//! never forwards a half-sequence, and it buffers only the small window
//! it needs to identify a sequence's type — a 100 KiB hyperlink or inline
//! image streams through with the buffer under the cap. A filter does not
//! fail: it decides drop or pass as the emulator would tokenise the same
//! bytes — a control spliced into a parameter, a leading zero, a number
//! padded past the cap resolve here the way they resolve there — and when
//! a sequence is too long to identify and still reproduce, it is dropped
//! closed rather than passed as an op the emulator would re-form. Drawing
//! bytes are never dropped.

/// Bytes buffered while identifying an OSC or DCS type — the number
/// before the first `;`, or the DCS's two intro bytes. Past this the type
/// is treated as unidentified and the sequence streams through.
const OSC_ID_CAP: usize = 16;
/// Bytes buffered for a CSI before its final byte. Past this the CSI is
/// dropped closed at its final byte: it cannot be reproduced from the
/// frozen buffer, and no real drawing CSI is this long. Set clear of the
/// longest real sequence — a truecolor SGR combining a 24-bit foreground,
/// background and underline colour is on the order of fifty bytes.
const CSI_CAP: usize = 256;

/// Where the automaton is between bytes. `buf` holds the introducer and
/// the bytes accumulated so far for a sequence being identified; the
/// string states (`Str`/`StrEsc`/`StrC2`) hold no bytes and stream
/// directly, remembering only whether they drop and whether BEL ends the
/// string.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum State {
    /// Drawing bytes; each is forwarded at once. `ESC` and `C2` open a
    /// lookahead; everything else (text, C0 controls, raw `0x80`–`0x9F`
    /// continuation bytes, other UTF-8 lead and trailing bytes) passes.
    #[default]
    Ground,
    /// Saw `C2`: one byte of lookahead for a C1 control encoded in UTF-8.
    Utf8C2,
    /// Saw `ESC`: the next byte says what kind of sequence this is.
    Esc,
    /// Accumulating a CSI into `buf` until its final byte (`0x40`–`0x7E`).
    Csi,
    /// Accumulating an OSC's identify window into `buf`.
    OscId,
    /// Accumulating a DCS's identify window into `buf`.
    DcsId,
    /// Inside a string (OSC or DCS) past identification, streaming.
    Str,
    /// Inside a string, saw `ESC`: maybe the `ESC \` string terminator.
    StrEsc,
    /// Inside a string, saw `C2`: maybe the `C2 9C` (C1 ST) terminator.
    StrC2,
}

pub struct VtFilter {
    state: State,
    /// The introducer plus the identify-window bytes of the sequence in
    /// flight. Bounded by `OSC_ID_CAP` / `CSI_CAP`; empty in every
    /// streaming and ground state.
    buf: Vec<u8>,
    /// A CSI whose parameters passed `CSI_CAP`. `buf` is frozen at the cap
    /// and nothing of the CSI is forwarded: it is dropped closed at its
    /// final byte, so leading-zero or padding bytes cannot re-form a `t`
    /// op past the cap the way passing it raw would let them.
    csi_over: bool,
    /// The OSC number parsed as the emulator parses it — numerically, so
    /// leading zeros collapse and unlimited padding never grows the
    /// buffer. Saturating; `osc_seen` says a digit was actually seen;
    /// `osc_over` says the buffer froze past the cap so the OSC, whatever
    /// its number, is dropped closed rather than passed unreproducible.
    osc_num: u32,
    osc_seen: bool,
    osc_over: bool,
    /// A DCS whose identify window (params and intermediates before the
    /// final byte) passed the cap. `buf` is frozen and the DCS is dropped
    /// closed: a DCS this long is only ever a padded query echo, so a
    /// padded `$ q`/`+ q` cannot stream its reply-echoing form through.
    dcs_over: bool,
    /// The current string is dropped (swallowed) rather than forwarded.
    str_drop: bool,
    /// A BEL ends the current string (OSC), as opposed to only ST (DCS).
    str_bel: bool,
}

impl Default for VtFilter {
    fn default() -> Self {
        Self {
            state: State::Ground,
            buf: Vec::new(),
            csi_over: false,
            osc_num: 0,
            osc_seen: false,
            osc_over: false,
            dcs_over: false,
            str_drop: false,
            str_bel: false,
        }
    }
}

impl VtFilter {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed one input chunk. Forwarded bytes are appended to `out`; the
    /// name of each class dropped in this chunk is appended to `dropped`
    /// (once per sequence). A sequence split across calls is completed on
    /// a later call; a sequence truncated at the end of `input` forwards
    /// nothing of itself and waits.
    pub fn feed(
        &mut self,
        input: &[u8],
        out: &mut Vec<u8>,
        dropped: &mut Vec<&'static str>,
    ) {
        for &b in input {
            self.step(b, out, dropped);
        }
    }

    /// Enter the streaming string state, discarding the identify buffer.
    fn enter_str(&mut self, drop: bool, bel: bool) {
        self.state = State::Str;
        self.str_drop = drop;
        self.str_bel = bel;
        self.buf.clear();
    }

    /// One byte through the automaton. Reprocesses a byte at most a couple
    /// of times when a state change hands it to another state.
    fn step(
        &mut self,
        b: u8,
        out: &mut Vec<u8>,
        dropped: &mut Vec<&'static str>,
    ) {
        match self.state {
            State::Ground => match b {
                0x1b => self.state = State::Esc,
                0xc2 => self.state = State::Utf8C2,
                _ => out.push(b),
            },
            State::Utf8C2 => match b {
                0x9b => self.open(&[0xc2, 0x9b], State::Csi),
                0x9d => self.open(&[0xc2, 0x9d], State::OscId),
                0x90 => self.open(&[0xc2, 0x90], State::DcsId),
                // Any other C1 control (ST included), or a Latin-1 char
                // (U+00A0–U+00BF): pass the two UTF-8 bytes unchanged.
                0x80..=0xbf => {
                    out.extend_from_slice(&[0xc2, b]);
                    self.state = State::Ground;
                }
                // Not a continuation byte: the C2 was malformed. Forward
                // it and reprocess this byte from ground.
                _ => {
                    out.push(0xc2);
                    self.state = State::Ground;
                    self.step(b, out, dropped);
                }
            },
            State::Esc => match b {
                b'[' => self.open(&[0x1b, b'['], State::Csi),
                b']' => self.open(&[0x1b, b']'], State::OscId),
                b'P' => self.open(&[0x1b, b'P'], State::DcsId),
                // Any other escape (RIS, charset selection, a stray ST):
                // it draws or sets state, so forward the ESC and reprocess
                // this byte from ground.
                _ => {
                    out.push(0x1b);
                    self.state = State::Ground;
                    self.step(b, out, dropped);
                }
            },
            State::Csi => {
                if (0x40..=0x7e).contains(&b) {
                    // The final byte dispatches. A CSI past the cap is
                    // dropped closed regardless — its tail is withheld, so
                    // a padded op cannot reach the emulator.
                    match csi_decision(&self.buf, b) {
                        Some(name) => dropped.push(name),
                        None if self.csi_over => {}
                        None => {
                            out.extend_from_slice(&self.buf);
                            out.push(b);
                        }
                    }
                    self.buf.clear();
                    self.csi_over = false;
                    self.state = State::Ground;
                } else if (0x20..=0x3f).contains(&b) {
                    // A parameter or intermediate byte. Past the cap the
                    // buffer is frozen but the CSI stays open, so its final
                    // byte still decides — and drops it closed.
                    if !self.csi_over {
                        self.buf.push(b);
                        if self.buf.len() >= CSI_CAP {
                            self.csi_over = true;
                        }
                    }
                } else if matches!(b, 0x00..=0x17 | 0x19 | 0x1c..=0x1f) {
                    // A C0 control the emulator executes mid-CSI while the
                    // CSI stays open; forward it and stay, or a control
                    // spliced into the parameters would desync us from the
                    // emulator and let the op through.
                    out.push(b);
                } else if b == 0x7f {
                    // DEL is ignored inside a CSI; stay, forward nothing.
                } else {
                    // ESC, CAN, SUB, or a raw high byte cancels the CSI.
                    // Pass what was held raw (nothing if it is over the
                    // cap) and reprocess this byte from ground.
                    if !self.csi_over {
                        out.extend_from_slice(&self.buf);
                    }
                    self.buf.clear();
                    self.csi_over = false;
                    self.state = State::Ground;
                    self.step(b, out, dropped);
                }
            }
            State::OscId => {
                if b.is_ascii_digit() {
                    // The number, parsed as the emulator does: numerically.
                    // Leading zeros collapse, so no amount of padding hides
                    // the id — the running value keeps parsing even after
                    // the buffer freezes at the cap.
                    self.osc_seen = true;
                    self.osc_num = self
                        .osc_num
                        .saturating_mul(10)
                        .saturating_add(u32::from(b - b'0'));
                    if !self.osc_over {
                        self.buf.push(b);
                        if self.buf.len() >= OSC_ID_CAP {
                            self.osc_over = true;
                        }
                    }
                } else if matches!(
                    b,
                    0x00..=0x06 | 0x08..=0x17 | 0x19 | 0x1c..=0x1f | 0x7f
                ) {
                    // A C0 the emulator ignores inside the OSC number: skip
                    // it, or a control spliced into the number would keep
                    // us from recognising an id the emulator still parses.
                    // CAN (0x18) and SUB (0x1a) are deliberately excluded:
                    // the emulator aborts the whole sequence on them, while
                    // this filter lets them end the number and then swallows
                    // to the string terminator. The divergence is always in
                    // the fail-safe direction — an OSC followed by CAN/SUB
                    // can only over-drop trailing text, never pass an acting
                    // OSC.
                } else {
                    // Any other byte ends the number: `;`, a terminator, or
                    // a non-digit payload. Decide on the number seen. A
                    // terminator must still end the string it opened; `;`
                    // or a payload byte is part of the string.
                    let terminator = matches!(b, 0x07 | 0x1b | 0xc2);
                    match osc_decision(self.osc_seen, self.osc_num) {
                        Some(name) => {
                            dropped.push(name);
                            self.enter_str(true, true);
                            if terminator {
                                self.step(b, out, dropped);
                            }
                        }
                        // A number too long to reproduce is dropped closed
                        // even when it would pass, so nothing half-buffered
                        // is forwarded.
                        None if self.osc_over => {
                            self.enter_str(true, true);
                            if terminator {
                                self.step(b, out, dropped);
                            }
                        }
                        None => {
                            out.extend_from_slice(&self.buf);
                            self.enter_str(false, true);
                            self.step(b, out, dropped);
                        }
                    }
                }
            }
            State::DcsId => match b {
                // A parameter (`0`–`?`) or intermediate (` `–`/`) byte:
                // accumulate so the intermediate before the final is seen,
                // tolerating leading params before the `$`/`+`. Past the
                // cap the buffer freezes but the DCS stays open and is
                // dropped closed, so a padded query cannot stream through.
                0x20..=0x3f => {
                    if !self.dcs_over {
                        self.buf.push(b);
                        if self.buf.len() >= OSC_ID_CAP {
                            self.dcs_over = true;
                        }
                    }
                }
                // The final byte dispatches. Over the cap the DCS is
                // dropped closed — a DCS this long is only ever a padded
                // query echo. Otherwise `$ q` (DECRQSS) or `+ q`
                // (XTGETTCAP) — the intermediate immediately before a `q`
                // final, whatever params led it — echoes terminal state as
                // text. Every other DCS (tmux passthrough, sixel) passes.
                0x40..=0x7e => {
                    let intermediate = self.buf.last().copied();
                    if self.dcs_over
                        || (b == b'q'
                            && matches!(intermediate, Some(b'$' | b'+')))
                    {
                        dropped.push("query_echo");
                        self.enter_str(true, false);
                    } else {
                        self.buf.push(b);
                        out.extend_from_slice(&self.buf);
                        self.enter_str(false, false);
                    }
                }
                // A C0 the emulator ignores inside a DCS (BEL included: a
                // DCS ends only on ST): skip it and keep scanning, or a
                // control spliced before the `$`/`+` hides the query echo.
                0x00..=0x17 | 0x19 | 0x1c..=0x1f | 0x7f => {}
                // ESC or C2 can open the ST; anything else is malformed.
                // Over the cap this is dropped closed; otherwise nothing
                // matched the query-echo shape, so pass. Reprocess the byte
                // in the string state either way, so an ST still ends it.
                _ => {
                    if self.dcs_over {
                        dropped.push("query_echo");
                        self.enter_str(true, false);
                    } else {
                        out.extend_from_slice(&self.buf);
                        self.enter_str(false, false);
                    }
                    self.step(b, out, dropped);
                }
            },
            State::Str => match b {
                0x07 if self.str_bel => {
                    if !self.str_drop {
                        out.push(0x07);
                    }
                    self.state = State::Ground;
                }
                0x1b => self.state = State::StrEsc,
                0xc2 => self.state = State::StrC2,
                _ => {
                    if !self.str_drop {
                        out.push(b);
                    }
                }
            },
            State::StrEsc => {
                if b == b'\\' {
                    if !self.str_drop {
                        out.extend_from_slice(&[0x1b, b'\\']);
                    }
                    self.state = State::Ground;
                } else {
                    // The ESC began a new control, ending the string. Hand
                    // it and this byte to the escape state.
                    self.state = State::Esc;
                    self.step(b, out, dropped);
                }
            }
            State::StrC2 => match b {
                0x9c => {
                    if !self.str_drop {
                        out.extend_from_slice(&[0xc2, 0x9c]);
                    }
                    self.state = State::Ground;
                }
                // A UTF-8 char inside the string, not the C1 ST.
                0x80..=0xbf => {
                    if !self.str_drop {
                        out.extend_from_slice(&[0xc2, b]);
                    }
                    self.state = State::Str;
                }
                _ => {
                    if !self.str_drop {
                        out.push(0xc2);
                    }
                    self.state = State::Str;
                    self.step(b, out, dropped);
                }
            },
        }
    }

    /// Begin accumulating a sequence: seed `buf` with the introducer and
    /// reset the per-sequence identify state.
    fn open(&mut self, introducer: &[u8], state: State) {
        self.buf.clear();
        self.buf.extend_from_slice(introducer);
        self.csi_over = false;
        self.osc_num = 0;
        self.osc_seen = false;
        self.osc_over = false;
        self.dcs_over = false;
        self.state = state;
    }

    /// Bytes currently held in the identify buffer. For the test that a
    /// large hyperlink streams without the buffer growing past the cap.
    #[cfg(test)]
    #[must_use]
    fn buffered_len(&self) -> usize {
        self.buf.len()
    }
}

/// Whether a CSI ending in `final_byte` acts on the host. `prefix` is the
/// introducer and parameter bytes; only a `t` final is ever dropped.
fn csi_decision(prefix: &[u8], final_byte: u8) -> Option<&'static str> {
    if final_byte != b't' {
        return None;
    }
    let content = &prefix[2..];
    // `CSI > … t` sets the title modes.
    if content.first() == Some(&b'>') {
        return Some("title");
    }
    // The first numeric parameter names the window operation, read
    // numerically as the emulator reads it (so 018 is the 18 report). The
    // size and position reports have a fixed-form reply and pass; every
    // other op acts on the window or echoes its title/icon text back.
    let start = content
        .iter()
        .position(|&c| c != b'?' && c != b'<' && c != b'=')
        .unwrap_or(content.len());
    let end = start
        + content[start..]
            .iter()
            .take_while(|&&c| c.is_ascii_digit())
            .count();
    let param = std::str::from_utf8(&content[start..end])
        .ok()
        .and_then(|s| s.parse::<u32>().ok());
    match param {
        Some(11 | 13 | 14 | 16 | 18 | 19) => None,
        _ => Some("window"),
    }
}

/// Whether an OSC whose number resolved to `num` acts on the host: the
/// clipboard (52) and the title/icon (0, 1, 2). An unnumbered OSC (no
/// digit in the number position, `seen == false`) is treated as selector
/// 0 (set icon and title) and dropped as a title sequence: at least one
/// supported output path defaults the empty selector to 0, so it must be
/// dropped, not passed. Every other numbered OSC draws or reports a fixed
/// form and passes.
fn osc_decision(seen: bool, num: u32) -> Option<&'static str> {
    let num = if seen { num } else { 0 };
    match num {
        52 => Some("clipboard"),
        0..=2 => Some("title"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Feed one chunk and return what was forwarded and what was dropped.
    fn feed(
        filter: &mut VtFilter,
        input: &[u8],
    ) -> (Vec<u8>, Vec<&'static str>) {
        let mut out = Vec::new();
        let mut dropped = Vec::new();
        filter.feed(input, &mut out, &mut dropped);
        (out, dropped)
    }

    /// A whole session of one chunk: forwarded bytes and dropped names.
    fn once(input: &[u8]) -> (Vec<u8>, Vec<&'static str>) {
        feed(&mut VtFilter::new(), input)
    }

    #[test]
    fn a_full_screen_drawing_stream_passes_byte_for_byte() {
        // Clear, home, enter alt-screen, scroll region, SGR colour, text,
        // cursor motion, hide cursor, a 2-byte (©) and a 3-byte (✓) UTF-8
        // char, then leave the alt-screen. All of this draws; none acts.
        let input = b"\x1b[2J\x1b[H\x1b[?1049h\x1b[1;24r\x1b[31mred\x1b[0m\
              \x1b[5;10Hhere\x1b[?25l\xc2\xa9\xe2\x9c\x93 ok\x1b[?1049l";
        let (out, dropped) = once(input);

        assert_eq!(out, input, "a drawing stream is passed unchanged");
        assert!(dropped.is_empty(), "nothing drawn is dropped: {dropped:?}");
    }

    #[test]
    fn an_osc_52_clipboard_write_with_bel_is_dropped() {
        let (out, dropped) = once(b"before\x1b]52;c;SGVsbG8=\x07after");

        assert_eq!(out, b"beforeafter", "only the clipboard write is gone");
        assert_eq!(dropped, ["clipboard"]);
    }

    #[test]
    fn an_osc_52_with_a_string_terminator_is_dropped() {
        let (out, dropped) = once(b"x\x1b]52;c;SGVsbG8=\x1b\\y");

        assert_eq!(out, b"xy");
        assert_eq!(dropped, ["clipboard"]);
    }

    #[test]
    fn an_osc_52_split_across_three_feeds_is_dropped_once() {
        let mut filter = VtFilter::new();
        let mut out = Vec::new();
        let mut dropped = Vec::new();

        for chunk in [&b"draw\x1b]5"[..], b"2;c;AAAA", b"\x07more"] {
            filter.feed(chunk, &mut out, &mut dropped);
        }

        assert_eq!(out, b"drawmore", "the split write leaves no bytes");
        assert_eq!(dropped, ["clipboard"], "recorded once for the sequence");
    }

    #[test]
    fn an_osc_2_title_is_dropped() {
        let (out, dropped) = once(b"\x1b]2;my window title\x07");

        assert!(out.is_empty(), "the title set is gone: {out:?}");
        assert_eq!(dropped, ["title"]);
    }

    #[test]
    fn a_csi_21_t_title_report_is_dropped_as_window() {
        let (out, dropped) = once(b"\x1b[21t");

        assert!(out.is_empty(), "the acting window op is gone: {out:?}");
        assert_eq!(dropped, ["window"]);
    }

    #[test]
    fn a_csi_18_t_size_report_passes() {
        let input = b"\x1b[18t";
        let (out, dropped) = once(input);

        assert_eq!(out, input, "a fixed-form size report is kept");
        assert!(dropped.is_empty());
    }

    #[test]
    fn a_csi_gt_t_title_mode_is_dropped() {
        let (out, dropped) = once(b"\x1b[>2t");

        assert!(out.is_empty(), "a title-mode set is gone: {out:?}");
        assert_eq!(dropped, ["title"]);
    }

    #[test]
    fn device_and_status_queries_and_a_colour_query_pass() {
        // Primary device attributes, cursor position report, and the
        // background-colour query — their replies carry no attacker text.
        for seq in [
            &b"\x1b[c"[..],
            b"\x1b[6n",
            b"\x1b[?u",
            b"\x1b]11;?\x07",
            b"\x1b]10;?\x1b\\",
        ] {
            let (out, dropped) = once(seq);

            assert_eq!(out, seq, "a fixed-form query is kept: {seq:?}");
            assert!(dropped.is_empty(), "query dropped: {seq:?} {dropped:?}");
        }
    }

    #[test]
    fn a_decrqss_query_echo_is_dropped() {
        // DCS $ q m ST — a request for the SGR settings; the reply would
        // echo terminal state as text.
        let (out, dropped) = once(b"\x1bP$qm\x1b\\");

        assert!(out.is_empty(), "the query echo is gone: {out:?}");
        assert_eq!(dropped, ["query_echo"]);
    }

    #[test]
    fn an_xtgettcap_query_echo_is_dropped() {
        let (out, dropped) = once(b"\x1bP+q436f\x1b\\");

        assert!(out.is_empty(), "the capability query is gone: {out:?}");
        assert_eq!(dropped, ["query_echo"]);
    }

    #[test]
    fn a_tmux_passthrough_dcs_passes() {
        let input = b"\x1bPtmux;hello world\x1b\\";
        let (out, dropped) = once(input);

        assert_eq!(out, input, "tmux passthrough is not a query echo");
        assert!(dropped.is_empty());
    }

    #[test]
    fn a_sixel_dcs_passes() {
        let input = b"\x1bPq#0;2;0;0;0#0~~\x1b\\";
        let (out, dropped) = once(input);

        assert_eq!(out, input, "sixel graphics draw and are kept");
        assert!(dropped.is_empty());
    }

    #[test]
    fn an_osc_52_encoded_as_c1_in_utf8_is_dropped() {
        // C2 9D is U+009D (the OSC introducer) in UTF-8; C2 9C is the ST.
        // xterm.js interprets C1 after UTF-8 decoding, so this is as good
        // an OSC opener as ESC ]. The filter must close the bypass.
        let (out, dropped) = once(b"a\xc2\x9d52;c;AAAA\xc2\x9cb");

        assert_eq!(out, b"ab", "the C1-via-UTF-8 clipboard write is gone");
        assert_eq!(dropped, ["clipboard"]);
    }

    #[test]
    fn a_lone_9d_continuation_byte_passes() {
        // A raw 0x9D is a UTF-8 continuation byte, not a C1 introducer;
        // treating it as one would corrupt text.
        let input = b"before\x9dafter";
        let (out, dropped) = once(input);

        assert_eq!(out, input, "a raw continuation byte is left alone");
        assert!(dropped.is_empty());
    }

    #[test]
    fn a_large_osc_8_hyperlink_streams_under_the_identify_cap() {
        let mut payload = Vec::new();
        payload.extend_from_slice(b"\x1b]8;;https://example.com/");
        payload.resize(payload.len() + 100 * 1024, b'a');
        payload.extend_from_slice(b"\x1b\\");

        let mut filter = VtFilter::new();
        let mut out = Vec::new();
        let mut dropped = Vec::new();
        let mut max_buffered = 0;
        for chunk in payload.chunks(4096) {
            filter.feed(chunk, &mut out, &mut dropped);
            max_buffered = max_buffered.max(filter.buffered_len());
        }

        assert!(max_buffered <= 16, "buffer grew to {max_buffered} bytes");
        assert_eq!(out, payload, "the hyperlink draws and passes whole");
        assert!(dropped.is_empty());
    }

    #[test]
    fn a_sequence_truncated_at_the_end_of_a_feed_forwards_nothing_of_it() {
        let mut filter = VtFilter::new();

        // A CSI cut mid-parameter: nothing of it is forwarded yet.
        let (out1, dropped1) = feed(&mut filter, b"drawn\x1b[2");
        assert_eq!(out1, b"drawn", "the drawing before it is forwarded");
        assert!(dropped1.is_empty(), "no decision yet: {dropped1:?}");

        // The next feed completes it: CSI 21 t, an acting window op.
        let (out2, dropped2) = feed(&mut filter, b"1t");
        assert!(out2.is_empty(), "still nothing forwarded: {out2:?}");
        assert_eq!(dropped2, ["window"]);
    }

    #[test]
    fn an_escape_held_across_a_feed_boundary_completes_and_passes() {
        let mut filter = VtFilter::new();

        let (out1, dropped1) = feed(&mut filter, b"\x1b");
        assert!(out1.is_empty(), "a lone ESC is held: {out1:?}");
        assert!(dropped1.is_empty());

        let (out2, dropped2) = feed(&mut filter, b"[18t");
        assert_eq!(out2, b"\x1b[18t", "the size report is reassembled whole");
        assert!(dropped2.is_empty());
    }

    #[test]
    fn an_over_long_csi_t_op_is_dropped_closed_not_passed_raw() {
        // Passing an over-cap CSI raw would let the emulator absorb the
        // padding and dispatch the `t` op; withholding it closes that.
        let mut input = Vec::new();
        input.extend_from_slice(b"\x1b[");
        input.resize(input.len() + 300, b'1');
        input.push(b't');
        assert!(input.len() > 256, "the CSI is past the cap");

        let (out, dropped) = once(&input);

        assert!(out.is_empty(), "the padded op is withheld: {out:?}");
        assert_eq!(dropped, ["window"]);
    }

    #[test]
    fn an_over_cap_csi_that_would_pass_is_dropped_closed() {
        // A CSI whose final byte would otherwise pass (`m`, an SGR), but
        // past the cap: it cannot be reproduced, so it is withheld and,
        // being a would-pass final, recorded as nothing.
        let mut input = Vec::new();
        input.extend_from_slice(b"\x1b[");
        for _ in 0..140 {
            input.extend_from_slice(b"0;");
        }
        input.push(b'm');
        assert!(input.len() > 256, "the CSI is past the cap");

        let (out, dropped) = once(&input);

        assert!(out.is_empty(), "the over-cap CSI is withheld: {out:?}");
        assert!(dropped.is_empty(), "a would-pass final is not recorded");
    }

    // --- Non-canonical encodings: the emulator accepts these forms, so
    //     the filter must recognise them too, or they are bypasses. ---

    #[test]
    fn an_unnumbered_osc_is_dropped_as_title() {
        // An OSC with no digit in its selector position defaults to 0 on a
        // supported output path (set icon and title), so it must drop.
        let (out, dropped) = once(b"\x1b];pwned\x07");
        assert!(
            out.is_empty(),
            "the empty-selector title set is gone: {out:?}"
        );
        assert_eq!(dropped, ["title"]);

        // No `;` at all, a bare non-digit body: still selector 0.
        let (out, dropped) = once(b"\x1b]pwned\x07");
        assert!(out.is_empty(), "the unnumbered title set is gone: {out:?}");
        assert_eq!(dropped, ["title"]);

        // A real numbered drawing OSC still passes byte-for-byte.
        let link = b"\x1b]8;;https://x\x07";
        let (out, dropped) = once(link);
        assert_eq!(out, link, "the hyperlink is drawing and passes");
        assert!(dropped.is_empty());
    }

    #[test]
    fn an_osc_52_with_a_leading_zero_is_still_dropped() {
        // Emulators parse the OSC number numerically: 052 is 52.
        let (out, dropped) = once(b"before\x1b]052;c;SGk=\x07after");

        assert_eq!(out, b"beforeafter", "the padded clipboard write is gone");
        assert_eq!(dropped, ["clipboard"]);
    }

    #[test]
    fn an_osc_002_title_with_leading_zeros_is_still_dropped() {
        let (out, dropped) = once(b"\x1b]002;pwned\x07");

        assert!(out.is_empty(), "the padded title set is gone: {out:?}");
        assert_eq!(dropped, ["title"]);
    }

    #[test]
    fn an_osc_52_padded_past_the_identify_cap_is_still_dropped() {
        // Leading zeros keep the numeric value at 52 without growing the
        // buffer, so no length of padding turns it into a passed id.
        let mut input = Vec::new();
        input.extend_from_slice(b"\x1b]");
        input.resize(input.len() + 40, b'0');
        input.extend_from_slice(b"52;c;SGk=\x07");

        let (out, dropped) = once(&input);

        assert!(
            out.is_empty(),
            "the padded clipboard write is gone: {out:?}"
        );
        assert_eq!(dropped, ["clipboard"]);
    }

    #[test]
    fn a_genuinely_other_osc_number_still_passes() {
        // OSC 9 (desktop notification) is not one of ours and must pass,
        // proving the numeric match did not become a blanket drop.
        let input = b"\x1b]9;a build finished\x07";
        let (out, dropped) = once(input);

        assert_eq!(out, input, "an unlisted OSC number is kept");
        assert!(dropped.is_empty());
    }

    #[test]
    fn a_c0_control_spliced_into_a_csi_does_not_desync_the_drop() {
        // The emulator executes the BS mid-CSI and keeps the CSI open, so
        // param 21 still dispatches window op 21; the filter must too.
        let (out, dropped) = once(b"\x1b[21\x08t");

        assert_eq!(out, b"\x08", "only the executed control passes through");
        assert_eq!(dropped, ["window"]);
    }

    #[test]
    fn a_c0_control_spliced_into_an_osc_number_does_not_desync_the_drop() {
        // The emulator ignores the BS inside the OSC number, reading 52.
        let (out, dropped) = once(b"\x1b]5\x082;c;SGk=\x07");

        assert!(
            out.is_empty(),
            "the spliced clipboard write is gone: {out:?}"
        );
        assert_eq!(dropped, ["clipboard"]);
    }

    #[test]
    fn a_decrqss_with_a_leading_param_is_still_dropped() {
        // xterm dispatches DECRQSS by the `$` intermediate and `q` final
        // regardless of a leading parameter, so a leading `0` must not let
        // it through.
        let (out, dropped) = once(b"\x1bP0$qm\x1b\\");

        assert!(out.is_empty(), "the query echo is gone: {out:?}");
        assert_eq!(dropped, ["query_echo"]);
    }

    #[test]
    fn a_decrqss_padded_past_the_identify_cap_is_dropped_closed() {
        // Leading params past the cap must not let the DCS stream through:
        // the `$ q` still reaches the emulator and its reply echoes back.
        let mut input = Vec::new();
        input.extend_from_slice(b"\x1bP");
        input.resize(input.len() + 16, b'0');
        input.extend_from_slice(b"$qm\x1b\\");

        let (out, dropped) = once(&input);

        assert!(out.is_empty(), "the padded query echo is gone: {out:?}");
        assert_eq!(dropped, ["query_echo"]);
    }

    #[test]
    fn an_xtgettcap_padded_past_the_identify_cap_is_dropped_closed() {
        // The sharp case: the reply echoes attacker-chosen capability hex.
        let mut input = Vec::new();
        input.extend_from_slice(b"\x1bP");
        input.resize(input.len() + 16, b'0');
        input.extend_from_slice(b"+q544e\x1b\\");

        let (out, dropped) = once(&input);

        assert!(
            out.is_empty(),
            "the padded capability query is gone: {out:?}"
        );
        assert_eq!(dropped, ["query_echo"]);
    }

    #[test]
    fn a_sixel_with_leading_params_still_passes() {
        // Short params and a `q` final with no `$`/`+` before it: drawing.
        let input = b"\x1bP0;0;0q#0;2;0;0;0#0~~\x1b\\";
        let (out, dropped) = once(input);

        assert_eq!(out, input, "sixel graphics draw and are kept");
        assert!(dropped.is_empty());
    }

    #[test]
    fn a_long_but_legitimate_sgr_under_the_raised_cap_passes() {
        // A truecolor SGR combining several 24-bit colours runs past the
        // old 64-byte cap; the raised cap keeps it drawing, not dropped.
        let mut params = Vec::new();
        while params.len() < 120 {
            if !params.is_empty() {
                params.push(b';');
            }
            params.extend_from_slice(b"38;2;10;20;30");
        }
        let mut input = Vec::new();
        input.extend_from_slice(b"\x1b[");
        input.extend_from_slice(&params);
        input.push(b'm');
        assert!(input.len() > 64, "the SGR is past the old cap");

        let (out, dropped) = once(&input);

        assert_eq!(out, input, "a long real SGR is drawing and passes");
        assert!(dropped.is_empty());
    }

    #[test]
    fn a_zero_padded_csi_18_report_still_passes() {
        // The first parameter is read numerically, so 018 is the 18 size
        // report and is kept, not misread as a window op.
        let input = b"\x1b[018t";
        let (out, dropped) = once(input);

        assert_eq!(out, input, "a zero-padded size report is kept");
        assert!(dropped.is_empty());
    }
}
