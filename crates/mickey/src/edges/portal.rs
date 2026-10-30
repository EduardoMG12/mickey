//! Screen edges on GNOME and KDE (PRD 0004, phase 2): the InputCapture
//! portal's pointer barriers say when this PC's cursor is pushed against an
//! edge that leads to a placed peer. Mickey then releases the capture at
//! once, a few pixels inside, and switches the focus as it always does
//! (reading input through evdev): libei's input is never read here.
//!
//! GNOME 46's dialog for this has no "remember", so the capture starts only
//! once a peer is placed beside this PC: with no edge to watch, no question.
//!
//! On Sway, which has no InputCapture portal, the same lines become
//! one-pixel layer surfaces instead (`edges::sway`, #178).

use crate::edges::{self, Line};
use crate::monitors::Monitor;
use crate::threads::LockAnyway;
use ashpd::desktop::input_capture::{
    Barrier, BarrierID, Capabilities, CreateSessionOptions, InputCapture, ReleaseOptions,
    StartOptions,
};
use futures_util::{StreamExt, future::Either};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// How often the capture thread looks for a new set of barriers.
const LOOK_AGAIN: Duration = Duration::from_millis(500);

pub struct PortalEdges {
    /// The barriers the source wants; the thread sets them.
    wanted: Arc<Mutex<Vec<Line>>>,
    /// The pixel just off this desktop each push reached.
    hits: Receiver<(i32, i32)>,
    /// On Sway: makes its thread read `wanted` again.
    wake: Option<Box<dyn Fn() + Send>>,
}

impl PortalEdges {
    /// Starts the session (the dialog, the first time in a session) on a
    /// thread of its own.
    pub fn start(here: Vec<Monitor>, barriers: Vec<Line>) -> PortalEdges {
        let wanted = Arc::new(Mutex::new(barriers));
        let (hit, hits) = std::sync::mpsc::channel();
        if crate::desktop::session() == crate::desktop::Session::Sway {
            let wake = crate::edges::sway::start(Arc::clone(&wanted), hit)
                .inspect_err(|err| eprintln!("mickey: no screen edges here: {err}"))
                .ok();
            return PortalEdges { wanted, hits, wake };
        }
        let shared = Arc::clone(&wanted);
        crate::threads::spawn("edge capture", move || {
            if let Err(err) = async_io::block_on(watch(&here, &shared, &hit)) {
                eprintln!("mickey: no screen edges here: {err}");
            }
        });
        PortalEdges {
            wanted,
            hits,
            wake: None,
        }
    }

    /// Asks for these barriers instead (the layout or who is online changed).
    pub fn set(&self, barriers: Vec<Line>) {
        let mut wanted = self.wanted.lock_anyway();
        if *wanted == barriers {
            return;
        }
        *wanted = barriers;
        drop(wanted);
        if let Some(wake) = &self.wake {
            wake();
        }
    }

    /// The pixel off this desktop the cursor last pushed onto, if it did.
    pub fn hit(&self) -> Option<(i32, i32)> {
        self.hits.try_iter().last()
    }
}

/// From where the portal stopped the cursor, the pixel just inside this
/// desktop to put it back on, and the one just off it that it pushed onto.
fn sides(here: &[Monitor], (x, y): (i32, i32)) -> Option<((i32, i32), (i32, i32))> {
    let on = |point| edges::on_screen(here, point);
    if !on((x, y)) {
        // KDE names where the push would have gone, past the barrier (seen:
        // x = 1319 on a 1280-wide screen); GNOME names the pixel on it. Both
        // become the pixel just off the nearest edge.
        let (ex, ey) = nearest(here, (x, y))?;
        let (dx, dy) = if (x - ex).abs() >= (y - ey).abs() {
            ((x - ex).signum(), 0)
        } else {
            (0, (y - ey).signum())
        };
        let off = (ex + dx, ey + dy);
        return Some(((off.0 - 2 * dx, off.1 - 2 * dy), off));
    }
    [(1, 0), (-1, 0), (0, 1), (0, -1)]
        .into_iter()
        .find_map(|(dx, dy)| {
            let (behind, ahead) = ((x - dx, y - dy), (x + dx, y + dy));
            (on(behind) && !on(ahead)).then(|| {
                let off = if on((x, y)) { ahead } else { (x, y) };
                ((x - 2 * dx, y - 2 * dy), off)
            })
        })
}

/// The pixel on this desktop closest to a point off it.
fn nearest(here: &[Monitor], (x, y): (i32, i32)) -> Option<(i32, i32)> {
    here.iter()
        .map(|m| {
            (
                x.clamp(m.x, m.x + m.width - 1),
                y.clamp(m.y, m.y + m.height - 1),
            )
        })
        .min_by_key(|&(cx, cy)| (cx - x).pow(2) + (cy - y).pow(2))
}

