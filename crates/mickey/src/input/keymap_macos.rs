//! evdev key codes to macOS Carbon/HIToolbox virtual key codes (`kVK_*`).
//! Pure logic, so it is tested on every platform.

/// Every key code Mickey maps is below 256.
const TABLE_LEN: usize = 256;

/// macOS virtual key codes are 7-bit (`0x00..=0x7E`); `0x00` is `kVK_ANSI_A`,
/// so `0xFF` marks an unmapped entry instead of `0`.
const NONE: u8 = u8::MAX;

/// Indexed by evdev code. `NONE` means "no macOS equivalent".
const TABLE: [u8; TABLE_LEN] = build_table();

const fn build_table() -> [u8; TABLE_LEN] {
    let mut table = [NONE; TABLE_LEN];

    // Main block: letters (ANSI physical positions).
    table[30] = 0x00; // KEY_A -> kVK_ANSI_A
    table[31] = 0x01; // KEY_S -> kVK_ANSI_S
    table[32] = 0x02; // KEY_D -> kVK_ANSI_D
    table[33] = 0x03; // KEY_F -> kVK_ANSI_F
    table[35] = 0x04; // KEY_H -> kVK_ANSI_H
    table[34] = 0x05; // KEY_G -> kVK_ANSI_G
    table[44] = 0x06; // KEY_Z -> kVK_ANSI_Z
    table[45] = 0x07; // KEY_X -> kVK_ANSI_X
    table[46] = 0x08; // KEY_C -> kVK_ANSI_C
    table[47] = 0x09; // KEY_V -> kVK_ANSI_V
    table[86] = 0x0A; // KEY_102ND -> kVK_ISO_Section (key left of Z on ISO/ABNT2)
    table[48] = 0x0B; // KEY_B -> kVK_ANSI_B
    table[16] = 0x0C; // KEY_Q -> kVK_ANSI_Q
    table[17] = 0x0D; // KEY_W -> kVK_ANSI_W
    table[18] = 0x0E; // KEY_E -> kVK_ANSI_E
    table[19] = 0x0F; // KEY_R -> kVK_ANSI_R
    table[21] = 0x10; // KEY_Y -> kVK_ANSI_Y
    table[20] = 0x11; // KEY_T -> kVK_ANSI_T

    // Number row and punctuation.
    table[2] = 0x12; // KEY_1
    table[3] = 0x13; // KEY_2
    table[4] = 0x14; // KEY_3
    table[5] = 0x15; // KEY_4
    table[7] = 0x16; // KEY_6
    table[6] = 0x17; // KEY_5
    table[13] = 0x18; // KEY_EQUAL
    table[10] = 0x19; // KEY_9
    table[8] = 0x1A; // KEY_7
    table[12] = 0x1B; // KEY_MINUS
    table[9] = 0x1C; // KEY_8
    table[11] = 0x1D; // KEY_0
    table[27] = 0x1E; // KEY_RIGHTBRACE
    table[24] = 0x1F; // KEY_O
    table[22] = 0x20; // KEY_U
    table[26] = 0x21; // KEY_LEFTBRACE
    table[23] = 0x22; // KEY_I
    table[25] = 0x23; // KEY_P
    table[28] = 0x24; // KEY_ENTER -> kVK_Return
    table[38] = 0x25; // KEY_L
    table[36] = 0x26; // KEY_J
    table[40] = 0x27; // KEY_APOSTROPHE
    table[37] = 0x28; // KEY_K
    table[39] = 0x29; // KEY_SEMICOLON
    table[43] = 0x2A; // KEY_BACKSLASH
    table[51] = 0x2B; // KEY_COMMA
    table[53] = 0x2C; // KEY_SLASH
    table[49] = 0x2D; // KEY_N
    table[50] = 0x2E; // KEY_M
    table[52] = 0x2F; // KEY_DOT
    table[15] = 0x30; // KEY_TAB
    table[57] = 0x31; // KEY_SPACE
    table[41] = 0x32; // KEY_GRAVE
    table[14] = 0x33; // KEY_BACKSPACE -> kVK_Delete
    table[1] = 0x35; // KEY_ESC -> kVK_Escape

    // Modifiers.
    table[126] = 0x36; // KEY_RIGHTMETA -> kVK_RightCommand
    table[125] = 0x37; // KEY_LEFTMETA -> kVK_Command
    table[42] = 0x38; // KEY_LEFTSHIFT -> kVK_Shift
    table[58] = 0x39; // KEY_CAPSLOCK -> kVK_CapsLock
    table[56] = 0x3A; // KEY_LEFTALT -> kVK_Option
    table[29] = 0x3B; // KEY_LEFTCTRL -> kVK_Control
    table[54] = 0x3C; // KEY_RIGHTSHIFT -> kVK_RightShift
    table[100] = 0x3D; // KEY_RIGHTALT (AltGr) -> kVK_RightOption
    table[97] = 0x3E; // KEY_RIGHTCTRL -> kVK_RightControl

    // Keypad.
    table[83] = 0x41; // KEY_KPDOT -> kVK_ANSI_KeypadDecimal
    table[55] = 0x43; // KEY_KPASTERISK -> kVK_ANSI_KeypadMultiply
    table[78] = 0x45; // KEY_KPPLUS -> kVK_ANSI_KeypadPlus
    table[69] = 0x47; // KEY_NUMLOCK -> kVK_ANSI_KeypadClear
    table[115] = 0x48; // KEY_VOLUMEUP -> kVK_VolumeUp
    table[114] = 0x49; // KEY_VOLUMEDOWN -> kVK_VolumeDown
    table[113] = 0x4A; // KEY_MUTE -> kVK_Mute
    table[98] = 0x4B; // KEY_KPSLASH -> kVK_ANSI_KeypadDivide
    table[96] = 0x4C; // KEY_KPENTER -> kVK_ANSI_KeypadEnter
    table[74] = 0x4E; // KEY_KPMINUS -> kVK_ANSI_KeypadMinus
    table[82] = 0x52; // KEY_KP0
    table[79] = 0x53; // KEY_KP1
    table[80] = 0x54; // KEY_KP2
    table[81] = 0x55; // KEY_KP3
    table[75] = 0x56; // KEY_KP4
    table[76] = 0x57; // KEY_KP5
    table[77] = 0x58; // KEY_KP6
    table[71] = 0x59; // KEY_KP7
    table[72] = 0x5B; // KEY_KP8
    table[73] = 0x5C; // KEY_KP9

    // ABNT2 / JIS extra keys.
    table[89] = 0x5E; // KEY_RO ("/ ?" next to Right Shift) -> kVK_JIS_Underscore
    table[121] = 0x5F; // KEY_KPCOMMA (ABNT2 keypad ".") -> kVK_JIS_KeypadComma

    // Function and navigation keys.
    table[63] = 0x60; // KEY_F5
    table[64] = 0x61; // KEY_F6
    table[65] = 0x62; // KEY_F7
    table[61] = 0x63; // KEY_F3
    table[66] = 0x64; // KEY_F8
    table[67] = 0x65; // KEY_F9
    table[87] = 0x67; // KEY_F11
    table[99] = 0x69; // KEY_SYSRQ (Print Screen) -> kVK_F13
    table[68] = 0x6D; // KEY_F10
    table[88] = 0x6F; // KEY_F12
    table[110] = 0x72; // KEY_INSERT -> kVK_Help
    table[102] = 0x73; // KEY_HOME
    table[104] = 0x74; // KEY_PAGEUP
    table[111] = 0x75; // KEY_DELETE -> kVK_ForwardDelete
    table[62] = 0x76; // KEY_F4
    table[107] = 0x77; // KEY_END
    table[60] = 0x78; // KEY_F2
    table[109] = 0x79; // KEY_PAGEDOWN
    table[59] = 0x7A; // KEY_F1
    table[105] = 0x7B; // KEY_LEFT
    table[106] = 0x7C; // KEY_RIGHT
    table[108] = 0x7D; // KEY_DOWN
    table[103] = 0x7E; // KEY_UP

    // ponytail: media transport keys (play/pause, next/prev) need macOS
    // NX_KEYTYPE system-defined events rather than CGEventCreateKeyboardEvent.
    table
}

