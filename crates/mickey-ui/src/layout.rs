//! The Layout tab (PRD 0005): this PC in the middle, each peer on its side,
//! drawn with the monitors they reported. Dragging a peer next to another
//! side of this PC moves it there; dropping it in the top strip takes it
//! off every side (hotkeys only).

use crate::settings::PcRow;
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};
use mickey_config::Side;
use mickey_config::peers::{Monitor, Peers};

/// Used when a PC has not reported its monitors (offline, or older).
const UNKNOWN: Monitor = Monitor {
    x: 0,
    y: 0,
    width: 1920,
    height: 1080,
};
/// The strip at the top for peers without a side, in points.
const STRIP: f32 = 46.0;

/// What is being dragged: the peer's row and how far it moved.
#[derive(Default)]
pub struct Drag {
    row: Option<usize>,
    moved: Vec2,
}

/// The rectangle around all of a PC's monitors, and the monitors moved so
/// that rectangle starts at 0,0.
fn outline(monitors: &[Monitor]) -> (Vec2, Vec<Rect>) {
    let monitors = if monitors.is_empty() {
        &[UNKNOWN][..]
    } else {
        monitors
    };
    let left = monitors.iter().map(|m| m.x).min().unwrap_or(0);
    let top = monitors.iter().map(|m| m.y).min().unwrap_or(0);
    let right = monitors.iter().map(|m| m.x + m.width).max().unwrap_or(0);
    let bottom = monitors.iter().map(|m| m.y + m.height).max().unwrap_or(0);
    let rects = monitors
        .iter()
        .map(|m| {
            let at = Pos2::new((m.x - left) as f32, (m.y - top) as f32);
            Rect::from_min_size(at, Vec2::new(m.width as f32, m.height as f32))
        })
        .collect();
    (
        Vec2::new((right - left) as f32, (bottom - top) as f32),
        rects,
    )
}

/// Where a peer of `size` goes next to this PC (`home`), in desktop pixels.
fn place(home: Vec2, size: Vec2, side: Side, gap: f32) -> Pos2 {
    match side {
        Side::Right => Pos2::new(home.x + gap, (home.y - size.y) / 2.0),
        Side::Left => Pos2::new(-gap - size.x, (home.y - size.y) / 2.0),
        Side::Above => Pos2::new((home.x - size.x) / 2.0, -gap - size.y),
        Side::Below => Pos2::new((home.x - size.x) / 2.0, home.y + gap),
    }
}

/// The side of `home` (on screen) that `point` is closest to, comparing
/// distances relative to its width and height.
fn side_of(home: Rect, point: Pos2) -> Side {
    let d = point - home.center();
    if (d.x / home.width()).abs() > (d.y / home.height()).abs() {
        if d.x > 0.0 { Side::Right } else { Side::Left }
    } else if d.y > 0.0 {
        Side::Below
    } else {
        Side::Above
    }
}

/// Puts `row` on `side`; a peer already there takes `row`'s old side, so
/// each side still holds one PC.
pub fn move_to(pcs: &mut [PcRow], row: usize, side: Option<Side>) {
    let old = pcs[row].side;
    if let Some(other) = pcs
        .iter()
        .position(|pc| side.is_some() && pc.side == side)
        .filter(|&other| other != row)
    {
        pcs[other].side = old;
    }
    pcs[row].side = side;
}

