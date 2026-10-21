//! Operating-system interfaces. All `unsafe` code in Mickey lives here.

#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(windows)]
pub mod windows;
