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
const WS_POPUP: u32 = 0x8000_0000;
const WM_INPUT: u32 = 0x00FF;
const RIDEV_INPUTSINK: u32 = 0x0000_0100;
const RID_INPUT: u32 = 0x1000_0003;
const RIM_TYPEMOUSE: u32 = 0;
const MOUSE_MOVE_ABSOLUTE: u16 = 0x01;

/// `RAWINPUTDEVICE`.
#[repr(C)]
struct RawInputDevice {
    usage_page: u16,
    usage: u16,
    flags: u32,
    target: isize,
}
const WS_EX_TOOLWINDOW: u32 = 0x0000_0080;
const WS_EX_TOPMOST: u32 = 0x0000_0008;
const WS_EX_LAYERED: u32 = 0x0008_0000;
const SW_HIDE: i32 = 0;
const SW_SHOWNA: i32 = 8;

type WndProc = unsafe extern "system" fn(isize, u32, usize, isize) -> isize;

/// `WNDCLASSEXW`.
#[repr(C)]
struct WndClassExW {
    size: u32,
    style: u32,
    proc: WndProc,
    class_extra: i32,
    window_extra: i32,
    instance: isize,
    icon: isize,
    cursor: isize,
    background: isize,
    menu_name: *const u16,
    class_name: *const u16,
    icon_small: isize,
}
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
    fn RegisterClassExW(class: *const WndClassExW) -> u16;
    #[allow(clippy::too_many_arguments)]
    fn CreateWindowExW(
        ex_style: u32,
        class: *const u16,
        name: *const u16,
        style: u32,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        parent: isize,
        menu: isize,
        instance: isize,
        param: *const std::ffi::c_void,
    ) -> isize;
    fn DefWindowProcW(hwnd: isize, message: u32, wparam: usize, lparam: isize) -> isize;
    fn ShowWindow(hwnd: isize, command: i32) -> i32;
    fn SetForegroundWindow(hwnd: isize) -> i32;
    fn GetForegroundWindow() -> isize;
    fn BringWindowToTop(hwnd: isize) -> i32;
    fn GetWindowThreadProcessId(hwnd: isize, process: *mut u32) -> u32;
    fn AttachThreadInput(attach: u32, to: u32, on: i32) -> i32;
    fn DispatchMessageW(msg: *const Msg) -> isize;
    fn RegisterRawInputDevices(devices: *const RawInputDevice, count: u32, size: u32) -> i32;
    fn GetRawInputData(
        input: isize,
        command: u32,
        data: *mut std::ffi::c_void,
        size: *mut u32,
        header: u32,
    ) -> u32;
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
    /// Raw movement from the mouse itself (raw input), while swallowing:
    /// unaffected by a game pinning the cursor.
    Delta {
        dx: i32,
        dy: i32,
    },
}

struct Shared {
    events: Mutex<Sender<HookEvent>>,
    swallow: AtomicBool,
    /// The hook thread, which installs and removes the mouse hook.
    thread: AtomicU32,
    /// Raw mouse input is registered, so motion comes from `Delta`.
    raw_mouse: AtomicBool,
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
        raw_mouse: AtomicBool::new(false),
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
        let focus = FocusWindow::create(module);
        let raw_mouse = focus
            .as_ref()
            .is_some_and(|focus| focus.register_raw_mouse());
        if let Some(shared) = SHARED.get() {
            shared.raw_mouse.store(raw_mouse, Ordering::SeqCst);
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
                    if let Some(focus) = &focus {
                        focus.take();
                    }
                }
                MOUSE_OFF if mouse != 0 => {
                    // SAFETY: `mouse` is the hook this thread installed.
                    unsafe { UnhookWindowsHookEx(mouse) };
                    mouse = 0;
                    if let Some(focus) = &focus {
                        focus.give_back();
                    }
                }
                WM_INPUT => {
                    // Also while not swallowing: the source watches motion
                    // to notice the cursor pushing against a screen edge.
                    if let Some((dx, dy)) = raw_mouse_delta(msg.lparam)
                        && (dx != 0 || dy != 0)
                    {
                        report(HookEvent::Delta { dx, dy });
                    }
                    // WM_INPUT must reach DefWindowProc to be cleaned up.
                    // SAFETY: `msg` came from GetMessageW.
                    unsafe { DispatchMessageW(&msg) };
                }
                _ => {
                    // SAFETY: `msg` came from GetMessageW.
                    unsafe { DispatchMessageW(&msg) };
                }
            }
        }
    });
    installed
        .recv()
        .map_err(|_| io::Error::other("the hook thread stopped"))?
}

/// An invisible window that takes the foreground while the focus is on
/// another PC. Hooks cannot stop *raw input*, which games read the mouse
/// and keyboard with, but Windows only delivers raw input to the window in
/// the foreground: while this one is, a game sees nothing (PRD 0008).
struct FocusWindow {
    hwnd: isize,
    /// The window that had the foreground before, to give it back.
    previous: std::cell::Cell<isize>,
}

