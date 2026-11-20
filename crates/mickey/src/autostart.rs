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

/// Starts Mickey now, as logging in does (`mickey start`): after "Quit
/// Mickey", opening the settings brings the PCs back.
pub fn start() -> Result<(), String> {
    imp::start()
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

    pub fn start() -> Result<(), String> {
        run("systemctl", &["--user", "start", UNIT])
    }
}

/// A Windows tool by its full path, never one dropped next to
/// `mickey.exe` (Windows looks in the program's folder first).
/// Whether only administrators can change this Mickey (Windows; see
/// `imp::install_is_safe`), so it may run as administrator.
#[cfg(windows)]
pub fn install_is_safe() -> bool {
    imp::install_is_safe()
}

/// Mickey runs as administrator from a folder others can change (a logon
/// task made before #247): its logon task is made again without
/// administrator rights and started, for this one to end.
#[cfg(windows)]
pub fn step_down() -> Result<(), String> {
    if !is_on() {
        return Err("Mickey runs as administrator from a folder other users can change; run it without administrator rights, or install it with the installer".into());
    }
    imp::set(true)?;
    run(system_tool("schtasks.exe"), &["/run", "/tn", "Mickey"])
}

#[cfg(windows)]
pub fn system_tool(tool: &str) -> std::path::PathBuf {
    let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
    let system32 = std::path::Path::new(&root).join("System32");
    match tool {
        "powershell.exe" => system32.join("WindowsPowerShell\\v1.0\\powershell.exe"),
        _ => system32.join(tool),
    }
}

/// The logon task as Task Scheduler's XML: at the logon of `user` (a SID),
/// in their session, run `ui` with `start-mickey`; as administrator if `highest`.
/// Allowed on battery, never stopped for it, and with no time limit: on a
/// laptop Mickey would otherwise never start.
#[cfg_attr(not(any(windows, test)), expect(dead_code, reason = "Windows only"))]
fn task_xml(user: &str, ui: &str, highest: bool) -> String {
    let escape = |text: &str| {
        text.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
    };
    let (user, ui) = (escape(user), escape(ui));
    let level = if highest {
        "HighestAvailable"
    } else {
        "LeastPrivilege"
    };
    format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <Triggers>
    <LogonTrigger>
      <Enabled>true</Enabled>
      <UserId>{user}</UserId>
    </LogonTrigger>
  </Triggers>
  <Principals>
    <Principal id="Author">
      <UserId>{user}</UserId>
      <LogonType>InteractiveToken</LogonType>
      <RunLevel>{level}</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
    <Enabled>true</Enabled>
  </Settings>
  <Actions Context="Author">
    <Exec>
      <Command>{ui}</Command>
      <Arguments>start-mickey</Arguments>
    </Exec>
  </Actions>
</Task>
"#
    )
}

#[cfg(windows)]
mod imp {
    use super::run;
    use super::system_tool as system;

    /// The installer's task (packaging/windows/mickey.nsi).
    const TASK: &str = "Mickey";

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

    pub fn start() -> Result<(), String> {
        run(system("schtasks.exe"), &["/run", "/tn", TASK])
    }

