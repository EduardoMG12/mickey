//! Desktops other than Hyprland, through the RemoteDesktop portal and libei
//! (PRD 0004): GNOME 46 and newer, KDE Plasma 6.1 and newer.
//!
//! The portal asks the person once ("Allow Remote Interaction"); its
//! restore token, kept in Mickey's data folder, spares the question after
//! that, restarts included. libei then hands Mickey an absolute pointer
//! whose regions are this PC's monitors. One session for the whole run,
//! opened when Mickey starts, so the question never comes mid-switch.

use crate::monitors::Monitor;
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

/// The session, once started; `None` when this desktop has no portal for it.
static PORTAL: OnceLock<Option<Portal>> = OnceLock::new();

/// How long to wait, at start, for libei to name the monitors.
const FIRST_MONITORS: Duration = Duration::from_secs(5);

pub struct Portal {
    shared: Arc<Mutex<Shared>>,
}

/// What libei said, kept by the thread that listens to it.
#[derive(Default)]
struct Shared {
    monitors: Vec<Monitor>,
}

/// The portal's session, started on first use; `None`, logged once, when
/// it cannot be (no portal, an older desktop, the person said no).
pub fn get() -> Option<&'static Portal> {
    PORTAL
        .get_or_init(|| {
            Portal::start()
                .inspect_err(|err| eprintln!("mickey: no RemoteDesktop portal: {err}"))
                .ok()
        })
        .as_ref()
}

impl Portal {
    fn start() -> Result<Portal, String> {
        let shared = Arc::new(Mutex::new(Shared::default()));
        let (ready, started) = std::sync::mpsc::channel();
        let listening = Arc::clone(&shared);
        crate::threads::spawn("portal", move || {
            let result = async_io::block_on(listen(&listening, &ready));
            if let Err(err) = result {
                let _ = ready.send(Err(err.clone()));
                eprintln!("mickey: the RemoteDesktop session ended: {err}");
            }
        });
        // The first start may wait for the person to answer the question.
        started
            .recv()
            .map_err(|_| "the portal thread ended".to_string())??;
        let until = Instant::now() + FIRST_MONITORS;
        while shared
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .monitors
            .is_empty()
            && Instant::now() < until
        {
            std::thread::sleep(Duration::from_millis(20));
        }
        Ok(Portal { shared })
    }

    /// This PC's monitors, as libei's absolute pointer regions.
    pub fn monitors(&self) -> Vec<Monitor> {
        self.shared
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .monitors
            .clone()
    }
}

/// Where the restore token is kept: Mickey's data folder, readable only by
/// its owner (it lets a program control this PC without asking).
fn token_path() -> Option<std::path::PathBuf> {
    crate::identity::data_dir()
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
    ready: &std::sync::mpsc::Sender<Result<(), String>>,
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
    let _ = ready.send(Ok(()));

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
                shared.lock().unwrap_or_else(|p| p.into_inner()).monitors = monitors;
            }
            _ => {}
        }
        let _ = context.flush();
    }
    // The session object lives until here: dropping it would end the session.
    drop(session);
    Err("libei closed the connection".into())
}
