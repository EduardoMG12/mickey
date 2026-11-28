//! macOS Menu Bar icon (`NSStatusItem`) via hand-written AppKit / Objective-C
//! runtime bindings for Apple Silicon (`aarch64-apple-darwin`).
//!
//! AppKit requires `NSApplication` and `NSStatusItem` to run on the main
//! thread. `run_with_app` initializes `NSApplication` as an accessory app (no
//! Dock icon), runs Mickey's daemon on a worker thread, and services UI
//! updates dispatched onto the main queue via `dispatch_async_f`.

use crate::threads::LockAnyway;
use std::ffi::{CStr, CString, c_char, c_void};
use std::io;
use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};
use std::sync::{Mutex, OnceLock};

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
    look: Mutex<Look>,
    picked: Box<dyn Fn(u32) + Send + Sync>,
    status_item: AtomicPtr<c_void>,
    target: AtomicPtr<c_void>,
}

/// One menu bar icon per process; the Objective-C action callback finds it here.
static SHARED: OnceLock<Shared> = OnceLock::new();
/// Set while `run_with_app` is driving `NSApplication` on the main thread.
static MAIN_LOOP_RUNNING: AtomicBool = AtomicBool::new(false);

const ICON_PX: usize = 32;
const ICON_PT: f64 = 18.0;
const NS_ACTIVATION_POLICY_ACCESSORY: isize = 1;
const NS_EVENT_TYPE_APPLICATION_DEFINED: usize = 15;
const CG_IMAGE_ALPHA_LAST: u32 = 3;

#[repr(C)]
#[derive(Clone, Copy)]
struct NSSize {
    width: f64,
    height: f64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct NSPoint {
    x: f64,
    y: f64,
}

#[link(name = "AppKit", kind = "framework")]
unsafe extern "C" {}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFDataCreate(allocator: *const c_void, bytes: *const u8, length: isize) -> *mut c_void;
    fn CFRelease(cf: *const c_void);
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGColorSpaceCreateDeviceRGB() -> *mut c_void;
    fn CGDataProviderCreateWithCFData(data: *const c_void) -> *mut c_void;
    fn CGImageCreate(
        width: usize,
        height: usize,
        bits_per_component: usize,
        bits_per_pixel: usize,
        bytes_per_row: usize,
        space: *mut c_void,
        bitmap_info: u32,
        provider: *mut c_void,
        decode: *const f64,
        should_interpolate: bool,
        intent: u32,
    ) -> *mut c_void;
}

#[link(name = "objc")]
unsafe extern "C" {
    fn objc_getClass(name: *const c_char) -> *mut c_void;
    fn sel_registerName(name: *const c_char) -> *mut c_void;
    fn objc_allocateClassPair(
        superclass: *mut c_void,
        name: *const c_char,
        extra_bytes: usize,
    ) -> *mut c_void;
    fn class_addMethod(
        cls: *mut c_void,
        name: *mut c_void,
        imp: *const c_void,
        types: *const c_char,
    ) -> bool;
    fn objc_registerClassPair(cls: *mut c_void);
    fn objc_autoreleasePoolPush() -> *mut c_void;
    fn objc_autoreleasePoolPop(pool: *mut c_void);
    fn objc_msgSend();
}

#[repr(C)]
struct DispatchQueue {
    _p: [u8; 0],
}

#[link(name = "System")]
unsafe extern "C" {
    static _dispatch_main_q: DispatchQueue;
    fn dispatch_async_f(
        queue: *mut c_void,
        context: *mut c_void,
        work: unsafe extern "C" fn(*mut c_void),
    );
}

// SAFETY helpers: on Apple Silicon (ARM64), `objc_msgSend` uses the standard
// C ABI for all pointer, integer, floating-point, and small-struct signatures.

unsafe fn cls(name: &CStr) -> *mut c_void {
    // SAFETY: `name` is a valid NUL-terminated C string.
    unsafe { objc_getClass(name.as_ptr()) }
}

