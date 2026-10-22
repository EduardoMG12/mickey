//! The Layout tab (PRD 0005): this PC's monitors and each peer's, drawn
//! where they sit. Drag a peer anywhere; when dropped it snaps against the
//! nearest monitor of this PC, touching without overlapping, like Windows'
//! display settings. Peers without a position wait in the top strip.

use crate::settings::PcRow;
use crate::snap::{shifted, snap};
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};
use mickey_config::peers::{Monitor, Peers};

/// Used when a PC has not reported its monitors (offline, or older).
const UNKNOWN: Monitor = Monitor {
    x: 0,
    y: 0,
    width: 1920,
    height: 1080,
};
/// The strip at the top for peers without a position, in points.
const STRIP: f32 = 46.0;
/// What is being dragged: the peer's row and how far it moved.
#[derive(Default)]
pub struct Drag {
    row: Option<usize>,
    moved: Vec2,
}

fn known(monitors: &[Monitor]) -> Vec<Monitor> {
    if monitors.is_empty() {
        vec![UNKNOWN]
    } else {
        monitors.to_vec()
    }
}

fn to_rect(monitor: &Monitor) -> Rect {
    Rect::from_min_size(
        Pos2::new(monitor.x as f32, monitor.y as f32),
        Vec2::new(monitor.width as f32, monitor.height as f32),
    )
}

fn around(monitors: &[Monitor]) -> Rect {
    monitors
        .iter()
        .fold(Rect::NOTHING, |all, m| all.union(to_rect(m)))
}

pub fn show(ui: &mut egui::Ui, peers: &Peers, pcs: &mut [PcRow], drag: &mut Drag) {
    // Leaves room below for the Save row.
    let size = ui.available_size() - Vec2::new(0.0, 36.0);
    let (response, painter) = ui.allocate_painter(size.max(Vec2::splat(100.0)), Sense::hover());
    let canvas = response.rect;
    let strip = Rect::from_min_size(canvas.min, Vec2::new(canvas.width(), STRIP));
    let area = Rect::from_min_max(Pos2::new(canvas.min.x, strip.max.y + 8.0), canvas.max);

    let home = known(&peers.this_pc);
    let monitors_of = |name: &str| {
        let info = peers.peers.iter().find(|info| info.name == name);
        let online = info.is_some_and(|info| info.online);
        (known(info.map_or(&[][..], |info| &info.monitors)), online)
    };
    // Everything in desktop pixels first, then scaled to fit the area.
    let mut bounds = around(&home);
    for pc in pcs.iter() {
        if let Some(at) = pc.position {
            let (monitors, _) = monitors_of(&pc.name);
            let placed: Vec<Monitor> = monitors.iter().map(|m| shifted(m, at)).collect();
            bounds = bounds.union(around(&placed));
        }
    }
    let scale = (area.width() / bounds.width()).min(area.height() / bounds.height()) * 0.9;
    let origin = area.center() - bounds.center().to_vec2() * scale;
    let to_screen = |at: Pos2| origin + at.to_vec2() * scale;
    let to_desktop = |at: Pos2| ((at - origin) / scale).to_pos2();

    let text = ui.visuals().text_color();
    // Draws monitors already in screen points; `beside`: the name goes to
    // the right, the drawing being too small to hold it.
    let draw = |rects: &[Rect], name: &str, fill: Color32, beside: bool| {
        for rect in rects {
            painter.rect_filled(rect.shrink(1.0), 3.0, fill);
            painter.rect_stroke(
                rect.shrink(1.0),
                3.0,
                Stroke::new(1.0, text),
                StrokeKind::Inside,
            );
        }
        let all = rects
            .iter()
            .fold(Rect::NOTHING, |all, rect| all.union(*rect));
        let (label, align, color) = match beside {
            true => (
                all.right_center() + Vec2::new(6.0, 0.0),
                Align2::LEFT_CENTER,
                text,
            ),
            false => (all.center(), Align2::CENTER_CENTER, Color32::WHITE),
        };
        painter.text(label, align, name, FontId::proportional(14.0), color);
    };

    let home_rects: Vec<Rect> = home
        .iter()
        .map(|m| Rect::from_min_max(to_screen(to_rect(m).min), to_screen(to_rect(m).max)))
        .collect();
    draw(
        &home_rects,
        "This PC",
        Color32::from_rgb(70, 110, 170),
        false,
    );
    painter.text(
        strip.left_center() + Vec2::new(6.0, 0.0),
        Align2::LEFT_CENTER,
        "Not placed (drag next to This PC):",
        FontId::proportional(12.0),
        ui.visuals().weak_text_color(),
    );
    painter.rect_stroke(
        strip,
        4.0,
        Stroke::new(1.0, ui.visuals().weak_text_color()),
        StrokeKind::Inside,
    );

    let mut strip_x = strip.min.x + 230.0;
    // A drop changes the rows, which the loop is still reading: keep it.
    let mut dropped = None;
    for (row, pc) in pcs.iter().enumerate() {
        let (monitors, online) = monitors_of(&pc.name);
        let outline = around(&monitors);
        // Where the peer's outline goes on screen, and at what scale.
        let (at, this_scale) = match pc.position {
            Some([x, y]) => (
                to_screen(outline.min + Vec2::new(x as f32, y as f32)),
                scale,
            ),
            None => {
                let small = (STRIP - 8.0) / outline.height();
                let at = Pos2::new(strip_x, strip.min.y + 4.0);
                // Room for the name beside it, roughly 8 points a letter.
                strip_x += outline.width() * small + 8.0 * pc.name.len() as f32 + 24.0;
                (at, small)
            }
        };
        let moved = if drag.row == Some(row) {
            drag.moved
        } else {
            Vec2::ZERO
        };
        let at = at + moved;
        let rects: Vec<Rect> = monitors
            .iter()
            .map(|m| {
                let rect = to_rect(m);
                Rect::from_min_size(
                    at + (rect.min - outline.min) * this_scale,
                    rect.size() * this_scale,
                )
            })
            .collect();
        let whole = Rect::from_min_size(at, outline.size() * this_scale);
        let response = ui.interact(whole, ui.scope_id().with(("peer", row)), Sense::drag());
        if response.drag_started() {
            *drag = Drag {
                row: Some(row),
                moved: Vec2::ZERO,
            };
        }
        if response.dragged() {
            drag.moved += response.drag_delta();
        }
        if response.drag_stopped() && drag.row == Some(row) {
            *drag = Drag::default();
            dropped = Some(if strip.contains(whole.center()) {
                (row, None)
            } else {
                // The peer's outline centred where it was let go.
                let center = to_desktop(whole.center()) - outline.center().to_vec2();
                let wanted = [center.x.round() as i32, center.y.round() as i32];
                (row, snap(&home, &monitors, wanted))
            });
        }
        let fill = if online {
            Color32::from_rgb(70, 150, 90)
        } else {
            Color32::from_gray(90)
        };
        let name = if online {
            pc.name.clone()
        } else {
            format!("{} (offline)", pc.name)
        };
        draw(
            &rects,
            &name,
            fill,
            pc.position.is_none() && drag.row != Some(row),
        );
    }
    if let Some((row, position)) = dropped {
        pcs[row].position = position;
    }
}
