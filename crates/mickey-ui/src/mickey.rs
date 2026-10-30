//! Asking the `mickey` installed next to this program: the settings and
//! history windows read and change things through it.

use std::process::Command;

/// Runs `mickey <args>`: what it printed, or what it said went wrong.
pub fn run(args: &[&str]) -> Result<String, String> {
    let exe = std::env::current_exe().map_err(|err| err.to_string())?;
    let mut command =
        Command::new(exe.with_file_name(format!("mickey{}", std::env::consts::EXE_SUFFIX)));
    command.args(args);
    // mickey.exe is a console program: no console flashing over the window.
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let output = command.output().map_err(|err| err.to_string())?;
    match output.status.success() {
        true => Ok(String::from_utf8_lossy(&output.stdout).into_owned()),
        false => Err(String::from_utf8_lossy(&output.stderr).trim().to_string()),
    }
}

/// `run` for a one-line answer, trimmed.
pub fn ask(args: &[&str]) -> Result<String, String> {
    run(args).map(|text| text.trim().to_string())
}