unsafe fn sel(name: &CStr) -> *mut c_void {
    // SAFETY: `name` is a valid NUL-terminated C string.
    unsafe { sel_registerName(name.as_ptr()) }
}

unsafe fn msg_id(obj: *mut c_void, selector: *mut c_void) -> *mut c_void {
    type F = unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void;
    // SAFETY: caller guarantees `obj` responds to `selector` returning `id`.
    unsafe {
        let f: F = std::mem::transmute(objc_msgSend as *const ());
        f(obj, selector)
    }
}

unsafe fn msg_void(obj: *mut c_void, selector: *mut c_void) {
    type F = unsafe extern "C" fn(*mut c_void, *mut c_void);
    // SAFETY: caller guarantees `obj` responds to `selector` returning void.
    unsafe {
        let f: F = std::mem::transmute(objc_msgSend as *const ());
        f(obj, selector);
    }
}

unsafe fn msg_void_id(obj: *mut c_void, selector: *mut c_void, arg: *mut c_void) {
    type F = unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void);
    // SAFETY: caller guarantees signature `(id) -> void`.
    unsafe {
        let f: F = std::mem::transmute(objc_msgSend as *const ());
        f(obj, selector, arg);
    }
}

unsafe fn msg_void_bool(obj: *mut c_void, selector: *mut c_void, arg: bool) {
    type F = unsafe extern "C" fn(*mut c_void, *mut c_void, bool);
    // SAFETY: caller guarantees signature `(BOOL) -> void`.
    unsafe {
        let f: F = std::mem::transmute(objc_msgSend as *const ());
        f(obj, selector, arg);
    }
}

unsafe fn msg_void_isize(obj: *mut c_void, selector: *mut c_void, arg: isize) {
    type F = unsafe extern "C" fn(*mut c_void, *mut c_void, isize);
    // SAFETY: caller guarantees signature `(NSInteger) -> void`.
    unsafe {
        let f: F = std::mem::transmute(objc_msgSend as *const ());
        f(obj, selector, arg);
    }
}

unsafe fn msg_isize(obj: *mut c_void, selector: *mut c_void) -> isize {
    type F = unsafe extern "C" fn(*mut c_void, *mut c_void) -> isize;
    // SAFETY: caller guarantees signature `() -> NSInteger`.
    unsafe {
        let f: F = std::mem::transmute(objc_msgSend as *const ());
        f(obj, selector)
    }
}

unsafe fn msg_id_f64(obj: *mut c_void, selector: *mut c_void, arg: f64) -> *mut c_void {
    type F = unsafe extern "C" fn(*mut c_void, *mut c_void, f64) -> *mut c_void;
    // SAFETY: caller guarantees signature `(CGFloat) -> id`.
    unsafe {
        let f: F = std::mem::transmute(objc_msgSend as *const ());
        f(obj, selector, arg)
    }
}

unsafe fn msg_id_cstr(obj: *mut c_void, selector: *mut c_void, arg: *const c_char) -> *mut c_void {
    type F = unsafe extern "C" fn(*mut c_void, *mut c_void, *const c_char) -> *mut c_void;
    // SAFETY: caller guarantees signature `(const char*) -> id`.
    unsafe {
        let f: F = std::mem::transmute(objc_msgSend as *const ());
        f(obj, selector, arg)
    }
}

unsafe fn ns_string(text: &str) -> *mut c_void {
    let clean = text.replace('\0', "");
    let cstr = CString::new(clean).unwrap_or_default();
    // SAFETY: `NSString` responds to `stringWithUTF8String:` with a C string.
    unsafe {
        msg_id_cstr(
            cls(c"NSString"),
            sel(c"stringWithUTF8String:"),
            cstr.as_ptr(),
        )
    }
}