async fn watch(
    here: &[Monitor],
    wanted: &Mutex<Vec<Line>>,
    hit: &Sender<(i32, i32)>,
) -> Result<(), String> {
    let text = |err: ashpd::Error| err.to_string();
    let capture = InputCapture::new().await.map_err(text)?;
    let caps = Capabilities::Keyboard | Capabilities::Pointer;
    let session = match capture.create_session2(Default::default()).await {
        Ok(session) => {
            capture
                .start(
                    &session,
                    None,
                    StartOptions::default().set_capabilities(caps),
                )
                .await
                .map_err(text)?
                .response()
                .map_err(|_| "the person did not allow input capture".to_string())?;
            session
        }
        Err(ashpd::Error::RequiresVersion(_, _)) => {
            capture
                .create_session(None, CreateSessionOptions::default().set_capabilities(caps))
                .await
                .map_err(text)?
                .0
        }
        Err(err) => return Err(text(err)),
    };
    // The portal wants an EIS connection before it enables. KDE also waits
    // for libei's handshake on it before it lets go of a captured pointer
    // (seen in the Fedora KDE VM: the pointer froze), so Mickey shakes
    // hands as a receiver and drops whatever comes: its input still comes
    // through evdev.
    let eis = capture
        .connect_to_eis(&session, Default::default())
        .await
        .map_err(text)?;
    crate::threads::spawn("edge capture eis", move || {
        async_io::block_on(drain(eis));
    });
    let mut activated = capture.receive_activated().await.map_err(text)?;
    let mut set: Option<Vec<Line>> = None;
    loop {
        let now = wanted.lock_anyway().clone();
        if set.as_ref() != Some(&now) {
            let zones = capture
                .zones(&session, Default::default())
                .await
                .map_err(text)?
                .response()
                .map_err(text)?;
            let barriers: Vec<Barrier> = now
                .iter()
                .enumerate()
                .filter_map(|(n, line)| Some(Barrier::new(BarrierID::new(n as u32 + 1)?, *line)))
                .collect();
            let answer = capture
                .set_pointer_barriers(&session, &barriers, zones.zone_set(), Default::default())
                .await
                .map_err(text)?
                .response()
                .map_err(text)?;
            if !answer.failed_barriers().is_empty() {
                eprintln!(
                    "mickey: {} edge barriers were refused",
                    answer.failed_barriers().len()
                );
            }
            capture
                .enable(&session, Default::default())
                .await
                .map_err(text)?;
            set = Some(now);
        }
        let next = activated.next();
        let pause = async_io::Timer::after(LOOK_AGAIN);
        let Either::Left((Some(event), _)) =
            futures_util::future::select(next, std::pin::pin!(pause)).await
        else {
            continue;
        };
        let at = event
            .cursor_position()
            .map(|(x, y)| (x.round() as i32, y.round() as i32));
        let (inside, off) = at.and_then(|at| sides(here, at)).unzip();
        // Where the cursor is, never what is typed (hard rule 7).
        eprintln!("mickey: the cursor pushed against an edge at {at:?}");
        // Back to the person at once, just inside the edge.
        let back = inside.map(|(x, y)| (f64::from(x), f64::from(y)));
        let release = ReleaseOptions::default()
            .set_activation_id(event.activation_id())
            .set_cursor_position(back);
        capture.release(&session, release).await.map_err(text)?;
        capture
            .enable(&session, Default::default())
            .await
            .map_err(text)?;
        if let Some(off) = off {
            let _ = hit.send(off);
        }
    }
}

/// Speaks libei on the capture's socket, as a receiver, and drops every
/// event, for as long as the compositor keeps it open.
async fn drain(eis: std::os::fd::OwnedFd) {
    let stream = std::os::unix::net::UnixStream::from(eis);
    let Ok(context) = reis::ei::Context::new(stream) else {
        return;
    };
    let Ok((_connection, mut events)) = context
        .handshake_async_io("Mickey edges", reis::ei::handshake::ContextType::Receiver)
        .await
    else {
        return;
    };
    while let Some(Ok(event)) = events.next().await {
        if let reis::event::EiEvent::SeatAdded(added) = event {
            added.seat.bind_capabilities(
                reis::event::DeviceCapability::Pointer | reis::event::DeviceCapability::Keyboard,
            );
        }
        let _ = context.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hit_is_put_back_inside_and_named_by_the_pixel_beyond() {
        let laptop = [Monitor {
            x: 0,
            y: 0,
            width: 1280,
            height: 800,
        }];
        // Stopped on the right edge's barrier (x = 1280, just outside).
        assert_eq!(
            sides(&laptop, (1280, 288)),
            Some(((1278, 288), (1280, 288)))
        );
        // On the left edge's barrier (x = 0, the first column).
        assert_eq!(sides(&laptop, (0, 288)), Some(((2, 288), (-1, 288))));
        // Top.
        assert_eq!(sides(&laptop, (500, 0)), Some(((500, 2), (500, -1))));
        // KDE: where the push would have gone, past the right edge.
        assert_eq!(
            sides(&laptop, (1319, 428)),
            Some(((1278, 428), (1280, 428)))
        );
        // KDE, past the left edge.
        assert_eq!(sides(&laptop, (-39, 428)), Some(((1, 428), (-1, 428))));
        // In the middle of the screen: no edge.
        assert_eq!(sides(&laptop, (500, 400)), None);
    }
}
