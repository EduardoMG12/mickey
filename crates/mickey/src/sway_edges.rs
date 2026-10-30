//! Screen edges on Sway (#178), where no one says where the cursor is: a
//! one-pixel layer surface (wlr-layer-shell) on each stretch of edge that
//! leads to a placed peer. The cursor entering one is the push onto that
//! edge; Mickey then switches the focus as it always does, reading input
//! through evdev.
//!
//! A strip arms only once the cursor has left it: coming home, Mickey puts
//! the cursor back on the edge's last pixel, under the strip, and that
//! must not send it straight back. Sway answers that placing with a leave
//! and an enter, so an enter just after Mickey placed the cursor does not
//! count either (`PLACED_LATELY`).

use crate::edges::Line;
use crate::monitors::Monitor;
use crate::threads::LockAnyway;
use std::os::fd::AsFd;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use wayland_client::protocol::{
    wl_buffer::WlBuffer, wl_callback::WlCallback, wl_compositor::WlCompositor, wl_output,
    wl_pointer, wl_registry, wl_seat, wl_shm, wl_shm_pool::WlShmPool, wl_surface::WlSurface,
};
use wayland_client::{Connection, Dispatch, QueueHandle, WEnum};
use wayland_protocols_wlr::layer_shell::v1::client::{
    zwlr_layer_shell_v1::{Layer, ZwlrLayerShellV1},
    zwlr_layer_surface_v1::{self, Anchor, ZwlrLayerSurfaceV1},
};

/// An enter this soon after Mickey placed the cursor came from the placing.
const PLACED_LATELY: Duration = Duration::from_millis(300);

/// Starts the strips on a thread of their own. `wake` makes that thread
/// look at `wanted` again; the hits go to `hit`.
pub fn start(
    wanted: Arc<Mutex<Vec<Line>>>,
    hit: Sender<(i32, i32)>,
) -> Result<Box<dyn Fn() + Send>, String> {
    let connection = Connection::connect_to_env().map_err(|err| err.to_string())?;
    let mut queue = connection.new_event_queue();
    let handle = queue.handle();
    connection.display().get_registry(&handle, ());
    let mut strips = Strips {
        compositor: None,
        shm: None,
        layer_shell: None,
        outputs: Vec::new(),
        edges: Vec::new(),
        applied: Vec::new(),
        armed: true,
        wanted,
        hit,
    };
    queue
        .roundtrip(&mut strips)
        .map_err(|err| err.to_string())?;
    // A second round trip: the outputs' names, and the pointer.
    queue
        .roundtrip(&mut strips)
        .map_err(|err| err.to_string())?;
    if strips.compositor.is_none() || strips.shm.is_none() || strips.layer_shell.is_none() {
        return Err("the compositor has no wlr-layer-shell".into());
    }
    strips.apply(&handle);
    let display = connection.display();
    let waker = handle.clone();
    crate::threads::spawn("sway edges", move || {
        while queue.blocking_dispatch(&mut strips).is_ok() {}
    });
    // A sync's answer wakes the thread, which then applies `wanted`.
    Ok(Box::new(move || {
        display.sync(&waker, ());
        let _ = connection.flush();
    }))
}

struct Strips {
    compositor: Option<WlCompositor>,
    shm: Option<wl_shm::WlShm>,
    layer_shell: Option<ZwlrLayerShellV1>,
    /// Each output and the name it gave.
    outputs: Vec<(wl_output::WlOutput, Option<String>)>,
    edges: Vec<Edge>,
    /// The lines the strips stand on now.
    applied: Vec<Line>,
    /// Whether entering a strip counts (see the module's comment).
    armed: bool,
    wanted: Arc<Mutex<Vec<Line>>>,
    hit: Sender<(i32, i32)>,
}

struct Edge {
    surface: WlSurface,
    layer: ZwlrLayerSurfaceV1,
    buffer: Option<WlBuffer>,
    /// The pixel just off the desktop at the strip's start, and the axis
    /// the strip runs along.
    off: (i32, i32),
    along_y: bool,
    size: (u32, u32),
}

/// Where the strip for one line goes (`place_strip`).
#[derive(Debug, PartialEq)]
struct Placement {
    /// Which of this PC's monitors it is on.
    monitor: usize,
    /// The monitor's edge and the end the strip starts from.
    anchor: Anchor,
    /// How far from that end, along the edge.
    margin: i32,
    /// Its width and height: one pixel across, the line's length along.
    size: (u32, u32),
    /// The pixel just off the desktop at the strip's start.
    off: (i32, i32),
    /// Whether it runs along y (a left or right edge).
    along_y: bool,
}

