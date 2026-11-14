//! The General tab: the switcher's corner and order, the clipboard
//! history, updates, and the language.

use super::{Form, Settings};
use crate::i18n;
use eframe::egui;
use mickey_config as config;
use mickey_config::SWITCHER_POSITIONS;
use mickey_config::i18n::Lang;

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
        ui.label(i18n::t("general.order_in_switcher"));
        if form.order.is_empty() {
            ui.weak(i18n::t("general.as_layout"));
        } else if ui.button(i18n::t("general.like_layout")).clicked() {
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
    ui.weak(i18n::t("general.preview"));
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
        egui::ComboBox::from_label(i18n::t("general.switcher_position"))
            .selected_text(switcher_position_label(&form.position))
            .show_ui(ui, |ui| {
                for position in SWITCHER_POSITIONS {
                    ui.selectable_value(
                        &mut form.position,
                        position.to_string(),
                        switcher_position_label(position),
                    );
                }
            });
        ui.weak(i18n::t("general.position_hint"));
        ui.add_space(8.0);
        ui.checkbox(&mut form.vertical, i18n::t("general.column"));
        ui.add_space(8.0);
        switcher_order(ui, form);
        ui.add_space(12.0);
        ui.label(i18n::t("general.history_label"));
        for (choice, text) in [
            (None, i18n::t("general.history_auto")),
            (Some(true), i18n::t("general.history_always")),
            (Some(false), i18n::t("general.history_never")),
        ] {
            ui.radio_value(&mut form.history, choice, text);
        }
        ui.weak(i18n::t("general.history_hint"));
        ui.add_space(12.0);
        ui.checkbox(&mut form.check_updates, i18n::t("general.check_updates"));
        ui.weak(i18n::t("general.check_updates_hint"));
        ui.add_space(12.0);
        // Language: Auto follows the system; a chosen one applies at once.
        ui.label(i18n::t("general.language_label"));
        let mut current = form.language.clone();
        let chosen = egui::ComboBox::from_id_salt("language")
            .selected_text(language_label(&current))
            .show_ui(ui, |ui| {
                ui.selectable_value(
                    &mut current,
                    "auto".into(),
                    i18n::t("interface.language_auto"),
                );
                for lang in Lang::ALL {
                    ui.selectable_value(&mut current, lang.code().into(), lang.name());
                }
            })
            .response
            .changed();
        if chosen && current != form.language {
            form.language = current.clone();
            // Apply at once: the next frame reads the new language.
            i18n::apply(&current);
            // Repaint the whole window so labels update now.
            ui.ctx().request_repaint();
        }
        ui.weak(i18n::t("general.language_hint"));
    }
}

/// A switcher position's translated name. Each branch calls the catalogue
/// through a literal key so the test sees every key is used.
fn switcher_position_label(code: &str) -> String {
    match code {
        "center" => i18n::t("switcher_positions.center").to_string(),
        "top-left" => i18n::t("switcher_positions.top_left").to_string(),
        "top-right" => i18n::t("switcher_positions.top_right").to_string(),
        "bottom-left" => i18n::t("switcher_positions.bottom_left").to_string(),
        "bottom-right" => i18n::t("switcher_positions.bottom_right").to_string(),
        _ => code.to_string(),
    }
}

/// A language code's display name: `auto`, or the language's own name.
fn language_label(code: &str) -> String {
    if code == "auto" {
        i18n::t("interface.language_auto").to_string()
    } else {
        Lang::from_code(code)
            .map(|lang| lang.name().to_string())
            .unwrap_or_else(|| code.to_string())
    }
}
