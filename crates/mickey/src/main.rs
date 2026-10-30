//! Mickey: one keyboard and mouse shared across computers on the LAN.

mod audio;
mod autostart;
mod cli;
mod clipboard;
mod control;
#[cfg(target_os = "linux")]
mod desktop;
mod edges;
mod focus;
#[cfg(test)]
mod fuzz;
mod hex;
mod input;
mod monitors;
mod net;
// The Linux build never translates keys for Windows.
mod platform;
mod run;
mod source;
mod target;
mod threads;
mod tray;
mod update;
mod window;

fn main() {
    // Before anything is written: as administrator, never through a
    // junction a user made (see `refuse_user_junctions`).
    #[cfg(windows)]
    let junctions = !platform::acl::elevated() || platform::acl::refuse_user_junctions().is_ok();
    // Started hidden at logon (no console): what Mickey prints goes to a
    // log file, after the line above so it too is written safely.
    #[cfg(windows)]
    platform::log_file::when_hidden();
    #[cfg(windows)]
    if !junctions {
        eprintln!("mickey: cannot refuse users' junctions");
    }
    let result = std::env::args_os()
        .skip(1)
        .map(|arg| {
            arg.into_string()
                .map_err(|arg| format!("argument is not valid UTF-8: {}", arg.display()))
        })
        .collect::<Result<Vec<String>, String>>()
        .and_then(|args| cli::parse(&args))
        .and_then(run::run);
    if let Err(err) = result {
        eprintln!("mickey: {err}");
        std::process::exit(1);
    }
}
