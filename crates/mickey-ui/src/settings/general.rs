//! The General tab: the switcher's corner and order, the clipboard
//! history, updates.

use super::{Form, Settings};
use eframe::egui;
use mickey_config as config;
use mickey_config::SWITCHER_POSITIONS;

/// The switcher's order: every PC with buttons to move it. Moving one saves
/// the whole list as the custom order; "Like the layout" empties it.
pub(super) fn switcher_order(ui: &mut egui::Ui, form: &mut Form) {
    let this_pc = if form.name.trim().is_empty() {
        config::ThisPc::default().name()
    } else {
        form.name.trim().to_string()
    };
    let mut cards = vec![(this_pc.as_str(), Some([0, 0]))];
    cards.extend(form.pcs.iter().map(|pc| (pc.name.as_str(), pc.position)));
    let interface = config::Interface {
        switcher_vertical: form.vertical,
        switcher_order: form.order.clone(),
        ..config::Interface::default()
    };
    let shown: Vec<String> = interface
        .switcher_sort(&cards)
        .into_iter()
        .map(|index| cards[index].0.to_string())
        .collect();
    ui.horizontal(|ui| {
        ui.label("Order in the switcher");
        if form.order.is_empty() {
            ui.weak("(as the layout places them)");
        } else if ui.button("Like the layout").clicked() {
            form.order.clear();
        }
    });
    let mut moved = None;
    for (i, name) in shown.iter().enumerate() {
        ui.horizontal(|ui| {
            let up = ui.add_enabled(
                i > 0,
                egui::Button::new(crate::theme::icon::ARROW_UP).small(),
            );
            let down = ui.add_enabled(
                i + 1 < shown.len(),
                egui::Button::new(crate::theme::icon::ARROW_DOWN).small(),
            );
            if up.clicked() {
                moved = Some((i, i - 1));
            }
            if down.clicked() {
                moved = Some((i, i + 1));
            }
            ui.label(format!("{i}  {name}"));
        });
    }
    ui.add_space(8.0);
    ui.weak("Preview: how the switcher will look.");
    preview(ui, &shown, form.vertical);
    if let Some((from, to)) = moved {
        let mut order = shown;
        order.swap(from, to);
        form.order = order;
    }
}

/// The switcher as it will show, drawn by the same code as the real one
/// (`crate::cards`), with the first PC in use and the next one selected.
pub(super) fn preview(ui: &mut egui::Ui, names: &[String], vertical: bool) {
    let choice = crate::choice::Choice {
        corner: crate::place::Corner::BottomRight,
        vertical,
        pcs: names
            .iter()
            .enumerate()
            .map(|(id, name)| crate::choice::Pc {
                id,
                name: name.clone(),
                online: true,
            })
            .collect(),
        focused: 0,
        selected: usize::from(names.len() > 1),
    };
    // Measuring lays the cards out off screen, so it is done again only
    // when the names or the direction change.
    static WIDTH: std::sync::Mutex<Option<(Vec<String>, bool, f32)>> = std::sync::Mutex::new(None);
    let mut cached = WIDTH.lock().unwrap_or_else(|p| p.into_inner());
    let width = match cached.as_ref() {
        Some((known, was_vertical, width)) if known == names && *was_vertical == vertical => *width,
        _ => {
            let width = crate::measure(&choice).1;
            *cached = Some((names.to_vec(), vertical, width));
            width
        }
    };
    egui::Frame::NONE
        .fill(crate::theme::palette::INK)
        .corner_radius(10.0)
        .inner_margin(8.0)
        .show(ui, |ui| {
            crate::cards(ui, &choice, width);
        });
}

impl Settings {
    /// General: the switcher, the tray and the look.
    pub(super) fn general_tab(&mut self, ui: &mut egui::Ui) {
        let form = &mut self.form;
        egui::ComboBox::from_label("Switcher position")
            .selected_text(form.position.clone())
            .show_ui(ui, |ui| {
                for position in SWITCHER_POSITIONS {
                    ui.selectable_value(&mut form.position, position.to_string(), position);
                }
            });
        ui.weak("Where the switcher window appears when the hotkey opens it.");
        ui.add_space(8.0);
        ui.checkbox(&mut form.vertical, "Show the PCs in a column");
        ui.add_space(8.0);
        switcher_order(ui, form);
        ui.add_space(12.0);
        ui.label("History of copies on this PC");
        for (choice, text) in [
            (None, "Automatic: only when no other clipboard manager runs"),
            (Some(true), "Always keep one"),
            (Some(false), "Never: another manager keeps it"),
        ] {
            ui.radio_value(&mut form.history, choice, text);
        }
        ui.weak("Other managers: cliphist, CopyQ, Klipper, GPaste, Windows' Win+V. The clipboard is shared either way. Applies after a restart.");
        ui.add_space(12.0);
        ui.checkbox(
            &mut form.check_updates,
            "Check for a newer Mickey once a day",
        );
        ui.weak("Most releases fix security or speed. The check asks GitHub which release is the latest and sends nothing about you. Off, Mickey never contacts the internet.");
    }
}
