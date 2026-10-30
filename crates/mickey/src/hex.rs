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

/// 64 hex digits back to a 32-byte key. Allocates nothing: it runs on
/// every input event (`source::sound::audio_target`).
pub fn decode_key(text: &str) -> Option<[u8; 32]> {
    let digits = text.as_bytes();
    if digits.len() != 64 {
        return None;
    }
    let mut key = [0; 32];
    for (byte, [high, low]) in key.iter_mut().zip(digits.as_chunks::<2>().0) {
        *byte = digit(*high)? << 4 | digit(*low)?;
    }
    Some(key)
}

fn digit(c: u8) -> Option<u8> {
    char::from(c).to_digit(16).map(|value| value as u8)
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
        assert_eq!(decode_key(&"g".repeat(64)), None);
        assert_eq!(decode_key(&"é".repeat(32)), None, "64 bytes, not 64 digits");
    }

    #[test]
    fn a_key_reads_as_decode_reads_it() {
        let text = "00FFa0".repeat(10) + "0123";
        assert_eq!(decode_key(&text).map(Vec::from), decode(&text));
    }
}
