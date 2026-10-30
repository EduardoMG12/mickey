//! An X11 session (Mint's Cinnamon, Xubuntu's XFCE, GNOME on Xorg; #178):
//! the X server answers what Hyprland answers elsewhere. Its monitors
//! (RandR), where the pointer is (QueryPointer) and putting it at a point
//! (WarpPointer), over one connection kept for the whole run. Input still
//! goes in through uinput, which X reads like any other device.

use crate::monitors::Monitor;
use crate::threads::LockAnyway;
use std::sync::{Mutex, OnceLock};
use x11rb::connection::Connection;
use x11rb::protocol::randr::ConnectionExt as _;
use x11rb::protocol::xproto::{ConnectionExt as _, Window};
use x11rb::rust_connection::RustConnection;

/// The connection and the root window, made on first use. A failed
/// connection is kept as the error, so each question does not try again.
fn server() -> Result<&'static Mutex<(RustConnection, Window)>, String> {
    static SERVER: OnceLock<Result<Mutex<(RustConnection, Window)>, String>> = OnceLock::new();
    SERVER
        .get_or_init(|| {
            let (connection, screen) = RustConnection::connect(None)
                .map_err(|err| format!("cannot reach the X server: {err}"))?;
            let root = connection.setup().roots[screen].root;
            Ok(Mutex::new((connection, root)))
        })
        .as_ref()
        .map_err(Clone::clone)
}

/// The monitors as RandR lays them out, in pixels: X has no scaling, so
/// pixels are the logical size.
pub fn monitors() -> Result<Vec<Monitor>, String> {
    let server = server()?.lock_anyway();
    let (connection, root) = &*server;
    let reply = connection
        .randr_get_monitors(*root, true)
        .map_err(|err| err.to_string())?
        .reply()
        .map_err(|err| format!("the X server did not list its monitors: {err}"))?;
    let monitors: Vec<Monitor> = reply
        .monitors
        .iter()
        .map(|m| Monitor {
            x: i32::from(m.x),
            y: i32::from(m.y),
            width: i32::from(m.width),
            height: i32::from(m.height),
        })
        .collect();
    if monitors.is_empty() {
        return Err("the X server named no monitors".into());
    }
    Ok(monitors)
}

/// Where the pointer is on the whole desktop.
pub fn cursor() -> Result<(i32, i32), String> {
    let server = server()?.lock_anyway();
    let (connection, root) = &*server;
    let reply = connection
        .query_pointer(*root)
        .map_err(|err| err.to_string())?
        .reply()
        .map_err(|err| err.to_string())?;
    Ok((i32::from(reply.root_x), i32::from(reply.root_y)))
}

/// Puts the pointer at `x, y` of the desktop.
pub fn place(x: i32, y: i32) -> Result<(), String> {
    let (x, y) = (clamp(x), clamp(y));
    let server = server()?.lock_anyway();
    let (connection, root) = &*server;
    connection
        .warp_pointer(x11rb::NONE, *root, 0, 0, 0, 0, x, y)
        .map_err(|err| err.to_string())?;
    connection.flush().map_err(|err| err.to_string())
}

/// X coordinates are 16-bit.
fn clamp(value: i32) -> i16 {
    value.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16
}

#[cfg(test)]
mod tests {
    #[test]
    fn coordinates_fit_in_16_bits() {
        assert_eq!(super::clamp(1920), 1920);
        assert_eq!(super::clamp(-40_000), i16::MIN);
        assert_eq!(super::clamp(70_000), i16::MAX);
    }
}