/// Which side of which monitor `line` (from `edges::barriers`) is on.
/// Right and bottom lines lie just off the monitor; left and top ones on
/// its first pixel, so their pixel off the desktop is one further.
fn place_strip(here: &[Monitor], (x1, y1, x2, y2): Line) -> Option<Placement> {
    let within = |a: i32, start: i32, len: i32| a >= start && a < start + len;
    here.iter().enumerate().find_map(|(monitor, m)| {
        let tall = (1, (y2 - y1 + 1) as u32);
        let wide = ((x2 - x1 + 1) as u32, 1);
        let strip = |anchor, margin, size, off, along_y| {
            Some(Placement {
                monitor,
                anchor,
                margin,
                size,
                off,
                along_y,
            })
        };
        if x1 == x2 && within(y1, m.y, m.height) {
            if x1 == m.x + m.width {
                return strip(Anchor::Right | Anchor::Top, y1 - m.y, tall, (x1, y1), true);
            }
            if x1 == m.x {
                return strip(
                    Anchor::Left | Anchor::Top,
                    y1 - m.y,
                    tall,
                    (x1 - 1, y1),
                    true,
                );
            }
        }
        if y1 == y2 && within(x1, m.x, m.width) {
            if y1 == m.y + m.height {
                return strip(
                    Anchor::Bottom | Anchor::Left,
                    x1 - m.x,
                    wide,
                    (x1, y1),
                    false,
                );
            }
            if y1 == m.y {
                return strip(
                    Anchor::Top | Anchor::Left,
                    x1 - m.x,
                    wide,
                    (x1, y1 - 1),
                    false,
                );
            }
        }
        None
    })
}

impl Strips {
    /// Puts strips on the wanted lines, if they changed.
    fn apply(&mut self, handle: &QueueHandle<Strips>) {
        let wanted = self.wanted.lock_anyway().clone();
        if wanted == self.applied {
            return;
        }
        for edge in self.edges.drain(..) {
            edge.layer.destroy();
            edge.surface.destroy();
            if let Some(buffer) = edge.buffer {
                buffer.destroy();
            }
        }
        self.applied = wanted.clone();
        let (Some(compositor), Some(layer_shell)) = (&self.compositor, &self.layer_shell) else {
            return;
        };
        let named = crate::sway::outputs().unwrap_or_default();
        let here: Vec<Monitor> = named.iter().map(|(_, monitor)| *monitor).collect();
        for line in wanted {
            let Some(Placement {
                monitor,
                anchor,
                margin,
                size,
                off,
                along_y,
            }) = place_strip(&here, line)
            else {
                continue;
            };
            let output = self
                .outputs
                .iter()
                .find(|(_, name)| name.as_deref() == Some(named[monitor].0.as_str()))
                .map(|(output, _)| output);
            let surface = compositor.create_surface(handle, ());
            let index = self.edges.len();
            let layer = layer_shell.get_layer_surface(
                &surface,
                output,
                Layer::Overlay,
                "mickey-edge".into(),
                handle,
                index,
            );
            layer.set_anchor(anchor);
            layer.set_size(size.0, size.1);
            // Outside the edge the strip runs along, from that end.
            if along_y {
                layer.set_margin(margin, 0, 0, 0);
            } else {
                layer.set_margin(0, 0, 0, margin);
            }
            // Over windows, without pushing them aside.
            layer.set_exclusive_zone(-1);
            surface.commit();
            self.edges.push(Edge {
                surface,
                layer,
                buffer: None,
                off,
                along_y,
                size,
            });
        }
    }

    /// A transparent buffer of the strip's size, from a file in memory.
    fn buffer(
        &self,
        (width, height): (u32, u32),
        handle: &QueueHandle<Strips>,
    ) -> Option<WlBuffer> {
        let shm = self.shm.as_ref()?;
        let runtime = std::env::var_os("XDG_RUNTIME_DIR")?;
        let path =
            std::path::PathBuf::from(runtime).join(format!("mickey-edge-{}", std::process::id()));
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)
            .ok()?;
        // Only the open file is needed: no one else should find it.
        let _ = std::fs::remove_file(&path);
        let stride = width * 4;
        let bytes = stride * height;
        file.set_len(u64::from(bytes)).ok()?;
        let pool: WlShmPool = shm.create_pool(file.as_fd(), bytes as i32, handle, ());
        let buffer = pool.create_buffer(
            0,
            width as i32,
            height as i32,
            stride as i32,
            wl_shm::Format::Argb8888,
            handle,
            (),
        );
        pool.destroy();
        Some(buffer)
    }
}

impl Dispatch<wl_registry::WlRegistry, ()> for Strips {
    fn event(
        strips: &mut Strips,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        handle: &QueueHandle<Strips>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            match interface.as_str() {
                "wl_compositor" => {
                    strips.compositor = Some(registry.bind(name, version.min(4), handle, ()))
                }
                "wl_shm" => strips.shm = Some(registry.bind(name, 1, handle, ())),
                "zwlr_layer_shell_v1" => {
                    strips.layer_shell = Some(registry.bind(name, version.min(4), handle, ()));
                }
                // Version 4 names each output, as Sway's IPC does.
                "wl_output" if version >= 4 => {
                    let output = registry.bind(name, 4, handle, ());
                    strips.outputs.push((output, None));
                }
                "wl_seat" => {
                    let _: wl_seat::WlSeat = registry.bind(name, version.min(5), handle, ());
                }
                _ => {}
            }
        }
    }
}

