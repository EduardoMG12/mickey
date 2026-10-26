//! A notification-area icon on Windows (PRD 0009), by hand: a hidden
//! window on its own thread owns the icon, Windows tells it about clicks,
//! and it shows a popup menu. It knows nothing about Mickey: it shows the
//! picture, tooltip and menu it is given, and reports the id picked.

use std::ffi::c_void;
use std::io;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::thread;

use super::hooks::{Msg, Point};

/// One line of the menu; `id` 0 is a separator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuItem {
    pub label: String,
    pub enabled: bool,
    pub id: u32,
}

/// What the icon shows: `picture` is 32x32 RGBA.
pub struct Look {
    pub picture: Vec<u8>,
    pub tip: String,
    pub menu: Vec<MenuItem>,
}

struct Shared {
    hwnd: AtomicIsize,
    look: Mutex<Look>,
    picked: Box<dyn Fn(u32) + Send + Sync>,
}

/// One icon per process: the window procedure finds its state here.
static SHARED: OnceLock<Shared> = OnceLock::new();

const SIZE: i32 = 32;
const ICON_ID: u32 = 1;
/// Windows reports clicks on the icon with this message.
const WM_TRAY: u32 = 0x8000 + 1;
/// Mickey asks the window to show the new look.
const WM_REFRESH: u32 = 0x8000 + 2;
/// Mickey asks the window to remove the icon and end.
const WM_GOODBYE: u32 = 0x8000 + 3;
const WM_LBUTTONUP: u32 = 0x0202;
const WM_RBUTTONUP: u32 = 0x0205;
const WM_NULL: u32 = 0;
const NIM_ADD: u32 = 0;
const NIM_MODIFY: u32 = 1;
const NIM_DELETE: u32 = 2;
const NIF_MESSAGE: u32 = 1;
const NIF_ICON: u32 = 2;
const NIF_TIP: u32 = 4;
const NIF_GUID: u32 = 0x20;
/// Mickey's icon, the same in every run: a run killed without removing its
/// icon leaves a ghost in the tray, and the next run removes it by this.
const ICON_GUID: [u8; 16] = [
    0x6d, 0x69, 0x63, 0x6b, 0x65, 0x79, 0x4e, 0x01, 0x9a, 0x2f, 0x41, 0x7c, 0x0e, 0x53, 0x88, 0xb4,
];
/// Windows ties a GUID to the program's path; if it refuses ours (Mickey
/// moved), the icon goes by window and id instead, ghosts and all.
static BY_GUID: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);
const MF_STRING: u32 = 0;
const MF_GRAYED: u32 = 1;
const MF_SEPARATOR: u32 = 0x800;
const TPM_RETURNCMD: u32 = 0x100;
const TPM_RIGHTBUTTON: u32 = 2;

/// `NOTIFYICONDATAW`.
#[repr(C)]
struct NotifyIconData {
    size: u32,
    hwnd: isize,
    id: u32,
    flags: u32,
    callback_message: u32,
    icon: isize,
    tip: [u16; 128],
    state: u32,
    state_mask: u32,
    info: [u16; 256],
    version: u32,
    info_title: [u16; 64],
    info_flags: u32,
    guid: [u8; 16],
    balloon_icon: isize,
}

#[cfg(target_pointer_width = "64")]
const _: () = assert!(size_of::<NotifyIconData>() == 976);

/// `ICONINFO`.
#[repr(C)]
struct IconInfo {
    is_icon: i32,
    hotspot_x: u32,
    hotspot_y: u32,
    mask: isize,
    colour: isize,
}

/// `BITMAPINFO` with its `BITMAPINFOHEADER` and no colour table.
#[repr(C)]
struct BitmapInfo {
    size: u32,
    width: i32,
    height: i32,
    planes: u16,
    bit_count: u16,
    compression: u32,
    size_image: u32,
    x_per_metre: i32,
    y_per_metre: i32,
    used: u32,
    important: u32,
    colours: [u32; 1],
}

/// `WNDCLASSW`.
#[repr(C)]
struct WndClassW {
    style: u32,
    proc: unsafe extern "system" fn(isize, u32, usize, isize) -> isize,
    class_extra: i32,
    window_extra: i32,
    instance: isize,
    icon: isize,
    cursor: isize,
    background: isize,
    menu_name: *const u16,
    class_name: *const u16,
}