impl FocusWindow {
    fn create(module: isize) -> Option<FocusWindow> {
        let class: Vec<u16> = "MickeyFocus\0".encode_utf16().collect();
        let window_class = WndClassExW {
            size: std::mem::size_of::<WndClassExW>() as u32,
            style: 0,
            proc: default_proc,
            class_extra: 0,
            window_extra: 0,
            instance: module,
            icon: 0,
            cursor: 0,
            background: 0,
            menu_name: std::ptr::null(),
            class_name: class.as_ptr(),
            icon_small: 0,
        };
        // SAFETY: `window_class` is fully initialized and its strings live
        // until the class is registered (Windows copies them).
        unsafe { RegisterClassExW(&window_class) };
        // SAFETY: the class was just registered with this module; a 1x1
        // layered tool window is invisible and never shows in the taskbar.
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_LAYERED,
                class.as_ptr(),
                class.as_ptr(),
                WS_POPUP,
                0,
                0,
                1,
                1,
                0,
                0,
                module,
                std::ptr::null(),
            )
        };
        (hwnd != 0).then_some(FocusWindow {
            hwnd,
            previous: std::cell::Cell::new(0),
        })
    }

    fn take(&self) {
        // SAFETY: plain window calls on our own window and the current
        // foreground one; a stale handle only makes them fail.
        unsafe {
            let previous = GetForegroundWindow();
            self.previous.set(previous);
            ShowWindow(self.hwnd, SW_SHOWNA);
            bring_to_front(self.hwnd, previous);
        }
    }

    fn give_back(&self) {
        // SAFETY: as in `take`.
        unsafe {
            ShowWindow(self.hwnd, SW_HIDE);
            let previous = self.previous.replace(0);
            if previous != 0 {
                bring_to_front(previous, self.hwnd);
            }
        }
    }
}

impl FocusWindow {
    /// Asks Windows for raw mouse input on this window, even when it is not
    /// in front (`RIDEV_INPUTSINK`).
    fn register_raw_mouse(&self) -> bool {
        let device = RawInputDevice {
            usage_page: 0x01, // generic desktop
            usage: 0x02,      // mouse
            flags: RIDEV_INPUTSINK,
            target: self.hwnd,
        };
        // SAFETY: one fully initialized RAWINPUTDEVICE and its exact size.
        unsafe {
            RegisterRawInputDevices(&device, 1, std::mem::size_of::<RawInputDevice>() as u32) != 0
        }
    }
}

/// Whether raw mouse input is in use (then hook moves are not motion).
pub fn raw_mouse_active() -> bool {
    SHARED
        .get()
        .is_some_and(|shared| shared.raw_mouse.load(Ordering::SeqCst))
}

/// The relative movement in a WM_INPUT message, if it is a mouse move.
fn raw_mouse_delta(lparam: isize) -> Option<(i32, i32)> {
    // RAWINPUTHEADER (24 bytes on 64-bit) then RAWMOUSE (24 bytes).
    const HEADER: usize = 24;
    let mut data = [0u8; HEADER + 24];
    let mut size = data.len() as u32;
    // SAFETY: `data` is `size` bytes long; Windows writes at most that many
    // for the handle WM_INPUT gave us, and returns the count written.
    let written = unsafe {
        GetRawInputData(
            lparam,
            RID_INPUT,
            data.as_mut_ptr().cast(),
            &mut size,
            HEADER as u32,
        )
    };
    if written == u32::MAX || (written as usize) < HEADER + 20 {
        return None;
    }
    let kind = u32::from_ne_bytes([data[0], data[1], data[2], data[3]]);
    let flags = u16::from_ne_bytes([data[HEADER], data[HEADER + 1]]);
    if kind != RIM_TYPEMOUSE || flags & MOUSE_MOVE_ABSOLUTE != 0 {
        return None;
    }
    let at = |i: usize| i32::from_ne_bytes([data[i], data[i + 1], data[i + 2], data[i + 3]]);
    Some((at(HEADER + 12), at(HEADER + 16)))
}

/// `SetForegroundWindow` is refused unless the caller is in on the current
/// input; joining the input of the window that has it lifts that.
///
/// # Safety
/// `window` and `current` are window handles (possibly stale).
unsafe fn bring_to_front(window: isize, current: isize) {
    // SAFETY: the caller passes window handles; the thread ids they yield
    // are only used to attach and detach input for this call.
    unsafe {
        let ours = GetCurrentThreadId();
        let theirs = GetWindowThreadProcessId(current, std::ptr::null_mut());
        let joined = theirs != 0 && theirs != ours && AttachThreadInput(ours, theirs, 1) != 0;
        SetForegroundWindow(window);
        BringWindowToTop(window);
        if joined {
            AttachThreadInput(ours, theirs, 0);
        }
    }
}

unsafe extern "system" fn default_proc(
    hwnd: isize,
    message: u32,
    wparam: usize,
    lparam: isize,
) -> isize {
    // SAFETY: passes the window procedure's own arguments on unchanged.
    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
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

/// Whether a mouse button is down right now.
pub fn button_down() -> bool {
    // VK_LBUTTON, VK_RBUTTON, VK_MBUTTON, VK_XBUTTON1, VK_XBUTTON2.
    // SAFETY: as in `any_key_down`.
    [0x01, 0x02, 0x04, 0x05, 0x06]
        .into_iter()
        .any(|vk| unsafe { GetAsyncKeyState(vk) } < 0)
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
