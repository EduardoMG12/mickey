//! Operating-system interfaces. All `unsafe` code in Mickey lives here.

#[cfg(windows)]
pub mod acl;
#[cfg(windows)]
pub mod hooks;
#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(windows)]
pub mod log_file;
#[cfg(windows)]
pub mod netif;
#[cfg(windows)]
pub mod output;
#[cfg(target_os = "linux")]
pub mod signals;
#[cfg(windows)]
pub mod tray;
#[cfg(target_os = "linux")]
pub mod uinput;
#[cfg(windows)]
pub mod windows;