pub fn show(ui: &mut egui::Ui, peers: &Peers, pcs: &mut [PcRow], drag: &mut Drag) {
    let (response, painter) = ui.allocate_painter(ui.available_size(), Sense::hover());
    let canvas = response.rect;
    let strip = Rect::from_min_size(canvas.min, Vec2::new(canvas.width(), STRIP));
    let area = Rect::from_min_max(Pos2::new(canvas.min.x, strip.max.y + 8.0), canvas.max);

    // Lay everything out in desktop pixels, then scale to fit the area.
    let (home_size, home_rects) = outline(&peers.this_pc);
    let gap = home_size.x.max(home_size.y) * 0.15;
    let monitors_of = |name: &str| {
        let info = peers.peers.iter().find(|info| info.name == name);
        let online = info.is_some_and(|info| info.online);
        (outline(info.map_or(&[][..], |info| &info.monitors)), online)
    };
    let mut bounds = Rect::from_min_size(Pos2::ZERO, home_size);
    for pc in pcs.iter() {
        if let Some(side) = pc.side {
            let ((size, _), _) = monitors_of(&pc.name);
            bounds = bounds.union(Rect::from_min_size(place(home_size, size, side, gap), size));
        }
    }
    let scale = (area.width() / bounds.width()).min(area.height() / bounds.height()) * 0.9;
    let origin = area.center() - bounds.center().to_vec2() * scale;
    let to_screen = |at: Pos2| origin + at.to_vec2() * scale;

    let text = ui.visuals().text_color();
    // `beside`: the name goes right of the drawing, too small to hold it.
    let draw = |at: Pos2, rects: &[Rect], scale: f32, name: &str, fill: Color32, beside: bool| {
        for rect in rects {
            let rect = Rect::from_min_size(at + rect.min.to_vec2() * scale, rect.size() * scale);
            painter.rect_filled(rect.shrink(1.0), 3.0, fill);
            painter.rect_stroke(
                rect.shrink(1.0),
                3.0,
                Stroke::new(1.0, text),
                StrokeKind::Inside,
            );
        }
        let outline = Rect::from_min_size(at, outline_size(rects) * scale);
        let (label, align, color) = match beside {
            true => (
                outline.right_center() + Vec2::new(6.0, 0.0),
                Align2::LEFT_CENTER,
                text,
            ),
            false => (outline.center(), Align2::CENTER_CENTER, Color32::WHITE),
        };
        painter.text(label, align, name, FontId::proportional(14.0), color);
    };

    let home = Rect::from_min_size(to_screen(Pos2::ZERO), home_size * scale);
    draw(
        home.min,
        &home_rects,
        scale,
        "This PC",
        Color32::from_rgb(70, 110, 170),
        false,
    );
    painter.text(
        strip.left_center() + Vec2::new(6.0, 0.0),
        Align2::LEFT_CENTER,
        "No side (drag next to This PC):",
        FontId::proportional(12.0),
        ui.visuals().weak_text_color(),
    );
    painter.rect_stroke(
        strip,
        4.0,
        Stroke::new(1.0, ui.visuals().weak_text_color()),
        StrokeKind::Inside,
    );

    let mut strip_x = strip.min.x + 210.0;
    // A drop changes the rows, which the loop is still reading: keep it.
    let mut dropped = None;
    for (row, pc) in pcs.iter().enumerate() {
        let ((size, rects), online) = monitors_of(&pc.name);
        // Peers without a side sit in the strip, small.
        let (at, this_scale) = match pc.side {
            Some(side) => (to_screen(place(home_size, size, side, gap)), scale),
            None => {
                let small = (STRIP - 8.0) / size.y;
                let at = Pos2::new(strip_x, strip.min.y + 4.0);
                // Room for the name beside it, roughly 8 points a letter.
                strip_x += size.x * small + 8.0 * pc.name.len() as f32 + 24.0;
                (at, small)
            }
        };
        let moved = if drag.row == Some(row) {
            drag.moved
        } else {
            Vec2::ZERO
        };
        let rect = Rect::from_min_size(at + moved, size * this_scale);
        let response = ui.interact(rect, ui.scope_id().with(("peer", row)), Sense::drag());
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
            let at = rect.center();
            *drag = Drag::default();
            dropped = Some((row, (!strip.contains(at)).then(|| side_of(home, at))));
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
        draw(rect.min, &rects, this_scale, &name, fill, pc.side.is_none());
    }
    if let Some((row, side)) = dropped {
        move_to(pcs, row, side);
    }
}

fn outline_size(rects: &[Rect]) -> Vec2 {
    rects
        .iter()
        .fold(Rect::NOTHING, |all, rect| all.union(*rect))
        .size()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(name: &str, side: Option<Side>) -> PcRow {
        PcRow {
            name: name.into(),
            address: String::new(),
            hotkey: String::new(),
            side,
        }
    }

    #[test]
    fn moving_onto_a_taken_side_swaps() {
        let mut pcs = vec![row("A", Some(Side::Left)), row("B", Some(Side::Right))];
        move_to(&mut pcs, 0, Some(Side::Right));
        assert_eq!(
            (pcs[0].side, pcs[1].side),
            (Some(Side::Right), Some(Side::Left))
        );
        move_to(&mut pcs, 1, None);
        assert_eq!((pcs[0].side, pcs[1].side), (Some(Side::Right), None));
    }

    #[test]
    fn the_drop_point_picks_the_nearest_side() {
        let home = Rect::from_min_size(Pos2::ZERO, Vec2::new(200.0, 100.0));
        assert_eq!(side_of(home, Pos2::new(260.0, 50.0)), Side::Right);
        assert_eq!(side_of(home, Pos2::new(100.0, -40.0)), Side::Above);
        // Further right than below, but not relative to the size.
        assert_eq!(side_of(home, Pos2::new(150.0, 140.0)), Side::Below);
    }

    #[test]
    fn the_outline_starts_at_zero_even_with_monitors_above() {
        let windows = [
            Monitor {
                x: 0,
                y: 0,
                width: 2560,
                height: 1080,
            },
            Monitor {
                x: 246,
                y: -1080,
                width: 1920,
                height: 1080,
            },
            Monitor {
                x: 2560,
                y: -845,
                width: 1080,
                height: 1920,
            },
        ];
        let (size, rects) = outline(&windows);
        assert_eq!(size, Vec2::new(3640.0, 2160.0));
        assert_eq!(rects[1].min, Pos2::new(246.0, 0.0));
    }
}
