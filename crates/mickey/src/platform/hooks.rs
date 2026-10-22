//! Low-level keyboard and mouse hooks (PRD 0008): Windows calls them for
//! every input event before any program sees it. While the focus is on
//! another PC they *swallow* the event (return non-zero), which is the
//! Windows version of a grab.
//!
//! The way home: Windows skips a hook that does not answer in time
//! (`LowLevelHooksTimeout`), never routes Ctrl+Alt+Del through hooks, and
//! removes them when the process ends, however it ends.

use crate::keymap::ScanCode;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Mutex, OnceLock};
use std::thread;

const WH_KEYBOARD_LL: i32 = 13;
const WH_MOUSE_LL: i32 = 14;
const HC_ACTION: i32 = 0;
const WM_KEYDOWN: usize = 0x0100;
const WM_SYSKEYDOWN: usize = 0x0104;
const WM_MOUSEMOVE: usize = 0x0200;
const WM_LBUTTONDOWN: usize = 0x0201;
const WM_LBUTTONUP: usize = 0x0202;
const WM_RBUTTONDOWN: usize = 0x0204;
const WM_RBUTTONUP: usize = 0x0205;
const WM_MBUTTONDOWN: usize = 0x0207;
const WM_MBUTTONUP: usize = 0x0208;
const WM_MOUSEWHEEL: usize = 0x020A;
const WM_XBUTTONDOWN: usize = 0x020B;
const WM_XBUTTONUP: usize = 0x020C;
const WM_MOUSEHWHEEL: usize = 0x020E;
const LLKHF_EXTENDED: u32 = 0x01;
const LLKHF_INJECTED: u32 = 0x10;
const LLMHF_INJECTED: u32 = 0x01;

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

/// `KBDLLHOOKSTRUCT`.
#[repr(C)]
struct KeyboardInfo {
    vk_code: u32,
    scan_code: u32,
    flags: u32,
    time: u32,
    extra_info: usize,
}

/// `MSLLHOOKSTRUCT`.
#[repr(C)]
struct MouseInfo {
    pt: Point,
    mouse_data: u32,
    flags: u32,
    time: u32,
    extra_info: usize,
}

/// `MSG`, only so `GetMessageW` has somewhere to write.
#[repr(C)]
struct Msg {
    hwnd: isize,
    message: u32,
    wparam: usize,
    lparam: isize,
    time: u32,
    pt: Point,
    private: u32,
}

type HookProc = unsafe extern "system" fn(i32, usize, isize) -> isize;

#[link(name = "user32")]
unsafe extern "system" {
    fn SetWindowsHookExW(id: i32, proc: HookProc, module: isize, thread: u32) -> isize;
    fn CallNextHookEx(hook: isize, code: i32, wparam: usize, lparam: isize) -> isize;
    fn GetMessageW(msg: *mut Msg, hwnd: isize, min: u32, max: u32) -> i32;
    fn GetCursorPos(point: *mut Point) -> i32;
    fn GetAsyncKeyState(vk: i32) -> i16;
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetModuleHandleW(name: *const u16) -> isize;
}

/// What the hooks saw, before any translation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookEvent {
    Key {
        scan: ScanCode,
        pressed: bool,
    },
    /// 0 left, 1 right, 2 middle, 3 back (X1), 4 forward (X2).
    Button {
        button: u8,
        pressed: bool,
    },
    /// Where the cursor would go, in screen pixels.
    Move {
        x: i32,
        y: i32,
    },
    /// 120 per notch; positive is up or right.
    Wheel {
        vertical: i32,
        horizontal: i32,
    },
}

struct Shared {
    events: Mutex<Sender<HookEvent>>,
    swallow: AtomicBool,
}

/// The hook procedures are plain functions Windows calls, so they reach
/// their state through this. Set once by `start`.
static SHARED: OnceLock<Shared> = OnceLock::new();

/// Installs both hooks on their own thread, which runs the message loop
/// Windows needs to call them. Events go to `events`.
pub fn start(events: Sender<HookEvent>) -> io::Result<()> {
    let shared = Shared {
        events: Mutex::new(events),
        swallow: AtomicBool::new(false),
    };
    SHARED
        .set(shared)
        .map_err(|_| io::Error::other("the hooks are already running"))?;
    let (ready, installed) = mpsc::channel();
    thread::spawn(move || {
        // SAFETY: a null name asks for this program's own module handle.
        let module = unsafe { GetModuleHandleW(std::ptr::null()) };
        // SAFETY: both procedures have the `HookProc` signature and live as
        // long as the program; `module` is this program; thread 0 means
        // every thread on this desktop, as low-level hooks require.
        let keyboard = unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, keyboard_proc, module, 0) };
        // SAFETY: as above.
        let mouse = unsafe { SetWindowsHookExW(WH_MOUSE_LL, mouse_proc, module, 0) };
        if keyboard == 0 || mouse == 0 {
            let _ = ready.send(Err(io::Error::last_os_error()));
            return;
        }
        let _ = ready.send(Ok(()));
        let mut msg = Msg {
            hwnd: 0,
            message: 0,
            wparam: 0,
            lparam: 0,
            time: 0,
            pt: Point::default(),
            private: 0,
        };
        // SAFETY: `msg` is a valid MSG for every call; GetMessageW only
        // returns when a message arrives, which keeps the hooks alive.
        while unsafe { GetMessageW(&mut msg, 0, 0, 0) } > 0 {}
    });
    installed
        .recv()
        .map_err(|_| io::Error::other("the hook thread stopped"))?
}

