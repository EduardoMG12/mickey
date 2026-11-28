//! macOS input capture via `CGEventTap` on a dedicated `CFRunLoop` thread.
//!
//! While the focus is local, events pass straight through to macOS; while the
//! focus is on another PC (`SWALLOW == true`), the tap returns null to suppress
//! them and `CGAssociateMouseAndMouseCursorPosition(false)` freezes the local
//! cursor while CoreGraphics continues reporting relative mouse deltas.
//!
//! Input safety (CLAUDE.md hard rule 8):
//! - `Grabbed::drop` always calls `set_swallow(false, landing)`, which
//!   re-associates the mouse with the cursor and stops swallowing.
//! - `release_grab()` is also invoked from `signals::on_stop` on SIGINT/SIGTERM.
//! - macOS automatically tears down the tap and re-associates the mouse if the
//!   process exits abnormally.

use crate::platform::macos::{self, MICKEY_INPUT};
use std::ffi::c_void;
use std::io;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicPtr, AtomicU8, AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};

const CG_HID_EVENT_TAP: u32 = 0;
const CG_HEAD_INSERT_EVENT_TAP: u32 = 0;
const CG_EVENT_TAP_OPTION_DEFAULT: u32 = 0;

const CG_EVENT_LEFT_MOUSE_DOWN: u32 = 1;
const CG_EVENT_LEFT_MOUSE_UP: u32 = 2;
const CG_EVENT_RIGHT_MOUSE_DOWN: u32 = 3;
const CG_EVENT_RIGHT_MOUSE_UP: u32 = 4;
const CG_EVENT_MOUSE_MOVED: u32 = 5;
const CG_EVENT_LEFT_MOUSE_DRAGGED: u32 = 6;
const CG_EVENT_RIGHT_MOUSE_DRAGGED: u32 = 7;
const CG_EVENT_KEY_DOWN: u32 = 10;
const CG_EVENT_KEY_UP: u32 = 11;
const CG_EVENT_FLAGS_CHANGED: u32 = 12;
const CG_EVENT_SCROLL_WHEEL: u32 = 22;
const CG_EVENT_OTHER_MOUSE_DOWN: u32 = 25;
const CG_EVENT_OTHER_MOUSE_UP: u32 = 26;
const CG_EVENT_OTHER_MOUSE_DRAGGED: u32 = 27;
const CG_EVENT_TAP_DISABLED_BY_TIMEOUT: u32 = 0xFFFF_FFFE;
const CG_EVENT_TAP_DISABLED_BY_USER_INPUT: u32 = 0xFFFF_FFFF;

const CG_EVENT_FIELD_MOUSE_BUTTON_NUMBER: u32 = 3;
const CG_EVENT_FIELD_MOUSE_DELTA_X: u32 = 4;
const CG_EVENT_FIELD_MOUSE_DELTA_Y: u32 = 5;
const CG_EVENT_FIELD_KEYBOARD_KEYCODE: u32 = 9;
const CG_EVENT_FIELD_SCROLL_DELTA_AXIS1: u32 = 11;
const CG_EVENT_FIELD_SCROLL_DELTA_AXIS2: u32 = 12;
const CG_EVENT_FIELD_SOURCE_USER_DATA: u32 = 42;
const CG_EVENT_FIELD_SCROLL_IS_CONTINUOUS: u32 = 88;
const CG_EVENT_FIELD_SCROLL_POINT_DELTA_AXIS1: u32 = 96;
const CG_EVENT_FIELD_SCROLL_POINT_DELTA_AXIS2: u32 = 97;

