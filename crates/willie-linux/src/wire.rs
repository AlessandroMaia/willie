//! Framing shared by a session supervisor and its clients. Both directions
//! are framed: `type:u8 len:u16` big-endian, then the payload. Bytes flow
//! in `input`/`output`; everything else is a small JSON payload
//! (`willie_proto::supervisor`). Pure encoding, no I/O, so both binaries
//! share one definition and the tests run on any host.

use serde::{Serialize, de::DeserializeOwned};

/// Client → supervisor: bytes to write to the PTY master, as typed.
pub const INPUT: u8 = 0;
/// Client → supervisor: new window size, `rows:u16 cols:u16`, big-endian.
pub const RESIZE: u8 = 1;
/// Client → supervisor: leave, keep the session running.
pub const DETACH: u8 = 2;
/// Client → supervisor: mandatory first frame, JSON `Hello`.
pub const HELLO: u8 = 3;
/// Control client → supervisor: stop the harness (the stop ladder).
pub const STOP: u8 = 4;
/// Control client → supervisor: answer with a `STATUS` frame.
pub const STATUS_REQ: u8 = 5;
/// Supervisor → terminal: bytes from the PTY.
pub const OUTPUT: u8 = 16;
/// Supervisor → control: JSON `Status`.
pub const STATUS: u8 = 17;
/// Supervisor → control: one `events.jsonl` line, as written.
pub const EVENT: u8 = 18;
/// Supervisor → any client: JSON `Closed`, the last frame it sends.
pub const CLOSED: u8 = 19;

/// Header length: one type byte plus a 16-bit length.
pub const HEADER: usize = 3;
/// Largest payload a frame can carry, imposed by the 16-bit length.
pub const MAX_PAYLOAD: usize = u16::MAX as usize;

/// One decoded frame. The payload is empty for [`DETACH`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub kind: u8,
    pub payload: Vec<u8>,
}

/// Encode one frame. Payloads longer than [`MAX_PAYLOAD`] must be split by
/// the caller; every caller reads into a buffer smaller than that.
#[must_use]
pub fn encode(kind: u8, payload: &[u8]) -> Vec<u8> {
    let payload = &payload[..payload.len().min(MAX_PAYLOAD)];
    let len = u16::try_from(payload.len()).unwrap_or(u16::MAX);
    let mut out = Vec::with_capacity(HEADER + payload.len());
    out.push(kind);
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(payload);
    out
}

/// Encode a resize frame.
#[must_use]
pub fn encode_resize(rows: u16, cols: u16) -> Vec<u8> {
    let mut payload = [0u8; 4];
    payload[..2].copy_from_slice(&rows.to_be_bytes());
    payload[2..].copy_from_slice(&cols.to_be_bytes());
    encode(RESIZE, &payload)
}

/// Read a resize payload back. `None` when it is not four bytes, which is
/// how a client of a different vintage looks.
#[must_use]
pub fn decode_resize(payload: &[u8]) -> Option<(u16, u16)> {
    if payload.len() != 4 {
        return None;
    }
    let rows = u16::from_be_bytes([payload[0], payload[1]]);
    let cols = u16::from_be_bytes([payload[2], payload[3]]);
    Some((rows, cols))
}

/// Encode a JSON payload frame (`hello`, `status`, `event`, `closed`).
pub fn encode_json<T: Serialize>(
    kind: u8,
    value: &T,
) -> serde_json::Result<Vec<u8>> {
    Ok(encode(kind, &serde_json::to_vec(value)?))
}

/// Decode a JSON payload.
pub fn decode_json<T: DeserializeOwned>(
    payload: &[u8],
) -> serde_json::Result<T> {
    serde_json::from_slice(payload)
}

/// Split `bytes` into as many frames as the 16-bit length needs. PTY
/// output is read in 16 KiB chunks, so this is one frame in practice.
#[must_use]
pub fn chunks(kind: u8, bytes: &[u8]) -> Vec<Vec<u8>> {
    if bytes.is_empty() {
        return vec![encode(kind, b"")];
    }
    bytes.chunks(MAX_PAYLOAD).map(|c| encode(kind, c)).collect()
}

/// Reassembles frames from a stream that splits them anywhere.
#[derive(Debug, Default)]
pub struct Decoder {
    buf: Vec<u8>,
}

