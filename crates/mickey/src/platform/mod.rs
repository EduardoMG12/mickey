//! Operating-system interfaces. All `unsafe` code in Mickey lives here.

#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(target_os = "linux")]
pub mod uinput;
#[cfg(windows)]
pub mod hooks;
#[cfg(windows)]
pub mod windows;