#[link(name = "user32")]
unsafe extern "system" {
    fn RegisterClassW(class: *const WndClassW) -> u16;
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
        param: *const c_void,
    ) -> isize;
    fn DefWindowProcW(hwnd: isize, message: u32, wparam: usize, lparam: isize) -> isize;
    fn GetMessageW(msg: *mut Msg, hwnd: isize, min: u32, max: u32) -> i32;
    fn DispatchMessageW(msg: *const Msg) -> isize;
    fn PostMessageW(hwnd: isize, message: u32, wparam: usize, lparam: isize) -> i32;
    fn PostQuitMessage(code: i32);
    fn DestroyWindow(hwnd: isize) -> i32;
    fn RegisterWindowMessageW(name: *const u16) -> u32;
    fn CreatePopupMenu() -> isize;
    fn AppendMenuW(menu: isize, flags: u32, id: usize, text: *const u16) -> i32;
    fn TrackPopupMenu(
        menu: isize,
        flags: u32,
        x: i32,
        y: i32,
        reserved: i32,
        hwnd: isize,
        rect: *const c_void,
    ) -> i32;
    fn DestroyMenu(menu: isize) -> i32;
    fn GetCursorPos(point: *mut Point) -> i32;
    fn SetForegroundWindow(hwnd: isize) -> i32;
    fn CreateIconIndirect(info: *const IconInfo) -> isize;
    fn DestroyIcon(icon: isize) -> i32;
    fn GetDC(hwnd: isize) -> isize;
    fn ReleaseDC(hwnd: isize, dc: isize) -> i32;
}

#[link(name = "gdi32")]
unsafe extern "system" {
    fn CreateDIBSection(
        dc: isize,
        info: *const BitmapInfo,
        usage: u32,
        bits: *mut *mut c_void,
        section: isize,
        offset: u32,
    ) -> isize;
    fn CreateBitmap(
        width: i32,
        height: i32,
        planes: u32,
        bit_count: u32,
        bits: *const c_void,
    ) -> isize;
    fn DeleteObject(object: isize) -> i32;
}

#[link(name = "shell32")]
unsafe extern "system" {
    fn Shell_NotifyIconW(message: u32, data: *const NotifyIconData) -> i32;
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetModuleHandleW(name: *const u16) -> isize;
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}

/// The icon, while this lives.
pub struct Icon {
    /// The window's thread, which removes the icon before it ends.
    thread: Option<thread::JoinHandle<()>>,
}

impl Icon {
    /// Shows the icon; `picked` gets the id of each menu line picked.
    pub fn start(look: Look, picked: Box<dyn Fn(u32) + Send + Sync>) -> io::Result<Icon> {
        let shared = Shared {
            hwnd: AtomicIsize::new(0),
            look: Mutex::new(look),
            picked,
        };
        SHARED
            .set(shared)
            .map_err(|_| io::Error::other("the tray icon is already shown"))?;
        let (ready, started) = std::sync::mpsc::channel();
        let thread = crate::threads::spawn("tray window", move || run(&ready));
        started
            .recv()
            .map_err(|_| io::Error::other("the tray thread stopped"))??;
        Ok(Icon {
            thread: Some(thread),
        })
    }

    pub fn show(&self, look: Look) {
        if let Some(shared) = SHARED.get() {
            *shared.look.lock().unwrap_or_else(|p| p.into_inner()) = look;
            // SAFETY: posting to our own window; a 0 handle just fails.
            unsafe { PostMessageW(shared.hwnd.load(Ordering::SeqCst), WM_REFRESH, 0, 0) };
        }
    }
}