/// Runs `work` on a worker thread while driving `NSApplication` on the main
/// thread so `NSStatusItem` can live on thread 0 as AppKit requires.
pub fn run_with_app(
    work: impl FnOnce() -> Result<(), String> + Send + 'static,
) -> Result<(), String> {
    // SAFETY: called on the main thread; initializes `NSApplication` in
    // accessory mode (menu bar icon only, no Dock icon).
    let app = unsafe {
        let pool = objc_autoreleasePoolPush();
        let app = msg_id(cls(c"NSApplication"), sel(c"sharedApplication"));
        msg_void_isize(
            app,
            sel(c"setActivationPolicy:"),
            NS_ACTIVATION_POLICY_ACCESSORY,
        );
        objc_autoreleasePoolPop(pool);
        app
    };
    if app.is_null() {
        return work();
    }

    /// Always wakes and stops `[NSApp run]`, even if `work()` panics.
    struct StopAppOnDrop;
    impl Drop for StopAppOnDrop {
        fn drop(&mut self) {
            dispatch_to_main(stop_app_callback);
        }
    }

    MAIN_LOOP_RUNNING.store(true, Ordering::Release);
    let worker = crate::threads::spawn("mickey main", move || {
        let _stop = StopAppOnDrop;
        work()
    });

    // SAFETY: `app` is the shared `NSApplication` on the main thread.
    unsafe { msg_void(app, sel(c"run")) };
    MAIN_LOOP_RUNNING.store(false, Ordering::Release);

    worker
        .join()
        .map_err(|_| "mickey worker thread panicked".to_string())?
}

fn dispatch_to_main(work: unsafe extern "C" fn(*mut c_void)) {
    // SAFETY: `_dispatch_main_q` is libdispatch's static main queue; `work`
    // does not dereference the null context pointer.
    unsafe {
        dispatch_async_f(
            (&raw const _dispatch_main_q).cast::<c_void>().cast_mut(),
            std::ptr::null_mut(),
            work,
        );
    }
}

unsafe extern "C" fn stop_app_callback(_ctx: *mut c_void) {
    type OtherEventFn = unsafe extern "C" fn(
        *mut c_void,
        *mut c_void,
        usize,
        NSPoint,
        usize,
        f64,
        isize,
        *mut c_void,
        i16,
        isize,
        isize,
    ) -> *mut c_void;
    type PostEventFn = unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void, bool);

    // SAFETY: runs on the main queue; stops `NSApplication::run` and posts an
    // application-defined event so the run loop wakes up immediately.
    unsafe {
        let pool = objc_autoreleasePoolPush();
        let app = msg_id(cls(c"NSApplication"), sel(c"sharedApplication"));
        msg_void_id(app, sel(c"stop:"), std::ptr::null_mut());
        let other: OtherEventFn = std::mem::transmute(objc_msgSend as *const ());
        let event = other(
            cls(c"NSEvent"),
            sel(c"otherEventWithType:location:modifierFlags:timestamp:windowNumber:context:subtype:data1:data2:"),
            NS_EVENT_TYPE_APPLICATION_DEFINED,
            NSPoint { x: 0.0, y: 0.0 },
            0,
            0.0,
            0,
            std::ptr::null_mut(),
            0,
            0,
            0,
        );
        if !event.is_null() {
            let post: PostEventFn = std::mem::transmute(objc_msgSend as *const ());
            post(app, sel(c"postEvent:atStart:"), event, true);
        }
        objc_autoreleasePoolPop(pool);
    }
}

/// The menu bar icon, while this lives.
pub struct Icon;

impl Icon {
    pub fn start(look: Look, picked: Box<dyn Fn(u32) + Send + Sync>) -> io::Result<Icon> {
        if !MAIN_LOOP_RUNNING.load(Ordering::Acquire) {
            return Err(io::Error::other(
                "macOS menu bar requires the AppKit main loop",
            ));
        }
        if let Some(shared) = SHARED.get() {
            *shared.look.lock_anyway() = look;
        } else {
            let _ = SHARED.set(Shared {
                look: Mutex::new(look),
                picked,
                status_item: AtomicPtr::new(std::ptr::null_mut()),
                target: AtomicPtr::new(std::ptr::null_mut()),
            });
        }
        dispatch_to_main(apply_look_callback);
        Ok(Icon)
    }