    /// At this user's logon (the account creating it), hidden: the task
    /// runs `mickey-ui.exe start-mickey`, a program with no console, which
    /// starts `mickey.exe run` without one (#419: the `wscript` and `.vbs`
    /// command line used before is what Defender flagged). As
    /// administrator, so it can drive administrator windows, but only when
    /// nobody else can change the folder and the files that would run that
    /// way: otherwise any program could replace them and become
    /// administrator at the next logon.
    ///
    /// The folder must also be inside Program Files: a folder only
    /// administrators may change can still be renamed away, and replaced,
    /// by whoever may change the folder it is in. Inside Program Files,
    /// nobody but administrators can.
    ///
    /// The task is described in an XML file, removed after: as
    /// administrator in Windows' temp folder, where nobody else can swap
    /// it before `schtasks` reads it, even when the install folder is the
    /// user's to change.
    fn create() -> Result<(), String> {
        let exe = std::env::current_exe().map_err(|err| err.to_string())?;
        let dir = exe.parent().ok_or("mickey.exe has no folder")?;
        // As administrator, Mickey also needs Windows to refuse users'
        // junctions (main turned that on); a process without those rights
        // fails to make a "highest" task anyway, and asks again as one.
        let elevated = crate::platform::acl::elevated();
        let highest =
            install_is_safe() && (!elevated || crate::platform::acl::user_junctions_refused());
        if !highest {
            eprintln!(
                "mickey: {} is not inside Program Files or can be changed by any user, so Mickey will start without administrator rights (it cannot drive administrator windows); install it with the installer",
                dir.display()
            );
        }
        let user = crate::platform::acl::user_sid()
            .map_err(|err| format!("cannot tell which user the task is for: {err}"))?;
        let ui = exe.with_file_name("mickey-ui.exe");
        let xml = super::task_xml(&user, &ui.display().to_string(), highest);
        // A new file with a name nobody can guess, never one already there
        // (a link someone planted). As administrator, in Windows' own temp
        // folder, where nobody else can change or remove what an
        // administrator made: the install folder may be the user's to
        // change, and schtasks, run as administrator, reads it. Otherwise
        // next to this program: swapping it would give nothing.
        let random = crate::net::identity::random_bytes()?;
        let name = format!("mickey-task-{}.xml", crate::hex::encode(&random[..8]));
        let dir = if crate::platform::acl::elevated() {
            let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
            std::path::Path::new(&root).join("Temp")
        } else {
            dir.to_path_buf()
        };
        let file = dir.join(name);
        // schtasks reads UTF-16, with its byte order mark.
        let bytes: Vec<u8> = std::iter::once(0xFEFF)
            .chain(xml.encode_utf16())
            .flat_map(u16::to_le_bytes)
            .collect();
        let written = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&file)
            .and_then(|mut new| std::io::Write::write_all(&mut new, &bytes));
        written.map_err(|err| format!("cannot write {}: {err}", file.display()))?;
        let path = file.display().to_string();
        let made = run(
            system("schtasks.exe"),
            &["/create", "/tn", TASK, "/xml", &path, "/f"],
        );
        let _ = std::fs::remove_file(&file);
        made
    }

    /// Whether only administrators can change this Mickey: its folder is
    /// inside Program Files, and the folder and the files the logon task
    /// runs are theirs alone. Read from Windows directly: a program that
    /// runs PowerShell to read permissions, then makes an elevated logon
    /// task, looks like malware to Windows Defender, which flagged it.
    pub fn install_is_safe() -> bool {
        let Ok(exe) = std::env::current_exe() else {
            return false;
        };
        let Some(dir) = exe.parent() else {
            return false;
        };
        let ui = exe.with_file_name("mickey-ui.exe");
        in_program_files(dir)
            && [dir, exe.as_path(), ui.as_path()]
                .iter()
                .filter(|path| path.exists())
                .all(|path| crate::platform::acl::admin_only(path).unwrap_or(false))
    }

    /// Whether `dir` is inside Program Files, both with links and junctions
    /// followed (a junction inside could point anywhere).
    fn in_program_files(dir: &std::path::Path) -> bool {
        let real = |path: &std::path::Path| std::fs::canonicalize(path).ok();
        let program_files = crate::platform::acl::program_files().ok();
        match (real(dir), program_files.as_deref().and_then(real)) {
            (Some(dir), Some(root)) => dir.starts_with(root),
            _ => false,
        }
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

    #[cfg(test)]
    mod tests {
        #[test]
        fn only_folders_inside_program_files_count() {
            let root = crate::platform::acl::program_files().unwrap();
            assert!(super::in_program_files(&root.join("Common Files")));
            assert!(!super::in_program_files(std::path::Path::new("C:\\Users")));
            // `..` is resolved before comparing.
            assert!(!super::in_program_files(&root.join("..\\Users")));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_logon_task_runs_the_starter_with_no_script() {
        let xml = task_xml(
            "S-1-5-21-1-2-3-1001",
            "C:\\Program Files\\Mickey\\mickey-ui.exe",
            true,
        );
        assert!(xml.contains("<Command>C:\\Program Files\\Mickey\\mickey-ui.exe</Command>"));
        assert!(xml.contains("<Arguments>start-mickey</Arguments>"));
        assert!(xml.contains("<RunLevel>HighestAvailable</RunLevel>"));
        assert!(xml.contains("<UserId>S-1-5-21-1-2-3-1001</UserId>"));
        assert!(xml.contains("<DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>"));
        assert!(!xml.contains("wscript") && !xml.contains(".vbs"));
        let limited = task_xml("ana", "C:\\M\\mickey-ui.exe", false);
        assert!(limited.contains("<RunLevel>LeastPrivilege</RunLevel>"));
    }

    #[test]
    fn a_name_cannot_add_to_the_task() {
        let xml = task_xml("ana</UserId><RunLevel>x", "C:\\a&b\\mickey-ui.exe", false);
        assert!(xml.contains("ana&lt;/UserId&gt;&lt;RunLevel&gt;x"));
        assert!(xml.contains("C:\\a&amp;b\\mickey-ui.exe"));
        assert_eq!(xml.matches("<RunLevel>").count(), 1);
    }
}
