//! The tray icon (PRD 0009): the rat with a green, red or grey dot, a
//! tooltip naming the PC in control, and a menu to move the keyboard, open
//! the settings or quit. On Linux it is a StatusNotifierItem (Waybar's
//! tray, KDE, GNOME with the AppIndicator extension), through `ksni`; on
//! Windows an icon in the notification area (`platform::tray`).

use crate::control::Status;
use crate::tray_picture::{self, Dot};
use std::sync::mpsc::Sender;

/// What the owner picked in the menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Click {
    /// Bring the keyboard and mouse to this PC.
    Home,
    /// Send them to this peer.
    Go(usize),
    Settings,
    /// Open the clipboard history window.
    History,
    Quit,
}

/// The dot and the tooltip for a status.
pub fn looks(status: &Status) -> (Dot, String) {
    if !status.peers.iter().any(|(_, online)| *online) {
        return (Dot::Alone, "No other PC connected".into());
    }
    if status.here {
        return (Dot::Here, "In control: this PC".into());
    }
    let there = status
        .focus
        .map_or("another PC", |peer| status.peers[peer].0.as_str());
    (Dot::Elsewhere, format!("In control: {there}"))
}

#[cfg(target_os = "linux")]
struct Icon {
    status: Status,
    clicks: Sender<Click>,
}

#[cfg(target_os = "linux")]
impl ksni::Tray for Icon {
    fn id(&self) -> String {
        "mickey".into()
    }

    fn title(&self) -> String {
        "Mickey".into()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        let (dot, _) = looks(&self.status);
        let rgba = tray_picture::picture(dot, false);
        // The protocol wants ARGB, byte by byte.
        let (pixels, _) = rgba.as_chunks::<4>();
        let argb = pixels
            .iter()
            .flat_map(|&[r, g, b, a]| [a, r, g, b])
            .collect();
        let size = tray_picture::SIZE as i32;
        vec![ksni::Icon {
            width: size,
            height: size,
            data: argb,
        }]
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            title: "Mickey".into(),
            description: looks(&self.status).1,
            ..Default::default()
        }
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::StandardItem;
        let item = |label: String, enabled: bool, click: Click| {
            StandardItem {
                label,
                enabled,
                // Only a send: the menu must not wait on Mickey's loop.
                activate: Box::new(move |icon: &mut Icon| {
                    let _ = icon.clicks.send(click);
                }),
                ..Default::default()
            }
            .into()
        };
        let mut menu = vec![item("This PC".into(), true, Click::Home)];
        for (index, (name, online)) in self.status.peers.iter().enumerate() {
            menu.push(item(name.clone(), *online, Click::Go(index)));
        }
        menu.push(ksni::MenuItem::Separator);
        menu.push(item("Clipboard history".into(), true, Click::History));
        menu.push(item("Settings".into(), true, Click::Settings));
        menu.push(item("Quit Mickey".into(), true, Click::Quit));
        menu
    }

    // No tray (yet): Mickey went on waiting for one (`assume_sni_available`).
    fn watcher_offline(&self, _reason: ksni::OfflineReason) -> bool {
        eprintln!("mickey: no tray yet; the icon shows when the desktop's tray starts");
        true
    }

    fn watcher_online(&self) {
        eprintln!("mickey: a tray started; the icon is in it");
    }
}

/// The icon in the tray, while this lives.
#[cfg(target_os = "linux")]
pub struct Tray(ksni::blocking::Handle<Icon>);

#[cfg(target_os = "linux")]
impl Tray {
    /// Shows the icon; picks in its menu arrive on `clicks`. A desktop
    /// whose tray is not up yet (a panel that starts after Mickey at login,
    /// or restarts) gets the icon when its tray appears (#225); only a
    /// session with no D-Bus fails.
    pub fn start(status: Status, clicks: Sender<Click>) -> Result<Tray, String> {
        use ksni::blocking::TrayMethods;
        Icon { status, clicks }
            .assume_sni_available(true)
            .spawn()
            .map(Tray)
            .map_err(|err| format!("no tray to show the icon in: {err}"))
    }

    pub fn show(&self, status: &Status) {
        self.0.update(|icon| icon.status = status.clone());
    }
}