    pub fn show(&self, look: Look) {
        if let Some(shared) = SHARED.get() {
            *shared.look.lock_anyway() = look;
            dispatch_to_main(apply_look_callback);
        }
    }
}

impl Drop for Icon {
    fn drop(&mut self) {
        if MAIN_LOOP_RUNNING.load(Ordering::Acquire) {
            dispatch_to_main(remove_icon_callback);
        }
    }
}

unsafe extern "C" fn on_menu_click(_this: *mut c_void, _cmd: *mut c_void, sender: *mut c_void) {
    if sender.is_null() {
        return;
    }
    // SAFETY: `sender` is the `NSMenuItem` whose tag was set to the item id.
    let tag = unsafe { msg_isize(sender, sel(c"tag")) };
    if let (Ok(id), Some(shared)) = (u32::try_from(tag), SHARED.get()) {
        (shared.picked)(id);
    }
}

unsafe fn ensure_target(shared: &Shared) -> *mut c_void {
    let existing = shared.target.load(Ordering::Acquire);
    if !existing.is_null() {
        return existing;
    }
    // SAFETY: runs exclusively on the main thread; registers a minimal
    // `NSObject` subclass once and instantiates it.
    unsafe {
        let mut target_cls = cls(c"MickeyMenuTarget");
        if target_cls.is_null() {
            target_cls = objc_allocateClassPair(cls(c"NSObject"), c"MickeyMenuTarget".as_ptr(), 0);
            if !target_cls.is_null() {
                class_addMethod(
                    target_cls,
                    sel(c"menuItemClicked:"),
                    on_menu_click as *const c_void,
                    c"v@:@".as_ptr(),
                );
                objc_registerClassPair(target_cls);
            }
        }
        let target = msg_id(msg_id(target_cls, sel(c"alloc")), sel(c"init"));
        shared.target.store(target, Ordering::Release);
        target
    }
}

unsafe fn image_from_rgba(rgba: &[u8]) -> *mut c_void {
    type InitWithCgImage =
        unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void, NSSize) -> *mut c_void;

    if rgba.len() < ICON_PX * ICON_PX * 4 {
        return std::ptr::null_mut();
    }
    // SAFETY: `CFDataCreate` copies `rgba` into CoreFoundation-managed memory;
    // every CoreFoundation/CoreGraphics object is released before returning.
    unsafe {
        let cf_data = CFDataCreate(std::ptr::null(), rgba.as_ptr(), rgba.len() as isize);
        if cf_data.is_null() {
            return std::ptr::null_mut();
        }
        let provider = CGDataProviderCreateWithCFData(cf_data);
        let color_space = CGColorSpaceCreateDeviceRGB();
        let cg_image = if !provider.is_null() && !color_space.is_null() {
            CGImageCreate(
                ICON_PX,
                ICON_PX,
                8,
                32,
                ICON_PX * 4,
                color_space,
                CG_IMAGE_ALPHA_LAST,
                provider,
                std::ptr::null(),
                true,
                0,
            )
        } else {
            std::ptr::null_mut()
        };
        let ns_image = if !cg_image.is_null() {
            let alloc = msg_id(cls(c"NSImage"), sel(c"alloc"));
            let init: InitWithCgImage = std::mem::transmute(objc_msgSend as *const ());
            init(
                alloc,
                sel(c"initWithCGImage:size:"),
                cg_image,
                NSSize {
                    width: ICON_PT,
                    height: ICON_PT,
                },
            )
        } else {
            std::ptr::null_mut()
        };
        if !cg_image.is_null() {
            CFRelease(cg_image.cast());
        }
        if !color_space.is_null() {
            CFRelease(color_space.cast());
        }
        if !provider.is_null() {
            CFRelease(provider.cast());
        }
        CFRelease(cf_data.cast());
        ns_image
    }
}

