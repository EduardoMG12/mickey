//! Operating-system interfaces. All `unsafe` code in Mickey lives here.

#[cfg(windows)]
pub mod hooks;
#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(windows)]
pub mod tray;
#[cfg(target_os = "linux")]
pub mod uinput;
#[cfg(windows)]
pub mod windows;
