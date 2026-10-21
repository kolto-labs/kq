//! Byte-exact text conversion.
//!
//! Game data files and the INI instruction files store text as single-byte
//! characters. Every byte 0x00-0xFF maps to exactly one `char` in U+0000-U+00FF,
//! so the conversion round-trips any byte sequence without loss. Keeping this
//! mapping means string lengths written into binary headers always match the
//! bytes that follow them.

/// Decode raw bytes into a string, one byte per character.
pub fn decode(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| b as char).collect()
}

/// Encode a string back to raw bytes, one character per byte.
///
/// Characters above U+00FF cannot occur in text that came from `decode`. If one
/// is present anyway it is written as `?`, matching how the original tooling
/// degrades unmappable characters.
pub fn encode(text: &str) -> Vec<u8> {
    text.chars()
        .map(|c| if (c as u32) <= 0xFF { c as u8 } else { b'?' })
        .collect()
}

/// Byte length of `text` once encoded. Equal to the character count.
pub fn byte_len(text: &str) -> usize {
    text.chars().count()
}

/// Decode a fixed-width field, stopping at the first NUL.
pub fn decode_nul_terminated(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    decode(&bytes[..end])
}

/// Encode into a fixed-width NUL-padded buffer, truncating if too long.
pub fn encode_fixed<const N: usize>(text: &str) -> [u8; N] {
    let mut out = [0u8; N];
    for (slot, byte) in out.iter_mut().zip(encode(text)) {
        *slot = byte;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_every_byte() {
        let all: Vec<u8> = (0..=255u8).collect();
        assert_eq!(encode(&decode(&all)), all);
    }

    #[test]
    fn byte_len_matches_encoded_length() {
        let text = decode(&[0x41, 0xE9, 0xFF, 0x00, 0x7A]);
        assert_eq!(byte_len(&text), 5);
        assert_eq!(encode(&text).len(), 5);
    }

    #[test]
    fn fixed_buffer_pads_with_nul() {
        let raw: [u8; 8] = encode_fixed("abc");
        assert_eq!(&raw, b"abc\0\0\0\0\0");
    }

    #[test]
    fn fixed_buffer_truncates() {
        let raw: [u8; 4] = encode_fixed("abcdefgh");
        assert_eq!(&raw, b"abcd");
    }

    #[test]
    fn nul_terminated_stops_early() {
        assert_eq!(decode_nul_terminated(b"one\0two"), "one");
        assert_eq!(decode_nul_terminated(b"nonul"), "nonul");
    }
}