/// The icon in the notification area, while this lives.
#[cfg(windows)]
pub struct Tray(crate::platform::tray::Icon);

/// Menu ids on Windows: 1 is this PC, 2 and up the peers in order.
#[cfg(windows)]
const SETTINGS: u32 = 1000;
#[cfg(windows)]
const QUIT: u32 = 1001;
#[cfg(windows)]
const HISTORY: u32 = 1002;

#[cfg(windows)]
impl Tray {
    pub fn start(status: Status, clicks: Sender<Click>) -> Result<Tray, String> {
        // `Sender` may not be shared between threads, so it sits in a lock.
        let clicks = std::sync::Mutex::new(clicks);
        let picked = move |id: u32| {
            let click = match id {
                SETTINGS => Click::Settings,
                HISTORY => Click::History,
                QUIT => Click::Quit,
                1 => Click::Home,
                peer => Click::Go(peer as usize - 2),
            };
            let _ = clicks.lock().unwrap_or_else(|p| p.into_inner()).send(click);
        };
        let icon = crate::platform::tray::Icon::start(look(&status), Box::new(picked))
            .map(Tray)
            .map_err(|err| format!("no tray icon: {err}"))?;
        std::thread::spawn(keep_in_sight);
        Ok(icon)
    }

    pub fn show(&self, status: &Status) {
        self.0.show(look(status));
    }
}

/// Windows 11 hides every new tray icon behind the ^ arrow, where the
/// owner never sees Mickey's status. Explorer notes each icon under
/// `HKCU\Control Panel\NotifyIconSettings`, a moment after it appears;
/// Mickey marks its own as shown (`IsPromoted = 1`), unless the owner
/// already chose (the value exists), so hiding it on purpose sticks.
#[cfg(windows)]
fn keep_in_sight() {
    const SCRIPT: &str = concat!(
        r"Get-ChildItem 'HKCU:\Control Panel\NotifyIconSettings' -ErrorAction SilentlyContinue | ",
        r"Where-Object { $p = Get-ItemProperty $_.PSPath; ",
        r"$p.ExecutablePath -eq $env:MICKEY_EXE -and $null -eq $p.IsPromoted } | ",
        r"ForEach-Object { Set-ItemProperty $_.PSPath -Name IsPromoted -Value 1 -Type DWord }",
    );
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    // Explorer may take a few seconds to note a new icon.
    for _ in 0..6 {
        std::thread::sleep(std::time::Duration::from_secs(5));
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW: no console flashing up.
        let _ = std::process::Command::new("powershell")
            .args(["-NoProfile", "-Command", SCRIPT])
            .env("MICKEY_EXE", &exe)
            .creation_flags(0x0800_0000)
            .output();
    }
}

#[cfg(windows)]
fn look(status: &Status) -> crate::platform::tray::Look {
    use crate::platform::tray::{Look, MenuItem};
    let (dot, tip) = looks(status);
    let item = |label: &str, enabled: bool, id: u32| MenuItem {
        label: label.to_string(),
        enabled,
        id,
    };
    let mut menu = vec![item("This PC", true, 1)];
    for (index, (name, online)) in status.peers.iter().enumerate() {
        menu.push(item(name, *online, index as u32 + 2));
    }
    menu.push(item("", true, 0));
    menu.push(item("Clipboard history", true, HISTORY));
    menu.push(item("Settings", true, SETTINGS));
    menu.push(item("Quit Mickey", true, QUIT));
    Look {
        picture: tray_picture::picture(dot, false),
        tip: format!("Mickey · {tip}"),
        menu,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(focus: Option<usize>, here: bool, online: bool) -> Status {
        Status {
            focus,
            here,
            peers: vec![("Windows".into(), online)],
        }
    }

    #[test]
    fn the_dot_follows_the_keyboard() {
        assert_eq!(looks(&status(None, true, true)).0, Dot::Here);
        assert_eq!(
            looks(&status(Some(0), false, true)),
            (Dot::Elsewhere, "In control: Windows".into())
        );
        // Another PC's keyboard left for a third PC.
        assert_eq!(
            looks(&status(None, false, true)).1,
            "In control: another PC"
        );
        assert_eq!(looks(&status(None, true, false)).0, Dot::Alone);
    }
}
