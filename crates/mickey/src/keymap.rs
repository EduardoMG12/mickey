//! evdev key and button codes to what Windows expects. Pure logic, so it is
//! tested on every platform.

/// A Windows "set 1" scan code: the physical key position. `extended` keys
/// are sent with the `E0` prefix (`KEYEVENTF_EXTENDEDKEY` in `SendInput`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScanCode {
    pub code: u8,
    pub extended: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    /// Back side button (`XBUTTON1`).
    X1,
    /// Forward side button (`XBUTTON2`).
    X2,
}

/// Every key code Mickey maps is below 256.
const TABLE_LEN: usize = 256;

/// Marks a table entry as extended; the low byte is the scan code.
const E0: u16 = 0xE000;

/// Indexed by evdev code. 0 means "no Windows equivalent".
const TABLE: [u16; TABLE_LEN] = build_table();

const fn build_table() -> [u16; TABLE_LEN] {
    let mut table = [0u16; TABLE_LEN];

    // evdev codes 1..=83 (Esc to keypad dot) were taken from set 1, so they
    // are already the Windows scan code.
    let mut code = 1;
    while code <= 83 {
        table[code] = code as u16;
        code += 1;
    }

    table[86] = 0x56; // KEY_102ND: the extra key left of Z on ISO keyboards
    table[87] = 0x57; // KEY_F11
    table[88] = 0x58; // KEY_F12
    table[89] = 0x73; // KEY_RO: "/ ?" next to Right Shift on ABNT2
    table[121] = 0x7E; // KEY_KPCOMMA: the ABNT2 keypad "."

    table[96] = E0 | 0x1C; // KEY_KPENTER
    table[97] = E0 | 0x1D; // KEY_RIGHTCTRL
    table[98] = E0 | 0x35; // KEY_KPSLASH
    table[99] = E0 | 0x37; // KEY_SYSRQ (Print Screen)
    table[100] = E0 | 0x38; // KEY_RIGHTALT (AltGr)
    table[102] = E0 | 0x47; // KEY_HOME
    table[103] = E0 | 0x48; // KEY_UP
    table[104] = E0 | 0x49; // KEY_PAGEUP
    table[105] = E0 | 0x4B; // KEY_LEFT
    table[106] = E0 | 0x4D; // KEY_RIGHT
    table[107] = E0 | 0x4F; // KEY_END
    table[108] = E0 | 0x50; // KEY_DOWN
    table[109] = E0 | 0x51; // KEY_PAGEDOWN
    table[110] = E0 | 0x52; // KEY_INSERT
    table[111] = E0 | 0x53; // KEY_DELETE
    table[113] = E0 | 0x20; // KEY_MUTE
    table[114] = E0 | 0x2E; // KEY_VOLUMEDOWN
    table[115] = E0 | 0x30; // KEY_VOLUMEUP
    table[125] = E0 | 0x5B; // KEY_LEFTMETA (Windows key)
    table[126] = E0 | 0x5C; // KEY_RIGHTMETA
    table[127] = E0 | 0x5D; // KEY_COMPOSE (Menu)
    table[163] = E0 | 0x19; // KEY_NEXTSONG
    table[164] = E0 | 0x22; // KEY_PLAYPAUSE
    table[165] = E0 | 0x10; // KEY_PREVIOUSSONG
    table[166] = E0 | 0x24; // KEY_STOPCD

    // ponytail: KEY_PAUSE (119) needs the 6-byte E1 sequence, not a scan
    // code; it is dropped until someone misses it.
    table
}

/// `None` when the key has no Windows equivalent; the caller drops it.
pub fn scan_code(evdev_code: u16) -> Option<ScanCode> {
    let entry = *TABLE.get(usize::from(evdev_code))?;
    if entry == 0 {
        return None;
    }
    Some(ScanCode {
        code: (entry & 0xFF) as u8,
        extended: entry & E0 != 0,
    })
}

pub fn mouse_button(evdev_code: u16) -> Option<MouseButton> {
    match evdev_code {
        0x110 => Some(MouseButton::Left),   // BTN_LEFT
        0x111 => Some(MouseButton::Right),  // BTN_RIGHT
        0x112 => Some(MouseButton::Middle), // BTN_MIDDLE
        0x113 => Some(MouseButton::X1),     // BTN_SIDE
        0x114 => Some(MouseButton::X2),     // BTN_EXTRA
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(code: u8) -> Option<ScanCode> {
        Some(ScanCode {
            code,
            extended: false,
        })
    }

    fn extended(code: u8) -> Option<ScanCode> {
        Some(ScanCode {
            code,
            extended: true,
        })
    }

    #[test]
    fn main_block_keys_keep_their_code() {
        assert_eq!(scan_code(1), plain(0x01)); // Esc
        assert_eq!(scan_code(30), plain(0x1E)); // A
        assert_eq!(scan_code(29), plain(0x1D)); // Left Ctrl
        assert_eq!(scan_code(54), plain(0x36)); // Right Shift
        assert_eq!(scan_code(83), plain(0x53)); // keypad dot
    }

    #[test]
    fn abnt2_keys_are_mapped() {
        assert_eq!(scan_code(89), plain(0x73)); // "/ ?"
        assert_eq!(scan_code(121), plain(0x7E)); // keypad "."
        assert_eq!(scan_code(86), plain(0x56)); // "\ |"
    }

    #[test]
    fn navigation_and_right_modifiers_are_extended() {
        assert_eq!(scan_code(100), extended(0x38)); // AltGr
        assert_eq!(scan_code(97), extended(0x1D)); // Right Ctrl
        assert_eq!(scan_code(103), extended(0x48)); // Up
        assert_eq!(scan_code(111), extended(0x53)); // Delete
        assert_eq!(scan_code(96), extended(0x1C)); // keypad Enter
        assert_eq!(scan_code(125), extended(0x5B)); // Super
    }

    #[test]
    fn unknown_keys_are_none() {
        assert_eq!(scan_code(0), None);
        assert_eq!(scan_code(84), None); // gap in the evdev codes
        assert_eq!(scan_code(119), None); // Pause, see the ponytail note
        assert_eq!(scan_code(255), None);
        assert_eq!(scan_code(256), None); // past the table
        assert_eq!(scan_code(u16::MAX), None);
    }

    #[test]
    fn mouse_buttons_include_the_side_buttons() {
        assert_eq!(mouse_button(0x110), Some(MouseButton::Left));
        assert_eq!(mouse_button(0x113), Some(MouseButton::X1));
        assert_eq!(mouse_button(0x114), Some(MouseButton::X2));
        assert_eq!(mouse_button(0x115), None);
    }
}