impl Drop for Icon {
    /// Removes the icon, or it would stay in the tray until hovered, and
    /// waits until it is gone: Mickey may be about to exit.
    fn drop(&mut self) {
        if let Some(shared) = SHARED.get() {
            // SAFETY: as in `show`.
            unsafe { PostMessageW(shared.hwnd.load(Ordering::SeqCst), WM_GOODBYE, 0, 0) };
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// The window's thread: creates the window and the icon, says on `ready`
/// whether that worked, then answers messages until told goodbye.
fn run(ready: &std::sync::mpsc::Sender<io::Result<()>>) {
    let class = wide("MickeyTray");
    // SAFETY: no arguments; returns this program's module handle.
    let module = unsafe { GetModuleHandleW(std::ptr::null()) };
    let window_class = WndClassW {
        style: 0,
        proc: window_proc,
        class_extra: 0,
        window_extra: 0,
        instance: module,
        icon: 0,
        cursor: 0,
        background: 0,
        menu_name: std::ptr::null(),
        class_name: class.as_ptr(),
    };
    // SAFETY: `window_class` is fully set; Windows copies what it needs.
    unsafe { RegisterClassW(&window_class) };
    // SAFETY: the class was just registered with this module. The window is
    // never shown: it only receives the icon's messages.
    let hwnd = unsafe {
        CreateWindowExW(
            0,
            class.as_ptr(),
            class.as_ptr(),
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            module,
            std::ptr::null(),
        )
    };
    if hwnd == 0 {
        let _ = ready.send(Err(io::Error::last_os_error()));
        return;
    }
    let shared = SHARED.get().expect("set before the thread starts");
    shared.hwnd.store(hwnd, Ordering::SeqCst);
    remove_icon(hwnd);
    if !notify(hwnd, NIM_ADD) {
        BY_GUID.store(false, Ordering::SeqCst);
    }
    if !BY_GUID.load(Ordering::SeqCst) && !notify(hwnd, NIM_ADD) {
        let _ = ready.send(Err(io::Error::other("Windows refused the tray icon")));
        return;
    }
    // Not at the end: `start` waits for this, and the loop below runs for
    // as long as Mickey does.
    let _ = ready.send(Ok(()));
    let mut msg = Msg::default();
    // SAFETY: `msg` is a valid MSG for every call.
    while unsafe { GetMessageW(&mut msg, 0, 0, 0) } > 0 {
        // SAFETY: `msg` came from GetMessageW.
        unsafe { DispatchMessageW(&msg) };
    }
}

/// Adds or updates the icon with the current look. False if Windows said
/// no (no taskbar yet, for instance).
fn notify(hwnd: isize, message: u32) -> bool {
    let Some(shared) = SHARED.get() else {
        return false;
    };
    let look = shared.look.lock().unwrap_or_else(|p| p.into_inner());
    let icon = make_icon(&look.picture);
    // SAFETY: NOTIFYICONDATAW is plain data; all zeros is a valid start.
    let mut data: NotifyIconData = unsafe { std::mem::zeroed() };
    data.size = size_of::<NotifyIconData>() as u32;
    data.hwnd = hwnd;
    data.id = ICON_ID;
    data.flags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
    if BY_GUID.load(Ordering::SeqCst) {
        data.flags |= NIF_GUID;
        data.guid = ICON_GUID;
    }
    data.callback_message = WM_TRAY;
    data.icon = icon;
    for (slot, unit) in data.tip.iter_mut().zip(look.tip.encode_utf16().take(127)) {
        *slot = unit;
    }
    // SAFETY: `data` is fully set and lives for the call.
    let done = unsafe { Shell_NotifyIconW(message, &data) } != 0;
    if icon != 0 {
        // SAFETY: Windows copied the icon; ours can go.
        unsafe { DestroyIcon(icon) };
    }
    done
}

/// Removes Mickey's icon: this run's, or a ghost a killed run left.
fn remove_icon(hwnd: isize) {
    // SAFETY: NOTIFYICONDATAW is plain data; removing needs only the GUID,
    // or the window and the id.
    unsafe {
        let mut data: NotifyIconData = std::mem::zeroed();
        data.size = size_of::<NotifyIconData>() as u32;
        data.hwnd = hwnd;
        data.id = ICON_ID;
        if BY_GUID.load(Ordering::SeqCst) {
            data.flags = NIF_GUID;
            data.guid = ICON_GUID;
        }
        Shell_NotifyIconW(NIM_DELETE, &data);
    }
}

/// An icon from 32x32 RGBA; 0 if Windows could not make it.
fn make_icon(rgba: &[u8]) -> isize {
    let info = BitmapInfo {
        size: 40,
        width: SIZE,
        // Negative: rows from the top, as the picture has them.
        height: -SIZE,
        planes: 1,
        bit_count: 32,
        compression: 0,
        size_image: 0,
        x_per_metre: 0,
        y_per_metre: 0,
        used: 0,
        important: 0,
        colours: [0],
    };
    let mut bits: *mut c_void = std::ptr::null_mut();
    // SAFETY: the screen's device context, released right after; `info`
    // describes a 32x32 32-bit bitmap, and `bits` receives its memory.
    let colour = unsafe {
        let dc = GetDC(0);
        let colour = CreateDIBSection(dc, &info, 0, &mut bits, 0, 0);
        ReleaseDC(0, dc);
        colour
    };
    if colour == 0 || bits.is_null() {
        return 0;
    }
    let bgra: Vec<u8> = rgba
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|&[r, g, b, a]| [b, g, r, a])
        .collect();
    let len = (SIZE * SIZE * 4) as usize;
    let mask_bits = [0u8; (SIZE * SIZE / 8) as usize];
    // SAFETY: `bits` points at the bitmap's SIZE*SIZE*4 bytes, and `bgra`
    // has that many when the picture is the right size (checked).
    unsafe {
        if bgra.len() == len {
            std::ptr::copy_nonoverlapping(bgra.as_ptr(), bits.cast::<u8>(), len);
        }
        // The alpha channel decides what shows; the mask is all "show".
        let mask = CreateBitmap(SIZE, SIZE, 1, 1, mask_bits.as_ptr().cast());
        let icon = CreateIconIndirect(&IconInfo {
            is_icon: 1,
            hotspot_x: 0,
            hotspot_y: 0,
            mask,
            colour,
        });
        DeleteObject(mask);
        DeleteObject(colour);
        icon
    }
}

/// Shows the menu where the cursor is and reports the line picked.
fn show_menu(hwnd: isize) {
    let Some(shared) = SHARED.get() else {
        return;
    };
    let items = shared
        .look
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .menu
        .clone();
    // SAFETY: the menu is ours from creation to destruction here; every
    // string lives across the AppendMenuW call that copies it.
    let picked = unsafe {
        let menu = CreatePopupMenu();
        for item in &items {
            let label = wide(&item.label);
            let flags = match (item.id, item.enabled) {
                (0, _) => MF_SEPARATOR,
                (_, true) => MF_STRING,
                (_, false) => MF_STRING | MF_GRAYED,
            };
            AppendMenuW(menu, flags, item.id as usize, label.as_ptr());
        }
        let mut point = Point::default();
        GetCursorPos(&mut point);
        // Without these two, the menu would not close when clicking away.
        SetForegroundWindow(hwnd);
        let picked = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON,
            point.x,
            point.y,
            0,
            hwnd,
            std::ptr::null(),
        );
        PostMessageW(hwnd, WM_NULL, 0, 0);
        DestroyMenu(menu);
        picked
    };
    if picked > 0 {
        (shared.picked)(picked as u32);
    }
}

unsafe extern "system" fn window_proc(
    hwnd: isize,
    message: u32,
    wparam: usize,
    lparam: isize,
) -> isize {
    // Explorer sends this when it (re)starts: the icon must be added again.
    static TASKBAR_CREATED: OnceLock<u32> = OnceLock::new();
    // SAFETY: a constant, NUL-terminated name.
    let taskbar_created = *TASKBAR_CREATED
        .get_or_init(|| unsafe { RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()) });
    match message {
        WM_TRAY if matches!(lparam as u32 & 0xFFFF, WM_LBUTTONUP | WM_RBUTTONUP) => show_menu(hwnd),
        WM_REFRESH => {
            notify(hwnd, NIM_MODIFY);
        }
        WM_GOODBYE => {
            remove_icon(hwnd);
            // SAFETY: our own window, from its own thread.
            unsafe {
                DestroyWindow(hwnd);
                PostQuitMessage(0);
            }
        }
        _ if message == taskbar_created => {
            notify(hwnd, NIM_ADD);
        }
        _ => {}
    }
    // SAFETY: passes the window procedure's own arguments on unchanged.
    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}
