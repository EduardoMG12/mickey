//! Desktops other than Hyprland, through the RemoteDesktop portal and libei
//! (PRD 0004): GNOME 46 and newer, KDE Plasma 6.1 and newer.
//!
//! The portal asks the person once ("Allow Remote Interaction"); its
//! restore token, kept in Mickey's data folder, spares the question after
//! that, restarts included. libei then hands Mickey an absolute pointer
//! whose regions are this PC's monitors. One session for the whole run,
//! opened when Mickey starts, so the question never comes mid-switch.

use crate::monitors::Monitor;
use crate::threads::LockAnyway;
use ashpd::desktop::{
    CreateSessionOptions, PersistMode,
    remote_desktop::{
        ConnectToEISOptions, DeviceType, RemoteDesktop, SelectDevicesOptions, StartOptions,
    },
};
use futures_util::StreamExt;
use reis::{
    ei,
    event::{DeviceCapability, EiEvent},
};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// The session, once started.
static PORTAL: OnceLock<Portal> = OnceLock::new();

/// When the last try to start it failed: the portal may not be up yet at
/// login (seen on Fedora KDE: Mickey started first), so a later call tries
/// again, at most every `TRY_AGAIN`.
static FAILED_AT: Mutex<Option<Instant>> = Mutex::new(None);
const TRY_AGAIN: Duration = Duration::from_secs(5);

/// How long to wait, at start, for libei to name the monitors.
const FIRST_MONITORS: Duration = Duration::from_secs(5);

pub struct Portal {
    shared: Arc<Mutex<Shared>>,
    context: ei::Context,
}

/// What libei said, kept by the thread that listens to it, and where
/// Mickey put the cursor.
#[derive(Default)]
struct Shared {
    monitors: Vec<Monitor>,
    /// The absolute pointer, once libei lets Mickey use it.
    pointer: Option<Pointer>,
    /// Where Mickey last put the cursor. While another PC controls this
    /// one, only Mickey moves it, so this is where it is: no question to
    /// the compositor (PRD 0004).
    at: Option<(i32, i32)>,
}

struct Pointer {
    device: ei::Device,
    absolute: ei::PointerAbsolute,
    /// From libei's "resumed": every request about the device names it.
    serial: u32,
    emulating: bool,
    sequence: u32,
}

/// The portal's session, started on first use; `None`, logged, when it
/// cannot be yet (no portal running, an older desktop, the person said no).
/// Only one caller starts it; the others get `None` meanwhile.
pub fn get() -> Option<&'static Portal> {
    if let Some(portal) = PORTAL.get() {
        return Some(portal);
    }
    let mut failed_at = FAILED_AT.try_lock().ok()?;
    if failed_at.is_some_and(|at| at.elapsed() < TRY_AGAIN) {
        return None;
    }
    match Portal::start() {
        Ok(portal) => {
            *failed_at = None;
            Some(PORTAL.get_or_init(|| portal))
        }
        Err(err) => {
            eprintln!("mickey: no RemoteDesktop portal: {err}");
            *failed_at = Some(Instant::now());
            None
        }
    }
}

