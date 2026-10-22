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

/// A key's name as config.toml writes it, e.g. `RightShift`, `A`, `F5`.
pub fn key_name(code: u16) -> Option<String> {
    const MODIFIER_NAMES: [(u16, &str); 8] = [
        (29, "LeftCtrl"),
        (97, "RightCtrl"),
        (42, "LeftShift"),
        (54, "RightShift"),
        (56, "LeftAlt"),
        (100, "AltGr"),
        (125, "LeftSuper"),
        (126, "RightSuper"),
    ];
    if let Some((_, name)) = MODIFIER_NAMES.iter().find(|(known, _)| *known == code) {
        return Some(name.to_string());
    }
    let named = match code {
        57 => "Space",
        15 => "Tab",
        1 => "Esc",
        102 => "Home",
        107 => "End",
        105 => "Left",
        106 => "Right",
        103 => "Up",
        108 => "Down",
        87 => "F11",
        88 => "F12",
        59..=68 => return Some(format!("F{}", code - 58)),
        _ => {
            // Letters and digits: find the row they are on.
            const ROWS: [(&str, u16); 4] = [
                ("1234567890", 2),
                ("QWERTYUIOP", 16),
                ("ASDFGHJKL", 30),
                ("ZXCVBNM", 44),
            ];
            return ROWS.iter().find_map(|(row, first)| {
                let at = code.checked_sub(*first)? as usize;
                row.get(at..at + 1).map(str::to_string)
            });
        }
    };
    Some(named.to_string())
}

/// The text for keys pressed together, in the order they went down: the
/// modifiers first, then the other key. Checked with `parse`, so what
/// comes out is a hotkey Mickey accepts.
pub fn name_keys(pressed: &[u16]) -> Result<String, String> {
    let is_modifier = |code: &u16| MODIFIERS.iter().any(|(_, known)| known == code);
    let mut names = Vec::new();
    for code in pressed.iter().filter(|code| is_modifier(code)) {
        names.push(key_name(*code).ok_or("an unknown modifier")?);
    }
    let others: Vec<&u16> = pressed.iter().filter(|code| !is_modifier(code)).collect();
    match others[..] {
        [] => {}
        [code] => names
            .push(key_name(*code).ok_or_else(|| format!("key {code} has no name in Mickey yet"))?),
        _ => return Err("only one key besides the modifiers, e.g. AltGr+RightShift+H".into()),
    }
    let text = names.join("+");
    parse(&text)?;
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_go_back_to_what_parses() {
        for text in [
            "AltGr+RightShift",
            "LeftCtrl+LeftShift+H",
            "AltGr+Right",
            "RightCtrl+F5",
            "LeftSuper+1",
        ] {
            let chord = parse(text).unwrap();
            let pressed: Vec<u16> = chord.keys().collect();
            assert_eq!(
                parse(&name_keys(&pressed).unwrap()).unwrap(),
                chord,
                "{text}"
            );
        }
        assert_eq!(name_keys(&[100, 106]).unwrap(), "AltGr+Right");
        assert_eq!(key_name(54).unwrap(), "RightShift");
        assert_eq!(key_name(30).unwrap(), "A");
        assert!(name_keys(&[30]).is_err(), "no modifier");
        assert!(name_keys(&[29, 30, 31]).is_err(), "two plain keys");
    }

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
