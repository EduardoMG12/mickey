//! Hand-written bindings for macOS CoreGraphics and ApplicationServices.
//! Used on Apple Silicon (`aarch64-apple-darwin`) for monitors, cursor
//! positioning, and input injection (`CGEventPost`).

use crate::input::keymap::MouseButton;
use crate::monitors::Monitor;
use std::ffi::c_void;
use std::io;

/// Marks every event Mickey injects (`kCGEventSourceUserData`), so a macOS
/// source (`CGEventTap`) can tell them apart from physical input. "MICK" in ASCII.
pub const MICKEY_INPUT: i64 = 0x4D49_434B;

const CG_EVENT_LEFT_MOUSE_DOWN: u32 = 1;
const CG_EVENT_LEFT_MOUSE_UP: u32 = 2;
const CG_EVENT_RIGHT_MOUSE_DOWN: u32 = 3;
const CG_EVENT_RIGHT_MOUSE_UP: u32 = 4;
const CG_EVENT_MOUSE_MOVED: u32 = 5;
const CG_EVENT_LEFT_MOUSE_DRAGGED: u32 = 6;
const CG_EVENT_RIGHT_MOUSE_DRAGGED: u32 = 7;
const CG_EVENT_OTHER_MOUSE_DOWN: u32 = 25;
const CG_EVENT_OTHER_MOUSE_UP: u32 = 26;
const CG_EVENT_OTHER_MOUSE_DRAGGED: u32 = 27;

const CG_MOUSE_BUTTON_LEFT: u32 = 0;
const CG_MOUSE_BUTTON_RIGHT: u32 = 1;
const CG_MOUSE_BUTTON_CENTER: u32 = 2;

const CG_EVENT_FIELD_MOUSE_CLICK_STATE: u32 = 1;
const CG_EVENT_FIELD_MOUSE_BUTTON_NUMBER: u32 = 3;
const CG_EVENT_FIELD_MOUSE_DELTA_X: u32 = 4;
const CG_EVENT_FIELD_MOUSE_DELTA_Y: u32 = 5;
const CG_EVENT_FIELD_SOURCE_USER_DATA: u32 = 42;

const CG_SCROLL_EVENT_UNIT_PIXEL: u32 = 0;
const CG_HID_EVENT_TAP: u32 = 0;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
struct CGPoint {
    x: f64,
    y: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
struct CGSize {
    width: f64,
    height: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
struct CGRect {
    origin: CGPoint,
    size: CGSize,
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(cf: *const c_void);
}

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    /// CoreFoundation `Boolean` (`unsigned char`), not Rust `bool`.
    fn AXIsProcessTrusted() -> u8;
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGGetActiveDisplayList(
        max_displays: u32,
        active_displays: *mut u32,
        display_count: *mut u32,
    ) -> i32;
    fn CGDisplayBounds(display: u32) -> CGRect;
    fn CGWarpMouseCursorPosition(new_cursor_position: CGPoint) -> i32;
    fn CGEventCreate(source: *const c_void) -> *mut c_void;
    fn CGEventGetLocation(event: *const c_void) -> CGPoint;
    fn CGEventCreateKeyboardEvent(
        source: *const c_void,
        virtual_key: u16,
        key_down: bool,
    ) -> *mut c_void;
    fn CGEventCreateMouseEvent(
        source: *const c_void,
        mouse_type: u32,
        mouse_cursor_position: CGPoint,
        mouse_button: u32,
    ) -> *mut c_void;
    fn CGEventCreateScrollWheelEvent(
        source: *const c_void,
        units: u32,
        wheel_count: u32,
        wheel1: i32,
        ...
    ) -> *mut c_void;
    fn CGEventSetFlags(event: *mut c_void, flags: u64);
    fn CGEventSetIntegerValueField(event: *mut c_void, field: u32, value: i64);
    fn CGEventPost(tap: u32, event: *mut c_void);
}

/// Owning handle to a `CGEventRef`; always calls `CFRelease` on drop.
struct CfEvent(*mut c_void);

impl CfEvent {
    fn new(raw: *mut c_void) -> io::Result<Self> {
        if raw.is_null() {
            return Err(io::Error::other("CoreGraphics returned a null event"));
        }
        Ok(Self(raw))
    }

    fn set_flags(&self, flags: u64) {
        // SAFETY: `self.0` is a valid non-null `CGEventRef` owned by `self`.
        unsafe { CGEventSetFlags(self.0, flags) };
    }

    fn set_int(&self, field: u32, value: i64) {
        // SAFETY: `self.0` is a valid non-null `CGEventRef` owned by `self`.
        unsafe { CGEventSetIntegerValueField(self.0, field, value) };
    }

