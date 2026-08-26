//! Text produced by Windows console tools.
//!
//! `wsl.exe` writes its own messages (version, errors, listings) as
//! UTF-16LE; a program it launches with `--exec` writes raw bytes, usually
//! UTF-8. The decoder tells the two apart by the BOM or by the NUL high
//! bytes that Latin text produces in UTF-16LE.

/// Decodes console output that may be UTF-16LE or UTF-8.
#[must_use]
pub fn decode_wsl_output(bytes: &[u8]) -> String {
    let has_bom = bytes.starts_with(&[0xFF, 0xFE]);
    let high_zeroes =
        bytes.iter().skip(1).step_by(2).filter(|b| **b == 0).count();
    let looks_utf16 =
        has_bom || (bytes.len() >= 4 && high_zeroes > bytes.len() / 4);
    if !looks_utf16 {
        return String::from_utf8_lossy(bytes).into_owned();
    }
    let units: Vec<u16> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_le_bytes(*pair))
        .collect();
    String::from_utf16_lossy(&units)
        .trim_start_matches('\u{feff}')
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utf16le(text: &str, bom: bool) -> Vec<u8> {
        let mut out = if bom { vec![0xFF, 0xFE] } else { Vec::new() };
        for unit in text.encode_utf16() {
            out.extend_from_slice(&unit.to_le_bytes());
        }
        out
    }

    #[test]
    fn utf16le_with_bom_is_decoded_without_the_bom() {
        assert_eq!(
            decode_wsl_output(&utf16le("Versão do WSL: 2.6.1.0", true)),
            "Versão do WSL: 2.6.1.0"
        );
    }

    #[test]
    fn utf16le_without_bom_is_recognised_by_its_nul_bytes() {
        assert_eq!(
            decode_wsl_output(&utf16le("NAME  STATE  VERSION", false)),
            "NAME  STATE  VERSION"
        );
    }

    #[test]
    fn utf8_is_returned_unchanged() {
        assert_eq!(
            decode_wsl_output("cargo 1.98.0 ✓\n".as_bytes()),
            "cargo 1.98.0 ✓\n"
        );
    }

    #[test]
    fn empty_input_is_empty_output() {
        assert_eq!(decode_wsl_output(b""), "");
    }
}
