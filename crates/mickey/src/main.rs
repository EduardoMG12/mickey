//! Mickey: one keyboard and mouse shared across computers on the LAN.

mod audio;
mod audio_wire;
mod autostart;
#[cfg(target_os = "linux")]
mod capture;
mod cli;
mod clip_sync;
mod clip_watch;
mod clip_wire;
mod clipboard;
mod control;
#[cfg(target_os = "linux")]
mod desktop;
mod discovery;
mod edges;
mod emit;
mod event;
mod focus;
#[cfg(test)]
mod fuzz;
mod group;
mod held;
mod hex;
mod history;
mod hotkey;
mod identity;
mod input;
mod jitter;
mod latency;
mod mcp;
mod monitors;
// The Linux build never translates keys for Windows.
#[cfg(target_os = "linux")]
mod edge_capture;
#[cfg_attr(not(windows), allow(dead_code))]
mod keymap;
mod platform;
#[cfg(target_os = "linux")]
mod portal;
mod protocol;
mod received;
mod record;
mod run;
mod secure;
mod sound;
mod source;
mod speakers;
#[cfg(target_os = "linux")]
mod sway;
#[cfg(target_os = "linux")]
mod sway_edges;
mod target;
mod threads;
mod transport;
mod tray;
mod tray_picture;
mod trust;
mod update;
mod window;
#[cfg(target_os = "linux")]
mod x11;

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