/// Reverse mapping for a macOS source (`CGEventTap`), built from `TABLE` so
/// the two directions can never disagree. 0 means "unknown key".
const REVERSE: [u16; 128] = build_reverse();

const fn build_reverse() -> [u16; 128] {
    let mut reverse = [0u16; 128];
    let mut evdev = 0;
    while evdev < TABLE_LEN {
        let vk = TABLE[evdev];
        if vk != NONE && (vk as usize) < 128 {
            reverse[vk as usize] = evdev as u16;
        }
        evdev += 1;
    }
    reverse
}

/// The macOS virtual key code for an evdev code; `None` when unmapped.
pub fn macos_keycode(evdev_code: u16) -> Option<u16> {
    let entry = *TABLE.get(usize::from(evdev_code))?;
    (entry != NONE).then_some(u16::from(entry))
}

/// The evdev code for a macOS virtual key code; `None` for unknown keys.
pub fn evdev_code(vk: u16) -> Option<u16> {
    let code = *REVERSE.get(usize::from(vk))?;
    (code != 0).then_some(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_a_is_zero_and_still_maps() {
        assert_eq!(macos_keycode(30), Some(0x00));
        assert_eq!(evdev_code(0x00), Some(30));
    }

    #[test]
    fn modifiers_and_abnt2_keys_map() {
        assert_eq!(macos_keycode(42), Some(0x38)); // Left Shift
        assert_eq!(macos_keycode(54), Some(0x3C)); // Right Shift
        assert_eq!(macos_keycode(100), Some(0x3D)); // AltGr -> Right Option
        assert_eq!(macos_keycode(125), Some(0x37)); // Left Super -> Command
        assert_eq!(macos_keycode(86), Some(0x0A)); // ISO "\ |"
        assert_eq!(macos_keycode(89), Some(0x5E)); // ABNT2 "/ ?"
        assert_eq!(macos_keycode(121), Some(0x5F)); // ABNT2 keypad "."
    }

    #[test]
    fn unknown_keys_are_none() {
        assert_eq!(macos_keycode(0), None);
        assert_eq!(macos_keycode(84), None);
        assert_eq!(macos_keycode(256), None);
        assert_eq!(evdev_code(0x7F), None);
        assert_eq!(evdev_code(128), None);
    }

    #[test]
    fn every_mapped_key_round_trips() {
        for evdev in 0..TABLE_LEN as u16 {
            if let Some(vk) = macos_keycode(evdev) {
                assert_eq!(evdev_code(vk), Some(evdev), "evdev {evdev} -> vk {vk:#x}");
            }
        }
    }
}
