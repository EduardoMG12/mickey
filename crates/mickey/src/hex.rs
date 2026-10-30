//! Bytes as lowercase hex digits and back: keys in config.toml, the trust
//! list and the group map, stream keys, the release key and checksums.

/// Each byte as two lowercase hex digits.
pub fn encode(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Pairs of hex digits back to bytes; `None` for anything else.
pub fn decode(text: &str) -> Option<Vec<u8>> {
    if !text.is_ascii() || !text.len().is_multiple_of(2) {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).ok())
        .collect()
}

/// 64 hex digits back to a 32-byte key.
pub fn decode_key(text: &str) -> Option<[u8; 32]> {
    decode(text)?.try_into().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_round_trip() {
        let key: [u8; 32] = std::array::from_fn(|i| i as u8 * 7);
        assert_eq!(decode_key(&encode(&key)), Some(key));
        assert_eq!(decode("0aff"), Some(vec![0x0a, 0xff]));
    }

    #[test]
    fn anything_else_is_refused() {
        assert_eq!(decode("abc"), None);
        assert_eq!(decode("zz"), None);
        assert_eq!(decode("é0"), None);
        assert_eq!(decode_key("00"), None);
    }
}
