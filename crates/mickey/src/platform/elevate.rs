//! Running a program as administrator on Windows: the updater starts the
//! verified installer this way (PRD 0013), and Windows asks the owner once.

use std::io;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;

/// SW_SHOWNORMAL: the installer's own window, if it shows one.
const SW_SHOWNORMAL: i32 = 1;

#[link(name = "shell32")]
unsafe extern "system" {
    fn ShellExecuteW(
        window: isize,
        verb: *const u16,
        file: *const u16,
        parameters: *const u16,
        directory: *const u16,
        show: i32,
    ) -> isize;
}

fn wide(text: &std::ffi::OsStr) -> Vec<u16> {
    text.encode_wide().chain([0]).collect()
}

/// Starts `program` with `parameters`, as administrator (the `runas` verb:
/// Windows shows its UAC prompt). Returns once it started, not when it ends.
pub fn run_as_admin(program: &Path, parameters: &str) -> io::Result<()> {
    let verb = wide("runas".as_ref());
    let file = wide(program.as_os_str());
    let parameters = wide(parameters.as_ref());
    // SAFETY: every string is NUL-terminated UTF-16 that outlives the call;
    // no window and no directory are passed (0 and null).
    let result = unsafe {
        ShellExecuteW(
            0,
            verb.as_ptr(),
            file.as_ptr(),
            parameters.as_ptr(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    // Above 32 is success; the owner saying no to the prompt is 5.
    if result > 32 {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "Windows did not start the installer (code {result}; 5 means the administrator prompt was refused)"
        )))
    }
}