impl Dispatch<wl_output::WlOutput, ()> for Strips {
    fn event(
        strips: &mut Strips,
        output: &wl_output::WlOutput,
        event: wl_output::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Strips>,
    ) {
        if let wl_output::Event::Name { name } = event
            && let Some(entry) = strips.outputs.iter_mut().find(|(o, _)| o == output)
        {
            entry.1 = Some(name);
        }
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for Strips {
    fn event(
        _: &mut Strips,
        seat: &wl_seat::WlSeat,
        event: wl_seat::Event,
        _: &(),
        _: &Connection,
        handle: &QueueHandle<Strips>,
    ) {
        if let wl_seat::Event::Capabilities {
            capabilities: WEnum::Value(capabilities),
        } = event
            && capabilities.contains(wl_seat::Capability::Pointer)
        {
            seat.get_pointer(handle, ());
        }
    }
}

impl Dispatch<ZwlrLayerSurfaceV1, usize> for Strips {
    fn event(
        strips: &mut Strips,
        layer: &ZwlrLayerSurfaceV1,
        event: zwlr_layer_surface_v1::Event,
        index: &usize,
        _: &Connection,
        handle: &QueueHandle<Strips>,
    ) {
        if let zwlr_layer_surface_v1::Event::Configure { serial, .. } = event {
            layer.ack_configure(serial);
            let Some(size) = strips.edges.get(*index).map(|edge| edge.size) else {
                return;
            };
            let buffer = strips.buffer(size, handle);
            if let Some(edge) = strips.edges.get_mut(*index) {
                edge.surface.attach(buffer.as_ref(), 0, 0);
                edge.surface
                    .damage_buffer(0, 0, size.0 as i32, size.1 as i32);
                edge.surface.commit();
                edge.buffer = buffer;
            }
        }
    }
}

impl Dispatch<wl_pointer::WlPointer, ()> for Strips {
    fn event(
        strips: &mut Strips,
        _: &wl_pointer::WlPointer,
        event: wl_pointer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Strips>,
    ) {
        match event {
            wl_pointer::Event::Enter {
                surface,
                surface_x,
                surface_y,
                ..
            } => {
                let Some(edge) = strips.edges.iter().find(|edge| edge.surface == surface) else {
                    return;
                };
                if !strips.armed || crate::sway::placed_within(PLACED_LATELY) {
                    strips.armed = false;
                    return;
                }
                strips.armed = false;
                let (x, y) = edge.off;
                let point = if edge.along_y {
                    (x, y + surface_y as i32)
                } else {
                    (x + surface_x as i32, y)
                };
                let _ = strips.hit.send(point);
            }
            wl_pointer::Event::Leave { .. } => strips.armed = true,
            _ => {}
        }
    }
}

impl Dispatch<WlCallback, ()> for Strips {
    fn event(
        strips: &mut Strips,
        _: &WlCallback,
        _: <WlCallback as wayland_client::Proxy>::Event,
        _: &(),
        _: &Connection,
        handle: &QueueHandle<Strips>,
    ) {
        strips.apply(handle);
    }
}

// Nothing to do with these objects' own events.
wayland_client::delegate_noop!(Strips: ignore WlCompositor);
wayland_client::delegate_noop!(Strips: ignore wl_shm::WlShm);
wayland_client::delegate_noop!(Strips: ignore WlShmPool);
wayland_client::delegate_noop!(Strips: ignore WlBuffer);
wayland_client::delegate_noop!(Strips: ignore WlSurface);
wayland_client::delegate_noop!(Strips: ignore ZwlrLayerShellV1);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_strip_sits_on_the_side_its_line_names() {
        let here = [Monitor {
            x: 0,
            y: 0,
            width: 1280,
            height: 720,
        }];
        // A peer to the right, along the whole height.
        assert_eq!(
            place_strip(&here, (1280, 0, 1280, 719)),
            Some(Placement {
                monitor: 0,
                anchor: Anchor::Right | Anchor::Top,
                margin: 0,
                size: (1, 720),
                off: (1280, 0),
                along_y: true,
            })
        );
        // To the left, along the lower half: off the desktop is x = -1.
        let left = place_strip(&here, (0, 360, 0, 719)).unwrap();
        assert_eq!(
            (left.anchor, left.margin, left.size, left.off),
            (Anchor::Left | Anchor::Top, 360, (1, 360), (-1, 360))
        );
        // Above: off the desktop is y = -1.
        let above = place_strip(&here, (100, 0, 299, 0)).unwrap();
        assert_eq!(
            (above.size, above.off, above.along_y),
            ((200, 1), (100, -1), false)
        );
        // A line on no monitor's side.
        assert!(place_strip(&here, (5000, 0, 5000, 10)).is_none());
    }
}