type CGEventTapCallBack =
    unsafe extern "C" fn(*mut c_void, u32, *mut c_void, *mut c_void) -> *mut c_void;

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    static kCFRunLoopCommonModes: *const c_void;
    fn CFMachPortCreateRunLoopSource(
        allocator: *const c_void,
        port: *mut c_void,
        order: isize,
    ) -> *mut c_void;
    fn CFRunLoopGetCurrent() -> *mut c_void;
    fn CFRunLoopAddSource(rl: *mut c_void, source: *mut c_void, mode: *const c_void);
    fn CFRunLoopRun();
    fn CFRelease(cf: *const c_void);
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CGPoint {
    x: f64,
    y: f64,
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGPreflightListenEventAccess() -> bool;
    fn CGEventTapCreate(
        tap: u32,
        place: u32,
        options: u32,
        events_of_interest: u64,
        callback: CGEventTapCallBack,
        user_info: *mut c_void,
    ) -> *mut c_void;
    fn CGEventTapEnable(tap: *mut c_void, enable: bool);
    fn CGEventGetFlags(event: *const c_void) -> u64;
    fn CGEventGetLocation(event: *const c_void) -> CGPoint;
    fn CGEventGetIntegerValueField(event: *const c_void, field: u32) -> i64;
    // `boolean_t` in `<CoreGraphics/CGRemoteOperation.h>` is a 32-bit unsigned int.
    fn CGAssociateMouseAndMouseCursorPosition(connected: u32) -> i32;
}

/// Raw events reported by the `CGEventTap` before evdev translation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TapEvent {
    Key {
        vk: u16,
        pressed: bool,
    },
    /// 0 = left, 1 = right, 2 = middle, 3 = X1 (back), 4 = X2 (forward).
    Button {
        button: u8,
        pressed: bool,
    },
    Delta {
        dx: i32,
        dy: i32,
    },
    /// Hi-res wheel units (120 per notch); positive is up / right.
    Wheel {
        vertical: i32,
        horizontal: i32,
    },
}

/// Whether the tap is currently suppressing events (focus is on another PC).
static SWALLOW: AtomicBool = AtomicBool::new(false);
/// Cursor anchor while grabbed: if macOS's WindowServer (notably the built-in
/// trackpad) still advances the cursor after disassociation, the tap warps it
/// back here after reading the hardware delta.
static HAS_ANCHOR: AtomicBool = AtomicBool::new(false);
static ANCHOR_X: AtomicI32 = AtomicI32::new(0);
static ANCHOR_Y: AtomicI32 = AtomicI32::new(0);
/// Lock-free bitsets of currently held macOS virtual key codes (`0..128`).
static HELD_KEYS_LO: AtomicU64 = AtomicU64::new(0);
static HELD_KEYS_HI: AtomicU64 = AtomicU64::new(0);
/// Lock-free bitmask of currently held mouse buttons (`0..5`).
static HELD_BUTTONS: AtomicU8 = AtomicU8::new(0);

/// State owned by the `CFRunLoop` thread and passed via `user_info`.
struct TapContext {
    sender: Sender<TapEvent>,
    tap: AtomicPtr<c_void>,
}

/// Whether this process has macOS Input Monitoring permission.
pub fn input_monitoring_trusted() -> bool {
    // SAFETY: takes no arguments; queries TCC for this process.
    unsafe { CGPreflightListenEventAccess() }
}

/// Whether any key is currently held down.
pub fn any_key_down() -> bool {
    HELD_KEYS_LO.load(Ordering::Relaxed) != 0 || HELD_KEYS_HI.load(Ordering::Relaxed) != 0
}

/// Whether any mouse button is currently held down.
pub fn button_down() -> bool {
    HELD_BUTTONS.load(Ordering::Relaxed) != 0
}

