//! Starting Mickey with the system: `mickey autostart on|off|status`, the
//! switch behind "Start Mickey when this PC starts" in the settings. It
//! turns on and off what the installers set up (packaging/): a systemd user
//! service on Linux, a logon task on Windows. When Mickey was copied by
//! hand, `on` makes the same thing, pointing at this `mickey`.

use std::process::Command;

/// Turns starting with the system on or off.
pub fn set(on: bool) -> Result<(), String> {
    imp::set(on)
}

/// Whether Mickey starts with the system.
pub fn is_on() -> bool {
    imp::is_on()
}

fn run(program: &str, args: &[&str]) -> Result<(), String> {
    let output = Command::new(program)
        .args(args)
        .output()
        .map_err(|err| format!("{program}: {err}"))?;
    if output.status.success() {
        Ok(())
    } else {
        let said = String::from_utf8_lossy(&output.stderr);
        let said = if said.trim().is_empty() {
            String::from_utf8_lossy(&output.stdout)
        } else {
            said
        };
        Err(format!("{program} failed: {}", said.trim()))
    }
}

#[cfg(target_os = "linux")]
mod imp {
    use super::run;
    use std::path::PathBuf;

    const UNIT: &str = "mickey.service";

    fn unit_path() -> Result<PathBuf, String> {
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
            .ok_or("neither XDG_CONFIG_HOME nor HOME is set")?;
        Ok(config.join("systemd/user").join(UNIT))
    }

    /// The installer's unit (packaging/linux/mickey.service), for this
    /// `mickey`.
    fn unit(exe: &str) -> String {
        format!(
            "[Unit]\n\
             Description=Mickey: one keyboard and mouse for all your computers\n\
             # The switcher window needs the graphical session's environment.\n\
             PartOf=graphical-session.target\n\
             After=graphical-session.target\n\n\
             [Service]\n\
             ExecStart={exe} run\n\
             Restart=on-failure\n\
             RestartSec=3\n\n\
             [Install]\n\
             WantedBy=graphical-session.target\n"
        )
    }

    pub fn set(on: bool) -> Result<(), String> {
        if on {
            let path = unit_path()?;
            if !path.exists() {
                let exe = std::env::current_exe().map_err(|err| err.to_string())?;
                let dir = path.parent().ok_or("bad unit path")?;
                std::fs::create_dir_all(dir).map_err(|err| err.to_string())?;
                std::fs::write(&path, unit(&exe.to_string_lossy()))
                    .map_err(|err| err.to_string())?;
                run("systemctl", &["--user", "daemon-reload"])?;
            }
            // Enabled only: it starts at the next login, not now, beside
            // the Mickey already running.
            run("systemctl", &["--user", "enable", UNIT])
        } else {
            run("systemctl", &["--user", "disable", UNIT])
        }
    }

    pub fn is_on() -> bool {
        run("systemctl", &["--user", "is-enabled", "--quiet", UNIT]).is_ok()
    }
}

#[cfg(windows)]
mod imp {
    use super::run;

    /// The installer's task (packaging/windows/mickey.nsi).
    const TASK: &str = "Mickey";

    /// A new task does not start on battery and stops when the power cable
    /// is pulled; on a laptop, Mickey would never start. It also has no
    /// time limit. (Same as the installer.)
    const ON_BATTERY: &str = "Set-ScheduledTask -TaskName Mickey -Settings (New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -ExecutionTimeLimit 0) | Out-Null";

    pub fn set(on: bool) -> Result<(), String> {
        let result = if on {
            create()
        } else {
            run("schtasks", &["/delete", "/tn", TASK, "/f"])
        };
        match result {
            Ok(()) => Ok(()),
            // A task that runs as administrator needs an administrator to
            // make or remove it: ask Windows (the UAC prompt) and do it again.
            Err(_) if !elevated() => again_as_administrator(on),
            Err(err) => Err(err),
        }
    }

    pub fn is_on() -> bool {
        run("schtasks", &["/query", "/tn", TASK]).is_ok()
    }

    /// At this user's logon (the account creating it), as administrator (so
    /// it can drive administrator windows), hidden: `start-hidden.vbs` next to this `mickey.exe`, written if
    /// missing.
    fn create() -> Result<(), String> {
        let exe = std::env::current_exe().map_err(|err| err.to_string())?;
        let script = exe.with_file_name("start-hidden.vbs");
        if !script.exists() {
            std::fs::write(
                &script,
                include_str!("../../../packaging/windows/start-hidden.vbs"),
            )
            .map_err(|err| format!("cannot write {}: {err}", script.display()))?;
        }
        let action = format!("wscript.exe \"{}\"", script.display());
        run(
            "schtasks",
            &[
                "/create", "/tn", TASK, "/tr", &action, "/sc", "onlogon", "/it", "/rl", "highest",
                "/f",
            ],
        )?;
        run("powershell", &["-NoProfile", "-Command", ON_BATTERY])
    }

    /// `net session` only works for an administrator.
    fn elevated() -> bool {
        run("net", &["session"]).is_ok()
    }

    fn again_as_administrator(on: bool) -> Result<(), String> {
        let exe = std::env::current_exe().map_err(|err| err.to_string())?;
        let word = if on { "on" } else { "off" };
        let script = format!(
            "$p = Start-Process -FilePath '{}' -ArgumentList 'autostart','{word}' -Verb RunAs -WindowStyle Hidden -Wait -PassThru; exit $p.ExitCode",
            exe.display()
        );
        run("powershell", &["-NoProfile", "-Command", &script]).map_err(|_| {
            "Windows did not allow it (the administrator prompt was refused?)".to_string()
        })
    }
}