impl Decoder {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add bytes as they arrive from the socket.
    pub fn push(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    /// Next complete frame, if the buffer holds one.
    pub fn pop(&mut self) -> Option<Frame> {
        if self.buf.len() < HEADER {
            return None;
        }
        let kind = self.buf[0];
        let len = usize::from(u16::from_be_bytes([self.buf[1], self.buf[2]]));
        if self.buf.len() < HEADER + len {
            return None;
        }
        let payload = self.buf[HEADER..HEADER + len].to_vec();
        self.buf.drain(..HEADER + len);
        Some(Frame { kind, payload })
    }

    /// Bytes held back waiting for the rest of a frame.
    #[must_use]
    pub fn pending(&self) -> usize {
        self.buf.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_input_frame_round_trips() {
        let mut d = Decoder::new();
        d.push(&encode(INPUT, b"ls -la\r"));
        assert_eq!(
            d.pop(),
            Some(Frame {
                kind: INPUT,
                payload: b"ls -la\r".to_vec()
            })
        );
        assert_eq!(d.pop(), None);
    }

    #[test]
    fn a_resize_frame_round_trips() {
        let mut d = Decoder::new();
        d.push(&encode_resize(50, 160));
        let frame = d.pop().unwrap();
        assert_eq!(frame.kind, RESIZE);
        assert_eq!(decode_resize(&frame.payload), Some((50, 160)));
    }

    #[test]
    fn a_detach_frame_has_no_payload() {
        assert_eq!(encode(DETACH, b""), vec![DETACH, 0, 0]);
    }

    #[test]
    fn frames_split_byte_by_byte_are_reassembled() {
        let mut wire = encode(INPUT, b"abc");
        wire.extend_from_slice(&encode_resize(24, 80));
        let mut d = Decoder::new();
        let mut frames = Vec::new();
        for byte in &wire {
            d.push(&[*byte]);
            while let Some(f) = d.pop() {
                frames.push(f);
            }
        }
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].payload, b"abc");
        assert_eq!(decode_resize(&frames[1].payload), Some((24, 80)));
        assert_eq!(d.pending(), 0);
    }

    #[test]
    fn two_frames_arriving_together_are_both_decoded() {
        let mut wire = encode(INPUT, b"x");
        wire.extend_from_slice(&encode(DETACH, b""));
        let mut d = Decoder::new();
        d.push(&wire);
        assert_eq!(d.pop().map(|f| f.kind), Some(INPUT));
        assert_eq!(d.pop().map(|f| f.kind), Some(DETACH));
        assert_eq!(d.pop(), None);
    }

    #[test]
    fn a_truncated_header_yields_nothing_and_is_kept() {
        let mut d = Decoder::new();
        d.push(&[INPUT, 0]);
        assert_eq!(d.pop(), None);
        assert_eq!(d.pending(), 2);
    }

    #[test]
    fn a_payload_longer_than_the_length_field_is_truncated() {
        let big = vec![b'z'; MAX_PAYLOAD + 10];
        let wire = encode(INPUT, &big);
        assert_eq!(wire.len(), HEADER + MAX_PAYLOAD);
    }

    #[test]
    fn a_resize_payload_of_the_wrong_size_is_rejected() {
        assert_eq!(decode_resize(&[0, 24, 0]), None);
        assert_eq!(decode_resize(&[]), None);
    }

    #[test]
    fn the_type_ids_are_the_documented_ones() {
        assert_eq!(
            (INPUT, RESIZE, DETACH, HELLO, STOP, STATUS_REQ),
            (0, 1, 2, 3, 4, 5)
        );
        assert_eq!((OUTPUT, STATUS, EVENT, CLOSED), (16, 17, 18, 19));
    }

    #[test]
    fn a_json_payload_round_trips_through_a_frame() {
        let hello = willie_proto::supervisor::Hello {
            role: willie_proto::supervisor::Role::Terminal,
            rows: 51,
            cols: 140,
        };
        let mut d = Decoder::new();
        d.push(&encode_json(HELLO, &hello).unwrap());
        let f = d.pop().unwrap();
        assert_eq!(f.kind, HELLO);
        let back: willie_proto::supervisor::Hello =
            decode_json(&f.payload).unwrap();
        assert_eq!(back, hello);
    }

    #[test]
    fn output_longer_than_a_frame_is_split_into_full_frames() {
        let bytes = vec![b'x'; MAX_PAYLOAD * 2 + 5];
        let frames = chunks(OUTPUT, &bytes);
        assert_eq!(frames.len(), 3);
        let mut d = Decoder::new();
        for f in &frames {
            d.push(f);
        }
        let total: usize = std::iter::from_fn(|| d.pop())
            .map(|f| f.payload.len())
            .sum();
        assert_eq!(total, bytes.len());
    }
}
