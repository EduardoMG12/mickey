//! Hotkeys written as text in `config.toml` (PRD 0007), e.g.
//! `"AltGr+RightShift+1"`: one or more modifiers, then at most one other
//! key. Keys are physical positions named after a US keyboard (on ABNT2 the
//! letters and digits are in the same places).

/// A hotkey as evdev codes: the modifiers (sorted) and the optional last key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chord {
    pub modifiers: Vec<u16>,
    pub key: Option<u16>,
}

impl Chord {
    /// Every key of the chord, modifiers first.
    pub fn keys(&self) -> impl Iterator<Item = u16> + '_ {
        self.modifiers.iter().copied().chain(self.key)
    }
}

const MODIFIERS: &[(&str, u16)] = &[
    ("leftctrl", 29),
    ("rightctrl", 97),
    ("leftshift", 42),
    ("rightshift", 54),
    ("leftalt", 56),
    ("altgr", 100),
    ("rightalt", 100),
    ("leftsuper", 125),
    ("rightsuper", 126),
];

/// Keys by name. Letters and digits follow their rows on the keyboard,
/// which is how evdev numbers them.
fn key_code(name: &str) -> Option<u16> {
    const ROWS: [(&str, u16); 4] = [
        ("1234567890", 2),
        ("qwertyuiop", 16),
        ("asdfghjkl", 30),
        ("zxcvbnm", 44),
    ];
    if let [c] = name.as_bytes() {
        for (row, first) in ROWS {
            if let Some(i) = row.bytes().position(|key| key == *c) {
                return Some(first + i as u16);
            }
        }
    }
    let named = match name {
        "space" => 57,
        "tab" => 15,
        "esc" => 1,
        "home" => 102,
        "end" => 107,
        "left" => 105,
        "right" => 106,
        "up" => 103,
        "down" => 108,
        "f11" => 87,
        "f12" => 88,
        _ => {
            // F1..F10 are consecutive, 59..68.
            let n: u16 = name.strip_prefix('f')?.parse().ok()?;
            return (1..=10).contains(&n).then_some(58 + n);
        }
    };
    Some(named)
}

/// One key by name, modifier or not, e.g. `RightShift` for a double tap.
pub fn parse_key(text: &str) -> Result<u16, String> {
    let name = text.trim().to_ascii_lowercase();
    MODIFIERS
        .iter()
        .find(|(known, _)| *known == name)
        .map(|&(_, code)| code)
        .or_else(|| key_code(&name))
        .ok_or_else(|| format!("`{text}` is not a key Mickey knows"))
}

pub fn parse(text: &str) -> Result<Chord, String> {
    let mut modifiers = Vec::new();
    let mut key = None;
    let parts: Vec<&str> = text.split('+').map(str::trim).collect();
    for (i, part) in parts.iter().enumerate() {
        let name = part.to_ascii_lowercase();
        if let Some(&(_, code)) = MODIFIERS.iter().find(|(known, _)| *known == name) {
            if modifiers.contains(&code) {
                return Err(format!("in `{text}`, `{part}` appears twice"));
            }
            modifiers.push(code);
        } else if let Some(code) = key_code(&name) {
            if i + 1 != parts.len() {
                return Err(format!("in `{text}`, only the last key may be `{part}`"));
            }
            key = Some(code);
        } else {
            return Err(format!("in `{text}`, `{part}` is not a key Mickey knows"));
        }
    }
    // A hotkey without a modifier would steal an ordinary key from typing.
    if modifiers.is_empty() {
        return Err(format!("`{text}` needs at least one modifier, e.g. AltGr"));
    }
    modifiers.sort_unstable();
    Ok(Chord { modifiers, key })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_m0_hotkey_parses() {
        assert_eq!(
            parse("AltGr+RightShift").unwrap(),
            Chord {
                modifiers: vec![54, 100],
                key: None
            }
        );
    }

    #[test]
    fn a_last_key_and_case_and_spaces() {
        assert_eq!(
            parse("altgr + RightShift + 1").unwrap(),
            Chord {
                modifiers: vec![54, 100],
                key: Some(2)
            }
        );
        assert_eq!(parse("LeftCtrl+H").unwrap().key, Some(35));
        assert_eq!(parse("LeftCtrl+M").unwrap().key, Some(50));
        assert_eq!(parse("LeftCtrl+0").unwrap().key, Some(11));
        assert_eq!(parse("LeftCtrl+F1").unwrap().key, Some(59));
        assert_eq!(parse("LeftCtrl+F10").unwrap().key, Some(68));
        assert_eq!(parse("LeftCtrl+F12").unwrap().key, Some(88));
    }

    #[test]
    fn mistakes_name_the_bad_part() {
        let err = |text| parse(text).unwrap_err();
        assert!(err("AltGr+Banana").contains("`Banana` is not a key"));
        assert!(err("AltGr+1+2").contains("only the last key"));
        assert!(err("AltGr+1+RightShift").contains("only the last key may be `1`"));
        assert!(err("AltGr+AltGr").contains("twice"));
        assert!(err("H").contains("needs at least one modifier"));
        assert!(err("AltGr+F13").contains("`F13` is not a key"));
    }

    #[test]
    fn single_keys_parse_by_name() {
        assert_eq!(parse_key("RightShift"), Ok(54));
        assert_eq!(parse_key("right"), Ok(106));
        assert!(parse_key("Banana").is_err());
    }

    #[test]
    fn keys_lists_modifiers_then_the_key() {
        let chord = parse("RightShift+AltGr+S").unwrap();
        assert_eq!(chord.keys().collect::<Vec<_>>(), [54, 100, 31]);
    }
}
