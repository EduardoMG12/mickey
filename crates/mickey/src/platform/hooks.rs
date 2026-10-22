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
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Mutex, OnceLock};
use std::thread;

const WH_KEYBOARD_LL: i32 = 13;
/// Thread messages that turn the mouse hook on and off (`WM_APP` + n).
const MOUSE_ON: u32 = 0x8000 + 1;
const MOUSE_OFF: u32 = 0x8000 + 2;
const PM_NOREMOVE: u32 = 0;
const THREAD_PRIORITY_TIME_CRITICAL: i32 = 15;
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
/// Set on the Left Ctrl that Windows invents when AltGr (Right Alt on
/// layouts that have AltGr, such as US-International) is pressed.
const FAKE_CTRL_SCAN: u32 = 0x200;
const SCAN_RIGHT_SHIFT: u32 = 0x36;
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
    fn PeekMessageW(msg: *mut Msg, hwnd: isize, min: u32, max: u32, remove: u32) -> i32;
    fn PostThreadMessageW(thread: u32, message: u32, wparam: usize, lparam: isize) -> i32;
    fn UnhookWindowsHookEx(hook: isize) -> i32;
    fn GetCursorPos(point: *mut Point) -> i32;
    fn GetAsyncKeyState(vk: i32) -> i16;
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetModuleHandleW(name: *const u16) -> isize;
    fn GetCurrentThreadId() -> u32;
    fn GetCurrentThread() -> isize;
    fn SetThreadPriority(thread: isize, priority: i32) -> i32;
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
    /// The hook thread, which installs and removes the mouse hook.
    thread: AtomicU32,
}

/// The hook procedures are plain functions Windows calls, so they reach
/// their state through this. Set once by `start`.
static SHARED: OnceLock<Shared> = OnceLock::new();

/// Installs the keyboard hook on its own thread, which runs the message
/// loop Windows needs to call hooks. The mouse hook is only installed while
/// swallowing: a game sends thousands of mouse events a second, and none of
/// them should pass through Mickey while the focus is local.
/// Events go to `events`.
pub fn start(events: Sender<HookEvent>) -> io::Result<()> {
    let shared = Shared {
        events: Mutex::new(events),
        swallow: AtomicBool::new(false),
        thread: AtomicU32::new(0),
    };
    SHARED
        .set(shared)
        .map_err(|_| io::Error::other("the hooks are already running"))?;
    let (ready, installed) = mpsc::channel();
    thread::spawn(move || {
        // Windows waits for this thread on every key and mouse event, and
        // skips the hook if it is late: under load (a game) a late hook
        // means input lag locally and lost key releases on the target. The
        // thread only ever runs for a moment per event, so it may go first.
        // SAFETY: GetCurrentThread returns a pseudo-handle valid for this
        // call; raising our own thread's priority has no other effect.
        unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_TIME_CRITICAL) };
        // SAFETY: a null name asks for this program's own module handle.
        let module = unsafe { GetModuleHandleW(std::ptr::null()) };
        // SAFETY: both procedures have the `HookProc` signature and live as
        // long as the program; `module` is this program; thread 0 means
        // every thread on this desktop, as low-level hooks require.
        let keyboard = unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, keyboard_proc, module, 0) };
        if keyboard == 0 {
            let _ = ready.send(Err(io::Error::last_os_error()));
            return;
        }
        let mut msg = Msg {
            hwnd: 0,
            message: 0,
            wparam: 0,
            lparam: 0,
            time: 0,
            pt: Point::default(),
            private: 0,
        };
        // A thread gets a message queue on its first message call; create
        // it now, so `set_swallow` can post to it right away.
        // SAFETY: `msg` is a valid MSG; nothing is removed from the queue.
        unsafe { PeekMessageW(&mut msg, 0, 0, 0, PM_NOREMOVE) };
        if let Some(shared) = SHARED.get() {
            // SAFETY: no arguments; returns this thread's id.
            shared
                .thread
                .store(unsafe { GetCurrentThreadId() }, Ordering::SeqCst);
        }
        let _ = ready.send(Ok(()));
        let mut mouse = 0;
        // SAFETY: `msg` is a valid MSG for every call; GetMessageW only
        // returns when a message arrives, which keeps the hooks alive.
        while unsafe { GetMessageW(&mut msg, 0, 0, 0) } > 0 {
            match msg.message {
                MOUSE_ON if mouse == 0 => {
                    // SAFETY: as for the keyboard hook above.
                    mouse = unsafe { SetWindowsHookExW(WH_MOUSE_LL, mouse_proc, module, 0) };
                }
                MOUSE_OFF if mouse != 0 => {
                    // SAFETY: `mouse` is the hook this thread installed.
                    unsafe { UnhookWindowsHookEx(mouse) };
                    mouse = 0;
                }
                _ => {}
            }
        }
    });
    installed
        .recv()
        .map_err(|_| io::Error::other("the hook thread stopped"))?
}

/// While `true`, every hooked event is swallowed, and the mouse is hooked.
pub fn set_swallow(on: bool) {
    if let Some(shared) = SHARED.get() {
        shared.swallow.store(on, Ordering::SeqCst);
        let message = if on { MOUSE_ON } else { MOUSE_OFF };
        // SAFETY: posting a plain message to our own hook thread; if it has
        // gone, the call fails harmlessly.
        unsafe { PostThreadMessageW(shared.thread.load(Ordering::SeqCst), message, 0, 0) };
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

fn swallowing() -> bool {
    SHARED
        .get()
        .is_some_and(|shared| shared.swallow.load(Ordering::SeqCst))
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
    swallowing()
}

unsafe extern "system" fn keyboard_proc(code: i32, wparam: usize, lparam: isize) -> isize {
    if code == HC_ACTION {
        // SAFETY: for HC_ACTION, `lparam` points to a KBDLLHOOKSTRUCT that
        // stays valid until this procedure returns.
        let info = unsafe { &*(lparam as *const KeyboardInfo) };
        // Injected events (ours from SendInput, or other programs') pass.
        if info.flags & LLKHF_INJECTED == 0 {
            if info.scan_code & FAKE_CTRL_SCAN != 0 {
                // Not a real key: never sent on, and the hotkey must not see
                // it as "another key pressed". Swallowed with the rest.
                if swallowing() {
                    return 1;
                }
            } else {
                let scan = ScanCode {
                    code: info.scan_code as u8,
                    // Hooks mark Right Shift as extended, though its scan
                    // code is a plain 0x36; ignore that.
                    extended: info.flags & LLKHF_EXTENDED != 0
                        && info.scan_code != SCAN_RIGHT_SHIFT,
                };
                let pressed = wparam == WM_KEYDOWN || wparam == WM_SYSKEYDOWN;
                if report(HookEvent::Key { scan, pressed }) {
                    return 1;
                }
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