impl Portal {
    fn start() -> Result<Portal, String> {
        let shared = Arc::new(Mutex::new(Shared::default()));
        let (ready, started) = std::sync::mpsc::channel::<Result<ei::Context, String>>();
        let listening = Arc::clone(&shared);
        crate::threads::spawn("portal", move || {
            let result = async_io::block_on(listen(&listening, &ready));
            if let Err(err) = result {
                let _ = ready.send(Err(err.clone()));
                eprintln!("mickey: the RemoteDesktop session ended: {err}");
            }
        });
        // The first start may wait for the person to answer the question.
        let context = started
            .recv()
            .map_err(|_| "the portal thread ended".to_string())??;
        let until = Instant::now() + FIRST_MONITORS;
        while shared.lock_anyway().monitors.is_empty() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(20));
        }
        Ok(Portal { shared, context })
    }

    /// Puts the cursor at `(x, y)` of the desktop, exactly (absolute
    /// motion: no pointer acceleration in between).
    pub fn place(&self, x: i32, y: i32) -> std::io::Result<()> {
        let mut shared = self.shared.lock_anyway();
        let pointer = shared
            .pointer
            .as_mut()
            .ok_or_else(|| std::io::Error::other("libei has not given the pointer yet"))?;
        if !pointer.emulating {
            pointer
                .device
                .start_emulating(pointer.serial, pointer.sequence);
            pointer.sequence += 1;
            pointer.emulating = true;
        }
        pointer.absolute.motion_absolute(x as f32, y as f32);
        pointer.device.frame(pointer.serial, 0);
        shared.at = Some((x, y));
        drop(shared);
        self.context.flush().map_err(std::io::Error::other)
    }

    /// Moves the cursor by `(dx, dy)`, within the monitors (`slide`).
    pub fn move_by(&self, dx: i32, dy: i32) -> std::io::Result<()> {
        let (monitors, at) = {
            let shared = self.shared.lock_anyway();
            (shared.monitors.clone(), shared.at)
        };
        // Never placed yet: start from the middle of the first monitor.
        let at = at.or_else(|| {
            monitors
                .first()
                .map(|m| (m.x + m.width / 2, m.y + m.height / 2))
        });
        let Some(at) = at else {
            return Ok(());
        };
        let (x, y) = slide(&monitors, at, (dx, dy));
        self.place(x, y)
    }

    /// Where Mickey last put the cursor.
    pub fn cursor(&self) -> Option<(i32, i32)> {
        self.shared.lock_anyway().at
    }

    /// This PC's monitors, as libei's absolute pointer regions.
    pub fn monitors(&self) -> Vec<Monitor> {
        self.shared.lock_anyway().monitors.clone()
    }
}

/// Where the restore token is kept: Mickey's data folder, readable only by
/// its owner (it lets a program control this PC without asking).
fn token_path() -> Option<std::path::PathBuf> {
    crate::net::identity::data_dir()
        .ok()
        .map(|dir| dir.join("portal-token"))
}

fn save_token(token: &str) {
    use std::os::unix::fs::OpenOptionsExt;
    let Some(path) = token_path() else { return };
    // On a fresh install the data folder may not exist yet.
    if let Some(dir) = path.parent()
        && let Err(err) = std::fs::create_dir_all(dir)
    {
        eprintln!("mickey: cannot keep the portal's permission: {err}");
        return;
    }
    let written = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&path)
        .and_then(|mut file| std::io::Write::write_all(&mut file, token.as_bytes()));
    if let Err(err) = written {
        eprintln!("mickey: cannot keep the portal's permission: {err}");
    }
}

/// Opens the session, then listens to libei for as long as it lasts.
/// `ready` hears once whether the session started.
async fn listen(
    shared: &Mutex<Shared>,
    ready: &std::sync::mpsc::Sender<Result<ei::Context, String>>,
) -> Result<(), String> {
    let text = |err: ashpd::Error| err.to_string();
    let portal = RemoteDesktop::new().await.map_err(text)?;
    let session = portal
        .create_session(CreateSessionOptions::default())
        .await
        .map_err(text)?;
    let token = token_path().and_then(|path| std::fs::read_to_string(path).ok());
    let options = SelectDevicesOptions::default()
        .set_devices(DeviceType::Keyboard | DeviceType::Pointer)
        .set_persist_mode(PersistMode::ExplicitlyRevoked)
        .set_restore_token(token.as_deref());
    portal
        .select_devices(&session, options)
        .await
        .map_err(text)?;
    let chosen = portal
        .start(&session, None, StartOptions::default())
        .await
        .map_err(text)?
        .response()
        .map_err(|_| "the person did not allow remote interaction".to_string())?;
    if let Some(token) = chosen.restore_token() {
        save_token(token);
    }
    let fd = portal
        .connect_to_eis(&session, ConnectToEISOptions::default())
        .await
        .map_err(text)?;
    let stream = std::os::unix::net::UnixStream::from(fd);
    let context = ei::Context::new(stream).map_err(|err| err.to_string())?;
    let (_connection, mut events) = context
        .handshake_async_io("Mickey", ei::handshake::ContextType::Sender)
        .await
        .map_err(|err| err.to_string())?;
    let _ = ready.send(Ok(context.clone()));

    while let Some(event) = events.next().await {
        match event.map_err(|err| err.to_string())? {
            EiEvent::SeatAdded(added) => {
                added.seat.bind_capabilities(
                    DeviceCapability::PointerAbsolute | DeviceCapability::Keyboard,
                );
            }
            EiEvent::DeviceAdded(added)
                if added
                    .device
                    .has_capability(DeviceCapability::PointerAbsolute) =>
            {
                let monitors = added
                    .device
                    .regions()
                    .iter()
                    .map(|region| Monitor {
                        x: region.x as i32,
                        y: region.y as i32,
                        width: region.width as i32,
                        height: region.height as i32,
                    })
                    .collect();
                shared.lock_anyway().monitors = monitors;
            }
            EiEvent::DeviceResumed(resumed) => {
                if let Some(absolute) = resumed.device.interface::<ei::PointerAbsolute>() {
                    shared.lock_anyway().pointer = Some(Pointer {
                        device: resumed.device.device().clone(),
                        absolute,
                        serial: resumed.serial,
                        emulating: false,
                        sequence: 1,
                    });
                }
            }
            EiEvent::DevicePaused(paused)
                if paused
                    .device
                    .has_capability(DeviceCapability::PointerAbsolute) =>
            {
                shared.lock_anyway().pointer = None;
            }
            _ => {}
        }
        let _ = context.flush();
    }
    // The session object lives until here: dropping it would end the session.
    drop(session);
    Err("libei closed the connection".into())
}

