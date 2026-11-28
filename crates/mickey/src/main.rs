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
mod fs;
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
#[cfg(test)]
mod test_util;
mod threads;
mod transfer;
mod tray;
mod update;
mod whats_new;
mod window;

fn main() {
    // The language the owner picked in config.toml, or the system's, set
    // once before anything else: every message this program writes goes
    // through `i18n::t` and `i18n::tf` from here on.
    let language = mickey_config::Config::load()
        .ok()
        .flatten()
        .map(|config| config.interface.language.clone())
        .unwrap_or_else(|| "auto".into());
    if language == "auto" {
        mickey_config::i18n::set_language(mickey_config::i18n::Lang::system());
    } else if let Some(lang) = mickey_config::i18n::Lang::from_code(&language) {
        mickey_config::i18n::set_language(lang);
    }
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
        .and_then(parse_args)
        .and_then(dispatch);
    if let Err(err) = result {
        eprintln!("mickey: {err}");
        std::process::exit(1);
    }
}

fn parse_args(args: Vec<String>) -> Result<cli::Command, String> {
    #[cfg(target_os = "macos")]
    {
        // Finder / Spotlight launch `Mickey.app/Contents/MacOS/mickey` with no
        // arguments (or a legacy `-psn_*` argument) and no controlling TTY.
        let filtered: Vec<String> = args
            .into_iter()
            .filter(|arg| !arg.starts_with("-psn_"))
            .collect();
        if filtered.is_empty() && launched_as_app() {
            return Ok(cli::Command::Run);
        }
        cli::parse(&filtered)
    }
    #[cfg(not(target_os = "macos"))]
    cli::parse(&args)
}

#[cfg(target_os = "macos")]
fn launched_as_app() -> bool {
    use std::io::IsTerminal;
    if std::io::stdin().is_terminal() {
        return false;
    }
    let Ok(exe) = std::env::current_exe().and_then(std::fs::canonicalize) else {
        return false;
    };
    is_app_bundle_exe(&exe)
}

#[cfg(target_os = "macos")]
fn is_app_bundle_exe(exe: &std::path::Path) -> bool {
    let mut parts = exe.components().rev();
    let _bin = parts.next();
    let macos = parts.next().and_then(|c| c.as_os_str().to_str());
    let contents = parts.next().and_then(|c| c.as_os_str().to_str());
    let bundle = parts.next().and_then(|c| c.as_os_str().to_str());
    macos == Some("MacOS")
        && contents == Some("Contents")
        && bundle.is_some_and(|name| name.ends_with(".app"))
}

fn dispatch(command: cli::Command) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    if matches!(command, cli::Command::Run | cli::Command::Source { .. }) {
        // `mickey run` installs a `sigwait` cleanup thread on the worker; block
        // stop signals on thread 0 first so the AppKit main loop inherits the
        // mask and does not terminate before cleanup runs.
        if matches!(command, cli::Command::Run) {
            let _ = platform::signals::block_stop_signals();
        }
        return platform::macos_tray::run_with_app(move || run::run(command));
    }
    run::run(command)
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::is_app_bundle_exe;
    use std::path::Path;

    #[test]
    fn recognizes_macos_app_bundle_executable_path() {
        assert!(is_app_bundle_exe(Path::new(
            "/Users/alice/Applications/Mickey.app/Contents/MacOS/mickey"
        )));
        assert!(!is_app_bundle_exe(Path::new(
            "/Users/alice/.local/bin/mickey"
        )));
        assert!(!is_app_bundle_exe(Path::new(
            "/Users/alice/Mickey/Contents/MacOS/mickey"
        )));
    }
}
