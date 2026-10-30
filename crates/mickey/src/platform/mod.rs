//! Operating-system interfaces. All `unsafe` code in Mickey lives here.

#[cfg(windows)]
pub mod acl;
#[cfg(windows)]
pub mod elevate;
#[cfg(windows)]
pub mod hooks;
#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(windows)]
pub mod listener;
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

/// Keeps a console program started from Mickey from flashing a console
/// window up on Windows (`CREATE_NO_WINDOW`); nothing elsewhere.
pub fn hide_console(command: &mut std::process::Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    let _ = command;
}