    fn post(&self) {
        self.set_int(CG_EVENT_FIELD_SOURCE_USER_DATA, MICKEY_INPUT);
        // SAFETY: `self.0` is a valid non-null `CGEventRef`; `CGEventPost`
        // copies or retains what it needs synchronously.
        unsafe { CGEventPost(CG_HID_EVENT_TAP, self.0) };
    }
}

impl Drop for CfEvent {
    fn drop(&mut self) {
        // SAFETY: `self.0` is a valid non-null `CGEventRef` created by one of
        // the `CGEventCreate*` functions and not yet released.
        unsafe { CFRelease(self.0.cast()) };
    }
}

/// Whether this process has macOS Accessibility permission, which `CGEventPost`
/// requires to inject keyboard and mouse events into other apps.
pub fn accessibility_trusted() -> bool {
    // SAFETY: takes no arguments; queries TCC for this process.
    unsafe { AXIsProcessTrusted() != 0 }
}

/// Every active display's rectangle in global logical points (the same
/// coordinate space as `CGEventGetLocation` and `CGWarpMouseCursorPosition`).
pub fn monitors() -> io::Result<Vec<Monitor>> {
    let mut count = 0u32;
    // SAFETY: passing null for `active_displays` queries the active display count.
    let err = unsafe { CGGetActiveDisplayList(0, std::ptr::null_mut(), &mut count) };
    if err != 0 {
        return Err(io::Error::other(format!(
            "CGGetActiveDisplayList count failed ({err})"
        )));
    }
    if count == 0 {
        return Ok(Vec::new());
    }
    let mut ids = vec![0u32; count as usize];
    // SAFETY: `ids` has capacity for `count` display IDs and lives across the call.
    let err = unsafe { CGGetActiveDisplayList(count, ids.as_mut_ptr(), &mut count) };
    if err != 0 {
        return Err(io::Error::other(format!(
            "CGGetActiveDisplayList failed ({err})"
        )));
    }
    ids.truncate(count as usize);
    Ok(ids
        .into_iter()
        .map(|id| {
            // SAFETY: `id` is an active `CGDirectDisplayID` returned above.
            let rect = unsafe { CGDisplayBounds(id) };
            Monitor {
                x: rect.origin.x.round() as i32,
                y: rect.origin.y.round() as i32,
                width: rect.size.width.round() as i32,
                height: rect.size.height.round() as i32,
            }
        })
        .collect())
}

/// Current cursor position in global display coordinates.
pub fn cursor_pos() -> Option<(i32, i32)> {
    // SAFETY: passing null creates an empty event carrying the current cursor position.
    let event = CfEvent::new(unsafe { CGEventCreate(std::ptr::null()) }).ok()?;
    // SAFETY: `event.0` is a valid non-null `CGEventRef`.
    let point = unsafe { CGEventGetLocation(event.0) };
    Some((point.x.round() as i32, point.y.round() as i32))
}

/// Moves the cursor to `(x, y)` in global display coordinates.
pub fn set_cursor_pos(x: i32, y: i32) -> io::Result<()> {
    let point = CGPoint {
        x: f64::from(x),
        y: f64::from(y),
    };
    // SAFETY: plain value struct; CoreGraphics clamps coordinates to display bounds.
    let err = unsafe { CGWarpMouseCursorPosition(point) };
    if err != 0 {
        return Err(io::Error::other(format!(
            "CGWarpMouseCursorPosition failed ({err})"
        )));
    }
    Ok(())
}

/// Injects a key press or release with the active modifier flags.
pub fn send_key(virtual_key: u16, pressed: bool, modifiers: u64) -> io::Result<()> {
    // SAFETY: null event source uses the default system source.
    let event = CfEvent::new(unsafe {
        CGEventCreateKeyboardEvent(std::ptr::null(), virtual_key, pressed)
    })?;
    event.set_flags(modifiers);
    event.post();
    Ok(())
}

/// Injects a mouse move (or drag, if a button is held) to `(x, y)` with
/// relative deltas `(dx, dy)`.
pub fn move_mouse(
    x: i32,
    y: i32,
    dx: i32,
    dy: i32,
    held_buttons: u8,
    modifiers: u64,
) -> io::Result<()> {
    let (kind, button, other_button_number) = if held_buttons & 0b0001 != 0 {
        (CG_EVENT_LEFT_MOUSE_DRAGGED, CG_MOUSE_BUTTON_LEFT, None)
    } else if held_buttons & 0b0010 != 0 {
        (CG_EVENT_RIGHT_MOUSE_DRAGGED, CG_MOUSE_BUTTON_RIGHT, None)
    } else if held_buttons & 0b0100 != 0 {
        (
            CG_EVENT_OTHER_MOUSE_DRAGGED,
            CG_MOUSE_BUTTON_CENTER,
            Some(2),
        )
    } else if held_buttons & 0b1000 != 0 {
        (
            CG_EVENT_OTHER_MOUSE_DRAGGED,
            CG_MOUSE_BUTTON_CENTER,
            Some(3),
        )
    } else if held_buttons & 0b1_0000 != 0 {
        (
            CG_EVENT_OTHER_MOUSE_DRAGGED,
            CG_MOUSE_BUTTON_CENTER,
            Some(4),
        )
    } else {
        (CG_EVENT_MOUSE_MOVED, CG_MOUSE_BUTTON_LEFT, None)
    };
    let pos = CGPoint {
        x: f64::from(x),
        y: f64::from(y),
    };
    // SAFETY: null event source and plain value arguments.
    let event =
        CfEvent::new(unsafe { CGEventCreateMouseEvent(std::ptr::null(), kind, pos, button) })?;
    if let Some(num) = other_button_number {
        event.set_int(CG_EVENT_FIELD_MOUSE_BUTTON_NUMBER, num);
    }
    event.set_int(CG_EVENT_FIELD_MOUSE_DELTA_X, i64::from(dx));
    event.set_int(CG_EVENT_FIELD_MOUSE_DELTA_Y, i64::from(dy));
    event.set_flags(modifiers);
    event.post();
    Ok(())
}

/// Injects a mouse button press or release at `(x, y)`.
pub fn send_button(
    x: i32,
    y: i32,
    button: MouseButton,
    pressed: bool,
    click_state: i64,
    modifiers: u64,
) -> io::Result<()> {
    let (kind, cg_button, button_number) = match button {
        MouseButton::Left => (
            if pressed {
                CG_EVENT_LEFT_MOUSE_DOWN
            } else {
                CG_EVENT_LEFT_MOUSE_UP
            },
            CG_MOUSE_BUTTON_LEFT,
            0,
        ),
        MouseButton::Right => (
            if pressed {
                CG_EVENT_RIGHT_MOUSE_DOWN
            } else {
                CG_EVENT_RIGHT_MOUSE_UP
            },
            CG_MOUSE_BUTTON_RIGHT,
            1,
        ),
        MouseButton::Middle => (
            if pressed {
                CG_EVENT_OTHER_MOUSE_DOWN
            } else {
                CG_EVENT_OTHER_MOUSE_UP
            },
            CG_MOUSE_BUTTON_CENTER,
            2,
        ),
        MouseButton::X1 => (
            if pressed {
                CG_EVENT_OTHER_MOUSE_DOWN
            } else {
                CG_EVENT_OTHER_MOUSE_UP
            },
            CG_MOUSE_BUTTON_CENTER,
            3,
        ),
        MouseButton::X2 => (
            if pressed {
                CG_EVENT_OTHER_MOUSE_DOWN
            } else {
                CG_EVENT_OTHER_MOUSE_UP
            },
            CG_MOUSE_BUTTON_CENTER,
            4,
        ),
    };
    let pos = CGPoint {
        x: f64::from(x),
        y: f64::from(y),
    };
    // SAFETY: null event source and plain value arguments.
    let event =
        CfEvent::new(unsafe { CGEventCreateMouseEvent(std::ptr::null(), kind, pos, cg_button) })?;
    if button_number >= 2 {
        event.set_int(CG_EVENT_FIELD_MOUSE_BUTTON_NUMBER, button_number);
    }
    event.set_int(CG_EVENT_FIELD_MOUSE_CLICK_STATE, click_state);
    event.set_flags(modifiers);
    event.post();
    Ok(())
}

/// Injects a pixel-unit scroll wheel event. Positive `vertical` scrolls up;
/// positive `horizontal` scrolls right (negated for CoreGraphics's wheel2).
pub fn scroll(vertical_px: i32, horizontal_px: i32, modifiers: u64) -> io::Result<()> {
    if vertical_px == 0 && horizontal_px == 0 {
        return Ok(());
    }
    // SAFETY: variadic C function called with `wheel_count = 2` followed by
    // exactly two `i32` wheel deltas.
    let event = CfEvent::new(unsafe {
        CGEventCreateScrollWheelEvent(
            std::ptr::null(),
            CG_SCROLL_EVENT_UNIT_PIXEL,
            2,
            vertical_px,
            -horizontal_px,
        )
    })?;
    event.set_flags(modifiers);
    event.post();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "reads real macOS displays; run by hand on a desktop session"]
    fn active_displays_and_cursor_are_readable() {
        let displays = monitors().unwrap();
        assert!(!displays.is_empty());
        assert!(cursor_pos().is_some());
    }
}