unsafe extern "C" fn apply_look_callback(_ctx: *mut c_void) {
    type InitMenuItem = unsafe extern "C" fn(
        *mut c_void,
        *mut c_void,
        *mut c_void,
        *mut c_void,
        *mut c_void,
    ) -> *mut c_void;

    let Some(shared) = SHARED.get() else {
        return;
    };
    let (picture, tip, menu_items) = {
        let look = shared.look.lock_anyway();
        (look.picture.clone(), look.tip.clone(), look.menu.clone())
    };

    // SAFETY: runs on the main queue inside an autorelease pool; all AppKit
    // objects are created, configured, and released on thread 0.
    unsafe {
        let pool = objc_autoreleasePoolPush();
        let mut status_item = shared.status_item.load(Ordering::Acquire);
        if status_item.is_null() {
            let bar = msg_id(cls(c"NSStatusBar"), sel(c"systemStatusBar"));
            let created = msg_id_f64(bar, sel(c"statusItemWithLength:"), -1.0);
            if !created.is_null() {
                status_item = msg_id(created, sel(c"retain"));
                shared.status_item.store(status_item, Ordering::Release);
            }
        }
        if status_item.is_null() {
            objc_autoreleasePoolPop(pool);
            return;
        }

        let button = msg_id(status_item, sel(c"button"));
        if !button.is_null() {
            let img = image_from_rgba(&picture);
            if !img.is_null() {
                msg_void_id(button, sel(c"setImage:"), img);
                msg_void(img, sel(c"release"));
            }
            msg_void_id(button, sel(c"setToolTip:"), ns_string(&tip));
        }

        let target = ensure_target(shared);
        let menu = msg_id(msg_id(cls(c"NSMenu"), sel(c"alloc")), sel(c"init"));
        msg_void_bool(menu, sel(c"setAutoenablesItems:"), false);
        let empty = ns_string("");
        let action = sel(c"menuItemClicked:");
        let init_item: InitMenuItem = std::mem::transmute(objc_msgSend as *const ());

        for item in &menu_items {
            if item.id == 0 {
                let sep = msg_id(cls(c"NSMenuItem"), sel(c"separatorItem"));
                msg_void_id(menu, sel(c"addItem:"), sep);
                continue;
            }
            let alloc = msg_id(cls(c"NSMenuItem"), sel(c"alloc"));
            let mi = init_item(
                alloc,
                sel(c"initWithTitle:action:keyEquivalent:"),
                ns_string(&item.label),
                action,
                empty,
            );
            msg_void_id(mi, sel(c"setTarget:"), target);
            msg_void_isize(mi, sel(c"setTag:"), item.id as isize);
            msg_void_bool(mi, sel(c"setEnabled:"), item.enabled);
            msg_void_id(menu, sel(c"addItem:"), mi);
            msg_void(mi, sel(c"release"));
        }

        msg_void_id(status_item, sel(c"setMenu:"), menu);
        msg_void(menu, sel(c"release"));
        objc_autoreleasePoolPop(pool);
    }
}

unsafe extern "C" fn remove_icon_callback(_ctx: *mut c_void) {
    let Some(shared) = SHARED.get() else {
        return;
    };
    let item = shared
        .status_item
        .swap(std::ptr::null_mut(), Ordering::AcqRel);
    if item.is_null() {
        return;
    }
    // SAFETY: runs on the main queue; removes and releases our retained
    // `NSStatusItem`.
    unsafe {
        let pool = objc_autoreleasePoolPush();
        let bar = msg_id(cls(c"NSStatusBar"), sel(c"systemStatusBar"));
        msg_void_id(bar, sel(c"removeStatusItem:"), item);
        msg_void(item, sel(c"release"));
        objc_autoreleasePoolPop(pool);
    }
}