/// Turns event suppression on or off. When leaving (`on == true`), disconnects
/// the mouse from the local cursor and anchors it so it stays frozen while
/// still producing relative deltas. When returning (`on == false`), optionally
/// places the cursor at `landing` before reconnecting the mouse.
pub fn set_swallow(on: bool, landing: Option<(i32, i32)>) {
    if on {
        let anchor = macos::cursor_pos();
        if let Some((x, y)) = anchor {
            ANCHOR_X.store(x, Ordering::Relaxed);
            ANCHOR_Y.store(y, Ordering::Relaxed);
            HAS_ANCHOR.store(true, Ordering::Release);
        } else {
            HAS_ANCHOR.store(false, Ordering::Release);
        }
        SWALLOW.store(true, Ordering::SeqCst);
        // SAFETY: plain `boolean_t` (`0` = false) argument; freezes local cursor
        // movement while keeping hardware deltas flowing into our CGEventTap.
        let err = unsafe { CGAssociateMouseAndMouseCursorPosition(0) };
        if err != 0 {
            eprintln!("mickey: CGAssociateMouseAndMouseCursorPosition(false) failed ({err})");
        }
        if let Some((x, y)) = anchor {
            let _ = macos::set_cursor_pos(x, y);
        }
    } else {
        HAS_ANCHOR.store(false, Ordering::Release);
        if let Some((x, y)) = landing {
            let _ = macos::set_cursor_pos(x, y);
        }
        // SAFETY: plain `boolean_t` (`1` = true) argument; restores normal cursor movement.
        let err = unsafe { CGAssociateMouseAndMouseCursorPosition(1) };
        if err != 0 {
            eprintln!("mickey: CGAssociateMouseAndMouseCursorPosition(true) failed ({err})");
        }
        SWALLOW.store(false, Ordering::SeqCst);
    }
}

/// Emergency release for signal handlers (`SIGINT`/`SIGTERM`).
pub fn release_grab() {
    HAS_ANCHOR.store(false, Ordering::Release);
    if SWALLOW.swap(false, Ordering::SeqCst) {
        // SAFETY: plain `boolean_t` (`1` = true) argument; restores normal cursor movement.
        let _ = unsafe { CGAssociateMouseAndMouseCursorPosition(1) };
    }
}

/// Starts the `CGEventTap` on a dedicated `CFRunLoop` thread and waits until
/// the tap is installed or fails.
pub fn start(events: Sender<TapEvent>) -> io::Result<()> {
    let (ready_tx, ready_rx) = mpsc::channel::<io::Result<()>>();
    crate::threads::spawn("macos event tap", move || {
        let context = Box::new(TapContext {
            sender: events,
            tap: AtomicPtr::new(std::ptr::null_mut()),
        });
        let ctx_ptr = Box::into_raw(context);
        let mask = event_mask();
        // SAFETY: `ctx_ptr` remains valid for the entire lifetime of the
        // `CFRunLoop` on this thread; `tap_callback` matches `CGEventTapCallBack`.
        let tap = unsafe {
            CGEventTapCreate(
                CG_HID_EVENT_TAP,
                CG_HEAD_INSERT_EVENT_TAP,
                CG_EVENT_TAP_OPTION_DEFAULT,
                mask,
                tap_callback,
                ctx_ptr.cast(),
            )
        };
        if tap.is_null() {
            // SAFETY: `tap` was not created, so no callback can ever read
            // `ctx_ptr`; reclaim the allocation.
            drop(unsafe { Box::from_raw(ctx_ptr) });
            let _ = ready_tx.send(Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "CGEventTapCreate returned null; grant Accessibility and Input Monitoring \
                 permissions in System Settings > Privacy & Security",
            )));
            return;
        }
        // SAFETY: `ctx_ptr` is exclusively owned by this thread until `CFRunLoopRun`.
        unsafe { (*ctx_ptr).tap.store(tap, Ordering::Release) };
        // SAFETY: `tap` is a valid `CFMachPortRef`.
        let source = unsafe { CFMachPortCreateRunLoopSource(std::ptr::null(), tap, 0) };
        if source.is_null() {
            // SAFETY: `tap` is valid and has not been released yet; `ctx_ptr`
            // was not scheduled on any run loop.
            unsafe {
                CFRelease(tap.cast());
                drop(Box::from_raw(ctx_ptr));
            }
            let _ = ready_tx.send(Err(io::Error::other(
                "CFMachPortCreateRunLoopSource returned null",
            )));
            return;
        }
        // SAFETY: `CFRunLoopGetCurrent`, `source`, `kCFRunLoopCommonModes`, and
        // `tap` are valid CoreFoundation objects on this thread.
        unsafe {
            let run_loop = CFRunLoopGetCurrent();
            CFRunLoopAddSource(run_loop, source, kCFRunLoopCommonModes);
            CGEventTapEnable(tap, true);
        }
        let _ = ready_tx.send(Ok(()));
        // SAFETY: blocks on this thread's run loop until the process exits.
        unsafe {
            CFRunLoopRun();
            CFRelease(source.cast());
            CFRelease(tap.cast());
            drop(Box::from_raw(ctx_ptr));
        }
    });
    ready_rx
        .recv()
        .unwrap_or_else(|_| Err(io::Error::other("macOS event tap thread exited early")))
}

