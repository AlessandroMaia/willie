//! The host dialect: the two messages the desktop app sends
//! `willie attach --host` on its stdin. Pure and transport-agnostic — the
//! engine encodes, the CLI decodes; neither reads nor writes here.
//!
//! Frames:
//!   input:  b'i', u32-be length, then that many payload bytes
//!   resize: b'r', u16-be rows, u16-be cols
//! The session's output travels raw on the child's stdout; it needs no
//! host framing.

const TAG_INPUT: u8 = b'i';
const TAG_RESIZE: u8 = b'r';

/// One decoded host frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostFrame {
    Input(Vec<u8>),
    Resize { rows: u16, cols: u16 },
}

/// A frame whose tag is not part of the dialect: a desynchronised or
/// wrong-protocol stream. `willie-proto` has no `thiserror` dependency, so
/// the `Error` impl is written by hand (a few lines, no new dep).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownTag(pub u8);

impl std::fmt::Display for UnknownTag {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "unknown host frame tag {:#04x}", self.0)
    }
}

impl std::error::Error for UnknownTag {}

/// Encode an input frame (keystrokes/paste).
#[must_use]
pub fn encode_input(bytes: &[u8]) -> Vec<u8> {
    let len = u32::try_from(bytes.len()).unwrap_or(u32::MAX);
    let mut v = Vec::with_capacity(5 + bytes.len());
    v.push(TAG_INPUT);
    v.extend_from_slice(&len.to_be_bytes());
    v.extend_from_slice(bytes);
    v
}

/// Encode a resize frame (rows before cols, as the tty reports).
#[must_use]
pub fn encode_resize(rows: u16, cols: u16) -> Vec<u8> {
    let mut v = Vec::with_capacity(5);
    v.push(TAG_RESIZE);
    v.extend_from_slice(&rows.to_be_bytes());
    v.extend_from_slice(&cols.to_be_bytes());
    v
}

/// Decode the first frame in `buf`. `Ok(None)` means `buf` does not yet
/// hold a whole frame (read more, then retry); `Ok(Some((frame, used)))`
/// returns the frame and how many bytes it consumed; `Err` is an
/// unrecognised tag.
pub fn decode_frame(
    buf: &[u8],
) -> Result<Option<(HostFrame, usize)>, UnknownTag> {
    let Some((&tag, rest)) = buf.split_first() else {
        return Ok(None);
    };
    match tag {
        TAG_INPUT => {
            if rest.len() < 4 {
                return Ok(None);
            }
            let len = u32::from_be_bytes([rest[0], rest[1], rest[2], rest[3]])
                as usize;
            let body = &rest[4..];
            if body.len() < len {
                return Ok(None);
            }
            Ok(Some((HostFrame::Input(body[..len].to_vec()), 5 + len)))
        }
        TAG_RESIZE => {
            if rest.len() < 4 {
                return Ok(None);
            }
            let rows = u16::from_be_bytes([rest[0], rest[1]]);
            let cols = u16::from_be_bytes([rest[2], rest[3]]);
            Ok(Some((HostFrame::Resize { rows, cols }, 5)))
        }
        other => Err(UnknownTag(other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_input_frame_round_trips() {
        let f = encode_input(b"ls -la\n");
        let (frame, used) = decode_frame(&f).unwrap().unwrap();
        assert_eq!(frame, HostFrame::Input(b"ls -la\n".to_vec()));
        assert_eq!(used, f.len());
    }

    #[test]
    fn a_resize_frame_round_trips() {
        let f = encode_resize(30, 100);
        let (frame, used) = decode_frame(&f).unwrap().unwrap();
        assert_eq!(
            frame,
            HostFrame::Resize {
                rows: 30,
                cols: 100
            }
        );
        assert_eq!(used, f.len());
    }

    #[test]
    fn a_partial_buffer_yields_none_not_an_error() {
        let f = encode_input(b"abcd");
        // Every strict prefix is incomplete.
        for cut in 0..f.len() {
            assert_eq!(decode_frame(&f[..cut]).unwrap(), None);
        }
    }

    #[test]
    fn decode_reports_bytes_consumed_and_leaves_the_rest() {
        let mut buf = encode_resize(24, 80);
        buf.extend_from_slice(&encode_input(b"x"));
        let (first, used) = decode_frame(&buf).unwrap().unwrap();
        assert_eq!(first, HostFrame::Resize { rows: 24, cols: 80 });
        let (second, _) = decode_frame(&buf[used..]).unwrap().unwrap();
        assert_eq!(second, HostFrame::Input(b"x".to_vec()));
    }

    #[test]
    fn an_unknown_tag_is_an_error() {
        assert_eq!(decode_frame(b"Zxx"), Err(UnknownTag(b'Z')));
    }
}