/// While `true`, every hooked event is swallowed.
pub fn set_swallow(on: bool) {
    if let Some(shared) = SHARED.get() {
        shared.swallow.store(on, Ordering::SeqCst);
    }
}

pub fn cursor_pos() -> Point {
    let mut point = Point::default();
    // SAFETY: `point` is a valid POINT for the call to fill.
    unsafe { GetCursorPos(&mut point) };
    point
}

/// Whether any key or mouse button is down right now.
pub fn any_key_down() -> bool {
    // SAFETY: GetAsyncKeyState takes any virtual-key code and only reads
    // the global key state; the top bit is set while the key is down.
    (1..=0xFE).any(|vk| unsafe { GetAsyncKeyState(vk) } < 0)
}

/// Sends the event and says whether to swallow it. Must be quick: Windows
/// skips a hook that takes too long.
fn report(event: HookEvent) -> bool {
    let Some(shared) = SHARED.get() else {
        return false;
    };
    if let Ok(events) = shared.events.lock() {
        let _ = events.send(event);
    }
    shared.swallow.load(Ordering::SeqCst)
}

unsafe extern "system" fn keyboard_proc(code: i32, wparam: usize, lparam: isize) -> isize {
    if code == HC_ACTION {
        // SAFETY: for HC_ACTION, `lparam` points to a KBDLLHOOKSTRUCT that
        // stays valid until this procedure returns.
        let info = unsafe { &*(lparam as *const KeyboardInfo) };
        // Injected events (ours from SendInput, or other programs') pass.
        if info.flags & LLKHF_INJECTED == 0 {
            let scan = ScanCode {
                code: info.scan_code as u8,
                extended: info.flags & LLKHF_EXTENDED != 0,
            };
            let pressed = wparam == WM_KEYDOWN || wparam == WM_SYSKEYDOWN;
            if report(HookEvent::Key { scan, pressed }) {
                return 1;
            }
        }
    }
    // SAFETY: passes the same arguments on, as the hook contract requires.
    unsafe { CallNextHookEx(0, code, wparam, lparam) }
}

unsafe extern "system" fn mouse_proc(code: i32, wparam: usize, lparam: isize) -> isize {
    if code == HC_ACTION {
        // SAFETY: for HC_ACTION, `lparam` points to an MSLLHOOKSTRUCT that
        // stays valid until this procedure returns.
        let info = unsafe { &*(lparam as *const MouseInfo) };
        if info.flags & LLMHF_INJECTED == 0
            && let Some(event) = mouse_event(wparam, info)
            && report(event)
        {
            return 1;
        }
    }
    // SAFETY: passes the same arguments on, as the hook contract requires.
    unsafe { CallNextHookEx(0, code, wparam, lparam) }
}

fn mouse_event(message: usize, info: &MouseInfo) -> Option<HookEvent> {
    // The wheel amount and the X button number live in the high word.
    let high = (info.mouse_data >> 16) as u16;
    let button = |button, pressed| Some(HookEvent::Button { button, pressed });
    match message {
        WM_MOUSEMOVE => Some(HookEvent::Move {
            x: info.pt.x,
            y: info.pt.y,
        }),
        WM_LBUTTONDOWN => button(0, true),
        WM_LBUTTONUP => button(0, false),
        WM_RBUTTONDOWN => button(1, true),
        WM_RBUTTONUP => button(1, false),
        WM_MBUTTONDOWN => button(2, true),
        WM_MBUTTONUP => button(2, false),
        WM_XBUTTONDOWN => button(2 + high as u8, true),
        WM_XBUTTONUP => button(2 + high as u8, false),
        WM_MOUSEWHEEL => Some(HookEvent::Wheel {
            vertical: i32::from(high as i16),
            horizontal: 0,
        }),
        WM_MOUSEHWHEEL => Some(HookEvent::Wheel {
            vertical: 0,
            horizontal: i32::from(high as i16),
        }),
        _ => None,
    }
}