const fn event_mask() -> u64 {
    (1 << CG_EVENT_LEFT_MOUSE_DOWN)
        | (1 << CG_EVENT_LEFT_MOUSE_UP)
        | (1 << CG_EVENT_RIGHT_MOUSE_DOWN)
        | (1 << CG_EVENT_RIGHT_MOUSE_UP)
        | (1 << CG_EVENT_MOUSE_MOVED)
        | (1 << CG_EVENT_LEFT_MOUSE_DRAGGED)
        | (1 << CG_EVENT_RIGHT_MOUSE_DRAGGED)
        | (1 << CG_EVENT_KEY_DOWN)
        | (1 << CG_EVENT_KEY_UP)
        | (1 << CG_EVENT_FLAGS_CHANGED)
        | (1 << CG_EVENT_SCROLL_WHEEL)
        | (1 << CG_EVENT_OTHER_MOUSE_DOWN)
        | (1 << CG_EVENT_OTHER_MOUSE_UP)
        | (1 << CG_EVENT_OTHER_MOUSE_DRAGGED)
}

unsafe extern "C" fn tap_callback(
    _proxy: *mut c_void,
    kind: u32,
    event: *mut c_void,
    user_info: *mut c_void,
) -> *mut c_void {
    if user_info.is_null() {
        return event;
    }
    // SAFETY: `user_info` is the `TapContext` allocated in `start` and kept
    // alive for the entire duration of `CFRunLoopRun`.
    let ctx = unsafe { &*(user_info.cast::<TapContext>()) };

    if kind == CG_EVENT_TAP_DISABLED_BY_TIMEOUT || kind == CG_EVENT_TAP_DISABLED_BY_USER_INPUT {
        let tap = ctx.tap.load(Ordering::Acquire);
        if !tap.is_null() {
            // SAFETY: `tap` is our live `CFMachPortRef`.
            unsafe { CGEventTapEnable(tap, true) };
        }
        return event;
    }

    if event.is_null() {
        return event;
    }

    // Never capture or suppress events injected by Mickey's own target side.
    // SAFETY: `event` is a valid non-null `CGEventRef` for this callback.
    if unsafe { CGEventGetIntegerValueField(event, CG_EVENT_FIELD_SOURCE_USER_DATA) }
        == MICKEY_INPUT
    {
        return event;
    }

    let translated = match kind {
        CG_EVENT_KEY_DOWN | CG_EVENT_KEY_UP => {
            // SAFETY: `event` is a valid keyboard `CGEventRef`.
            let vk = unsafe { CGEventGetIntegerValueField(event, CG_EVENT_FIELD_KEYBOARD_KEYCODE) };
            let pressed = kind == CG_EVENT_KEY_DOWN;
            if let Ok(vk) = u16::try_from(vk) {
                record_key(vk, pressed);
                Some(TapEvent::Key { vk, pressed })
            } else {
                None
            }
        }
        CG_EVENT_FLAGS_CHANGED => {
            // SAFETY: `event` is a valid flags-changed `CGEventRef`.
            let vk = unsafe { CGEventGetIntegerValueField(event, CG_EVENT_FIELD_KEYBOARD_KEYCODE) };
            // SAFETY: `event` is a valid `CGEventRef`.
            let flags = unsafe { CGEventGetFlags(event) };
            u16::try_from(vk).ok().and_then(|vk| {
                modifier_pressed(vk, flags).map(|pressed| {
                    record_key(vk, pressed);
                    TapEvent::Key { vk, pressed }
                })
            })
        }
        CG_EVENT_LEFT_MOUSE_DOWN => {
            record_button(0, true);
            Some(TapEvent::Button {
                button: 0,
                pressed: true,
            })
        }
        CG_EVENT_LEFT_MOUSE_UP => {
            record_button(0, false);
            Some(TapEvent::Button {
                button: 0,
                pressed: false,
            })
        }
        CG_EVENT_RIGHT_MOUSE_DOWN => {
            record_button(1, true);
            Some(TapEvent::Button {
                button: 1,
                pressed: true,
            })
        }
        CG_EVENT_RIGHT_MOUSE_UP => {
            record_button(1, false);
            Some(TapEvent::Button {
                button: 1,
                pressed: false,
            })
        }
        CG_EVENT_OTHER_MOUSE_DOWN | CG_EVENT_OTHER_MOUSE_UP => {
            // SAFETY: `event` is a valid mouse `CGEventRef`.
            let num =
                unsafe { CGEventGetIntegerValueField(event, CG_EVENT_FIELD_MOUSE_BUTTON_NUMBER) };
            let pressed = kind == CG_EVENT_OTHER_MOUSE_DOWN;
            u8::try_from(num).ok().filter(|&b| b < 5).map(|button| {
                record_button(button, pressed);
                TapEvent::Button { button, pressed }
            })
        }
        CG_EVENT_MOUSE_MOVED
        | CG_EVENT_LEFT_MOUSE_DRAGGED
        | CG_EVENT_RIGHT_MOUSE_DRAGGED
        | CG_EVENT_OTHER_MOUSE_DRAGGED => {
            // SAFETY: `event` is a valid mouse `CGEventRef`.
            let dx = unsafe { CGEventGetIntegerValueField(event, CG_EVENT_FIELD_MOUSE_DELTA_X) };
            // SAFETY: `event` is a valid mouse `CGEventRef`.
            let dy = unsafe { CGEventGetIntegerValueField(event, CG_EVENT_FIELD_MOUSE_DELTA_Y) };
            if SWALLOW.load(Ordering::Acquire) && HAS_ANCHOR.load(Ordering::Acquire) {
                // SAFETY: `event` is a valid mouse `CGEventRef`.
                let pos = unsafe { CGEventGetLocation(event) };
                let (ax, ay) = (
                    ANCHOR_X.load(Ordering::Relaxed),
                    ANCHOR_Y.load(Ordering::Relaxed),
                );
                if pos.x.round() as i32 != ax || pos.y.round() as i32 != ay {
                    let _ = macos::set_cursor_pos(ax, ay);
                }
            }
            let (dx, dy) = (clamp_i32(dx), clamp_i32(dy));
            (dx != 0 || dy != 0).then_some(TapEvent::Delta { dx, dy })
        }
        CG_EVENT_SCROLL_WHEEL => {
            // SAFETY: `event` is a valid scroll `CGEventRef`.
            let continuous =
                unsafe { CGEventGetIntegerValueField(event, CG_EVENT_FIELD_SCROLL_IS_CONTINUOUS) }
                    != 0;
            let (vertical, horizontal) = if continuous {
                // SAFETY: `event` is a valid continuous scroll `CGEventRef`.
                let dy = clamp_i32(unsafe {
                    CGEventGetIntegerValueField(event, CG_EVENT_FIELD_SCROLL_POINT_DELTA_AXIS1)
                });
                // SAFETY: `event` is a valid continuous scroll `CGEventRef`.
                let dx = clamp_i32(unsafe {
                    CGEventGetIntegerValueField(event, CG_EVENT_FIELD_SCROLL_POINT_DELTA_AXIS2)
                });
                (
                    scale_continuous_scroll(dy),
                    scale_continuous_scroll(dx.saturating_neg()),
                )
            } else {
                // SAFETY: `event` is a valid discrete scroll `CGEventRef`.
                let dy = clamp_i32(unsafe {
                    CGEventGetIntegerValueField(event, CG_EVENT_FIELD_SCROLL_DELTA_AXIS1)
                });
                // SAFETY: `event` is a valid discrete scroll `CGEventRef`.
                let dx = clamp_i32(unsafe {
                    CGEventGetIntegerValueField(event, CG_EVENT_FIELD_SCROLL_DELTA_AXIS2)
                });
                (
                    dy.saturating_mul(120),
                    dx.saturating_neg().saturating_mul(120),
                )
            };
            (vertical != 0 || horizontal != 0).then_some(TapEvent::Wheel {
                vertical,
                horizontal,
            })
        }
        _ => None,
    };

    if let Some(ev) = translated {
        let _ = ctx.sender.send(ev);
    }

    if SWALLOW.load(Ordering::Acquire) {
        std::ptr::null_mut()
    } else {
        event
    }
}

