//! Sway (#178): its IPC socket answers what Hyprland answers elsewhere,
//! except where the cursor is. `get_outputs` gives the monitors, and the
//! `seat - cursor set` command puts the cursor at a point. Like the portal
//! on GNOME and KDE, a controlled Sway moves the cursor in absolute steps
//! and remembers where it put it, so it knows when a push leaves the
//! desktop.

use crate::monitors::Monitor;
use crate::threads::LockAnyway;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const RUN_COMMAND: u32 = 0;
const GET_OUTPUTS: u32 = 3;
const MAGIC: &[u8; 6] = b"i3-ipc";

/// One IPC question: the magic, the payload's length and the message
/// kind (both little-endian u32), then the payload; the answer has the
/// same header and a JSON body.
fn ask(kind: u32, payload: &str) -> Result<serde_json::Value, String> {
    let path = std::env::var_os("SWAYSOCK").ok_or("SWAYSOCK is not set")?;
    let mut socket =
        UnixStream::connect(&path).map_err(|err| format!("cannot reach Sway: {err}"))?;
    socket
        .set_read_timeout(Some(Duration::from_secs(2)))
        .map_err(|err| err.to_string())?;
    let mut request = MAGIC.to_vec();
    request.extend((payload.len() as u32).to_le_bytes());
    request.extend(kind.to_le_bytes());
    request.extend(payload.as_bytes());
    socket.write_all(&request).map_err(|err| err.to_string())?;
    let mut header = [0u8; 14];
    socket
        .read_exact(&mut header)
        .map_err(|err| format!("Sway did not answer: {err}"))?;
    if &header[..6] != MAGIC {
        return Err("Sway answered something else".into());
    }
    let length = u32::from_le_bytes([header[6], header[7], header[8], header[9]]) as usize;
    let mut body = vec![0u8; length];
    socket
        .read_exact(&mut body)
        .map_err(|err| err.to_string())?;
    serde_json::from_slice(&body).map_err(|err| format!("Sway's answer is not JSON: {err}"))
}

/// The active outputs, in the logical layout Sway gives them.
pub fn monitors() -> Result<Vec<Monitor>, String> {
    Ok(outputs()?.into_iter().map(|(_, monitor)| monitor).collect())
}

/// The active outputs with their names, which Wayland's `wl_output` gives
/// too: how a layer surface is put on the right one (`sway_edges`).
pub fn outputs() -> Result<Vec<(String, Monitor)>, String> {
    parse_outputs(&ask(GET_OUTPUTS, "")?)
}

fn parse_outputs(outputs: &serde_json::Value) -> Result<Vec<(String, Monitor)>, String> {
    let outputs = outputs.as_array().ok_or("get_outputs is not a list")?;
    let number = |rect: &serde_json::Value, key: &str| {
        rect.get(key)
            .and_then(serde_json::Value::as_i64)
            .map(|n| n as i32)
            .ok_or_else(|| format!("an output's rect has no {key}"))
    };
    let mut monitors = Vec::new();
    for output in outputs {
        if output.get("active").and_then(serde_json::Value::as_bool) != Some(true) {
            continue;
        }
        let rect = output.get("rect").ok_or("an output has no rect")?;
        let name = output
            .get("name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string();
        monitors.push((
            name,
            Monitor {
                x: number(rect, "x")?,
                y: number(rect, "y")?,
                width: number(rect, "width")?,
                height: number(rect, "height")?,
            },
        ));
    }
    if monitors.is_empty() {
        return Err("Sway has no active output".into());
    }
    Ok(monitors)
}

/// Where Mickey last put the cursor, and the monitors it moves within
/// (read again once a second: one question per motion would be wasted).
struct Tracked {
    at: Option<(i32, i32)>,
    monitors: Vec<Monitor>,
    read: Option<Instant>,
}

/// When Mickey last put the cursor somewhere. Sway answers a
/// `cursor set` onto an edge strip as if the cursor had left it and come
/// back, which must not count as a push (`sway_edges`).
static PLACED: Mutex<Option<Instant>> = Mutex::new(None);

/// Whether Mickey put the cursor somewhere in the last `within`.
pub fn placed_within(within: Duration) -> bool {
    PLACED.lock_anyway().is_some_and(|at| at.elapsed() < within)
}

static TRACKED: Mutex<Tracked> = Mutex::new(Tracked {
    at: None,
    monitors: Vec::new(),
    read: None,
});

/// Puts the cursor at `x, y` of the desktop, exactly.
pub fn place(x: i32, y: i32) -> Result<(), String> {
    *PLACED.lock_anyway() = Some(Instant::now());
    let answer = ask(RUN_COMMAND, &format!("seat - cursor set {x} {y}"))?;
    let done = answer
        .get(0)
        .and_then(|first| first.get("success"))
        .and_then(serde_json::Value::as_bool);
    if done != Some(true) {
        return Err(format!("Sway refused to place the cursor: {answer}"));
    }
    TRACKED.lock_anyway().at = Some((x, y));
    Ok(())
}

/// Moves the cursor by `(dx, dy)`, within the monitors (`portal::slide`).
pub fn move_by(dx: i32, dy: i32) -> Result<(), String> {
    let (monitors, at) = {
        let mut tracked = TRACKED.lock_anyway();
        if tracked
            .read
            .is_none_or(|read| read.elapsed() > Duration::from_secs(1))
        {
            tracked.monitors = monitors().unwrap_or_default();
            tracked.read = Some(Instant::now());
        }
        (tracked.monitors.clone(), tracked.at)
    };
    // Never placed yet: start from the middle of the first monitor.
    let Some(at) = at.or_else(|| {
        monitors
            .first()
            .map(|m| (m.x + m.width / 2, m.y + m.height / 2))
    }) else {
        return Ok(());
    };
    let (x, y) = crate::desktop::portal::slide(&monitors, at, (dx, dy));
    place(x, y)
}

/// Where Mickey last put the cursor.
pub fn cursor() -> Option<(i32, i32)> {
    TRACKED.lock_anyway().at
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_active_outputs_count() {
        let outputs = serde_json::json!([
            {"name": "eDP-1", "active": true, "rect": {"x": 0, "y": 0, "width": 1536, "height": 864}},
            {"name": "HDMI-A-1", "active": false, "rect": {"x": 0, "y": 0, "width": 0, "height": 0}},
            {"name": "DP-1", "active": true, "rect": {"x": 1536, "y": -200, "width": 1920, "height": 1080}}
        ]);
        let monitors = parse_outputs(&outputs).unwrap();
        assert_eq!(monitors.len(), 2);
        assert_eq!(monitors[1].0, "DP-1");
        assert_eq!(
            monitors[1].1,
            Monitor {
                x: 1536,
                y: -200,
                width: 1920,
                height: 1080
            }
        );
        assert!(parse_outputs(&serde_json::json!([])).is_err());
    }
}
