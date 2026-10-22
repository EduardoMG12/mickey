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

fn run(program: impl AsRef<std::ffi::OsStr>, args: &[&str]) -> Result<(), String> {
    let program = program.as_ref();
    let shown = std::path::Path::new(program).display().to_string();
    let output = Command::new(program)
        .args(args)
        .output()
        .map_err(|err| format!("{shown}: {err}"))?;
    if output.status.success() {
        Ok(())
    } else {
        let said = String::from_utf8_lossy(&output.stderr);
        let said = if said.trim().is_empty() {
            String::from_utf8_lossy(&output.stdout)
        } else {
            said
        };
        Err(format!("{shown} failed: {}", said.trim()))
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

/// A Windows tool by its full path, never one dropped next to
/// `mickey.exe` (Windows looks in the program's folder first).
#[cfg(windows)]
pub fn system_tool(tool: &str) -> std::path::PathBuf {
    let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
    let system32 = std::path::Path::new(&root).join("System32");
    match tool {
        "powershell.exe" => system32.join("WindowsPowerShell\\v1.0\\powershell.exe"),
        _ => system32.join(tool),
    }
}

#[cfg(windows)]
mod imp {
    use super::run;
    use super::system_tool as system;

    /// The installer's task (packaging/windows/mickey.nsi).
    const TASK: &str = "Mickey";

    /// A new task does not start on battery and stops when the power cable
    /// is pulled; on a laptop, Mickey would never start. It also has no
    /// time limit. (Same as the installer.)
    const ON_BATTERY: &str = "Set-ScheduledTask -TaskName Mickey -Settings (New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -ExecutionTimeLimit 0) | Out-Null";

    /// Exits 1 if anyone but Administrators, SYSTEM, TrustedInstaller or
    /// CREATOR OWNER may change one of `MICKEY_PATHS` (separated by `|`):
    /// write, append, attributes, delete, permissions, owner, or generic
    /// write/all. Names differ per language, so it compares SIDs.
    const ADMIN_ONLY: &str = concat!(
        "$ok = 'S-1-5-32-544','S-1-5-18','S-1-3-0',",
        "'S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464'; ",
        "foreach ($path in $env:MICKEY_PATHS.Split('|')) { ",
        "(Get-Acl -LiteralPath $path).Access | Where-Object { ",
        "$_.AccessControlType -eq 'Allow' -and ([int]$_.FileSystemRights -band 0x500D0116) } | ",
        "ForEach-Object { $sid = try { $_.IdentityReference.Translate(",
        "[Security.Principal.SecurityIdentifier]).Value } catch { $_.IdentityReference.Value }; ",
        "if ($ok -notcontains $sid) { exit 1 } } }; exit 0",
    );

    pub fn set(on: bool) -> Result<(), String> {
        let result = if on {
            create()
        } else {
            run(system("schtasks.exe"), &["/delete", "/tn", TASK, "/f"])
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
        run(system("schtasks.exe"), &["/query", "/tn", TASK]).is_ok()
    }

    /// At this user's logon (the account creating it), hidden, through
    /// `start-hidden.vbs` next to this `mickey.exe` (written every time, so
    /// nobody's copy is ever kept). As administrator, so it can drive
    /// administrator windows, but only when nobody else can change the
    /// files that would run that way: otherwise any program could replace
    /// them and become administrator at the next logon.
    fn create() -> Result<(), String> {
        let exe = std::env::current_exe().map_err(|err| err.to_string())?;
        let script = exe.with_file_name("start-hidden.vbs");
        std::fs::write(
            &script,
            include_str!("../../../packaging/windows/start-hidden.vbs"),
        )
        .map_err(|err| format!("cannot write {}: {err}", script.display()))?;
        let ui = exe.with_file_name("mickey-ui.exe");
        let dir = exe.parent().ok_or("mickey.exe has no folder")?;
        let paths = [dir, exe.as_path(), script.as_path(), ui.as_path()]
            .iter()
            .filter(|path| path.exists())
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join("|");
        let safe = std::process::Command::new(system("powershell.exe"))
            .args(["-NoProfile", "-Command", ADMIN_ONLY])
            .env("MICKEY_PATHS", paths)
            .status()
            .is_ok_and(|status| status.success());
        let level = if safe {
            "highest"
        } else {
            eprintln!(
                "mickey: {} can be changed by any user, so Mickey will start without administrator rights (it cannot drive administrator windows); install it with the installer, or make the folder writable by administrators only",
                dir.display()
            );
            "limited"
        };
        let action = format!(
            "{} \"{}\"",
            system("wscript.exe").display(),
            script.display()
        );
        run(
            system("schtasks.exe"),
            &[
                "/create", "/tn", TASK, "/tr", &action, "/sc", "onlogon", "/it", "/rl", level, "/f",
            ],
        )?;
        run(
            system("powershell.exe"),
            &["-NoProfile", "-Command", ON_BATTERY],
        )
    }

    /// `net session` only works for an administrator.
    fn elevated() -> bool {
        run(system("net.exe"), &["session"]).is_ok()
    }

    fn again_as_administrator(on: bool) -> Result<(), String> {
        let exe = std::env::current_exe().map_err(|err| err.to_string())?;
        let word = if on { "on" } else { "off" };
        // The path goes in through the environment, never inside the
        // command text, where a quote in it would end the string.
        let script = format!(
            "$p = Start-Process -FilePath $env:MICKEY_EXE -ArgumentList 'autostart','{word}' -Verb RunAs -WindowStyle Hidden -Wait -PassThru; exit $p.ExitCode"
        );
        let status = std::process::Command::new(system("powershell.exe"))
            .args(["-NoProfile", "-Command", &script])
            .env("MICKEY_EXE", &exe)
            .status();
        match status {
            Ok(status) if status.success() => Ok(()),
            _ => {
                Err("Windows did not allow it (the administrator prompt was refused?)".to_string())
            }
        }
    }
}
