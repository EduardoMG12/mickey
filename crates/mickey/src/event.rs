//! Mickey's own input event. Codes are Linux evdev codes on every platform;
//! each emit backend translates them.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputEvent {
    Key {
        code: u16,
        pressed: bool,
    },
    Button {
        code: u16,
        pressed: bool,
    },
    Motion {
        dx: i32,
        dy: i32,
    },
    /// Hi-res wheel units: 120 per notch, like evdev's `REL_WHEEL_HI_RES`.
    Scroll {
        vertical: i32,
        horizontal: i32,
    },
}