fn record_key(vk: u16, pressed: bool) {
    if vk < 64 {
        let bit = 1u64 << vk;
        if pressed {
            HELD_KEYS_LO.fetch_or(bit, Ordering::Relaxed);
        } else {
            HELD_KEYS_LO.fetch_and(!bit, Ordering::Relaxed);
        }
    } else if vk < 128 {
        let bit = 1u64 << (vk - 64);
        if pressed {
            HELD_KEYS_HI.fetch_or(bit, Ordering::Relaxed);
        } else {
            HELD_KEYS_HI.fetch_and(!bit, Ordering::Relaxed);
        }
    }
}

fn record_button(button: u8, pressed: bool) {
    if button < 5 {
        let bit = 1u8 << button;
        if pressed {
            HELD_BUTTONS.fetch_or(bit, Ordering::Relaxed);
        } else {
            HELD_BUTTONS.fetch_and(!bit, Ordering::Relaxed);
        }
    }
}

fn clamp_i32(value: i64) -> i32 {
    value.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

/// Converts macOS continuous (trackpad/Magic Mouse) pixel deltas into Mickey's
/// hi-res wheel units (120 per notch, ~36 px per notch).
fn scale_continuous_scroll(px: i32) -> i32 {
    if px == 0 {
        return 0;
    }
    let scaled = px.saturating_mul(120) / 36;
    if scaled == 0 { px.signum() } else { scaled }
}

/// Decodes `kCGEventFlagsChanged` into pressed/released for a modifier key
/// using macOS's device-dependent left/right modifier bits (`IOLLEvent.h`).
fn modifier_pressed(vk: u16, flags: u64) -> Option<bool> {
    let mask = match vk {
        0x38 => 0x0000_0002, // kVK_Shift (Left Shift)
        0x3C => 0x0000_0004, // kVK_RightShift
        0x3B => 0x0000_0001, // kVK_Control (Left Control)
        0x3E => 0x0000_2000, // kVK_RightControl
        0x3A => 0x0000_0020, // kVK_Option (Left Option)
        0x3D => 0x0000_0040, // kVK_RightOption (AltGr)
        0x37 => 0x0000_0008, // kVK_Command (Left Command)
        0x36 => 0x0000_0010, // kVK_RightCommand
        0x39 => 0x0001_0000, // kVK_CapsLock
        _ => return None,
    };
    Some(flags & mask != 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modifier_flags_distinguish_left_and_right_sides() {
        // Right Shift pressed, Left Shift released.
        assert_eq!(modifier_pressed(0x3C, 0x0002_0004), Some(true));
        assert_eq!(modifier_pressed(0x38, 0x0002_0004), Some(false));
        // Right Option (AltGr) pressed, Left Option released.
        assert_eq!(modifier_pressed(0x3D, 0x0008_0040), Some(true));
        assert_eq!(modifier_pressed(0x3A, 0x0008_0040), Some(false));
        // Non-modifier keycode returns None.
        assert_eq!(modifier_pressed(0x00, 0), None);
    }

    #[test]
    fn continuous_scroll_preserves_direction_and_small_moves() {
        assert_eq!(scale_continuous_scroll(0), 0);
        assert_eq!(scale_continuous_scroll(36), 120);
        assert_eq!(scale_continuous_scroll(-36), -120);
        assert_eq!(scale_continuous_scroll(1), 3);
        assert_eq!(scale_continuous_scroll(-1), -3);
    }
}