/// Where the cursor goes when it moves by `(dx, dy)` from `at`: there, if
/// that is on a monitor (on another one too, crossing between them);
/// otherwise as far as it can within the monitor it is on, so pushing
/// against an edge stops on the edge's last pixel, where `edges::beyond`
/// sees the push.
pub fn slide(monitors: &[Monitor], at: (i32, i32), (dx, dy): (i32, i32)) -> (i32, i32) {
    let wanted = (at.0.saturating_add(dx), at.1.saturating_add(dy));
    if crate::edges::on_screen(monitors, wanted) {
        return wanted;
    }
    let inside = |m: &&Monitor| crate::edges::on_screen(std::slice::from_ref(*m), at);
    match monitors.iter().find(inside).or(monitors.first()) {
        Some(m) => (
            wanted.0.clamp(m.x, m.x + m.width - 1),
            wanted.1.clamp(m.y, m.y + m.height - 1),
        ),
        None => at,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen(x: i32, y: i32, width: i32, height: i32) -> Monitor {
        Monitor {
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn the_cursor_moves_and_stops_on_the_edge() {
        let one = [screen(0, 0, 1280, 800)];
        assert_eq!(slide(&one, (100, 100), (5, -3)), (105, 97));
        // Pushed past the right edge: on its last pixel, wherever the push.
        assert_eq!(slide(&one, (1275, 400), (20, 0)), (1279, 400));
        assert_eq!(slide(&one, (1279, 400), (4, 2)), (1279, 402));
        // Past a corner.
        assert_eq!(slide(&one, (2, 3), (-50, -50)), (0, 0));
    }

    #[test]
    fn the_cursor_crosses_onto_a_second_monitor() {
        let two = [screen(0, 0, 1280, 800), screen(1280, 0, 1920, 1080)];
        assert_eq!(slide(&two, (1279, 100), (10, 0)), (1289, 100));
        // Below the first monitor but within the second's height.
        assert_eq!(slide(&two, (1300, 900), (0, 50)), (1300, 950));
        // From the second, down past where the first ends: kept on the second.
        assert_eq!(slide(&two, (1300, 1070), (-40, 20)), (1280, 1079));
    }

    /// In a GNOME or KDE session (PRD 0004's VM check): the cursor goes to
    /// an exact point, then stops on the right edge however hard it is
    /// pushed, and Mickey knows where it is without asking.
    #[test]
    #[ignore = "needs a GNOME or KDE session with the RemoteDesktop portal"]
    fn the_portal_places_and_stops_on_the_edge() {
        let portal = get().expect("a RemoteDesktop session");
        let screen = portal.monitors()[0];
        // libei gives the pointer a moment after the session starts.
        let until = Instant::now() + Duration::from_secs(3);
        while portal.place(100, 100).is_err() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert_eq!(portal.cursor(), Some((100, 100)));
        portal.move_by(5000, 0).unwrap();
        let edge = (screen.x + screen.width - 1, 100);
        assert_eq!(portal.cursor(), Some(edge));
        let leaving = crate::edges::beyond(&portal.monitors(), edge, (4, 0));
        assert_eq!(leaving, Some((edge.0 + 1, 100)), "a push past it is seen");
        println!("the cursor is at {edge:?}, on the right edge");
        // Stays there long enough for a screenshot.
        std::thread::sleep(Duration::from_secs(4));
    }
}
