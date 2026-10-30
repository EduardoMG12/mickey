//! Sound in the settings: who listens, and each PC's volume there.

use super::Form;
use eframe::egui;
use mickey_config::SoundMode;

/// A TOML basic string: quotes and backslashes escaped.
/// A PC's volume on the listener and its mute, by its key; every PC shows
/// and changes the same, which Mickey shares (PRD 0012).
pub(super) fn volume(
    ui: &mut egui::Ui,
    key: &str,
    volumes: &mut std::collections::BTreeMap<String, u8>,
    muted: &mut Vec<String>,
) {
    ui.horizontal(|ui| {
        let mut level = volumes.get(key).copied().unwrap_or(100);
        ui.spacing_mut().slider_width = 110.0;
        if ui
            .add(egui::Slider::new(&mut level, 0..=100).suffix("%"))
            .changed()
        {
            volumes.insert(key.to_string(), level);
        }
        let mut silent = muted.iter().any(|muted| muted == key);
        if ui.checkbox(&mut silent, "mute").changed() {
            muted.retain(|muted| muted != key);
            if silent {
                muted.push(key.to_string());
            }
        }
    });
}

/// "Who listens" and "This PC's sound" (PRD 0012), in the This PC tab: the
/// listener is picked from every PC with a key, the same choice on every
/// PC (Mickey shares it); the mode is this PC's own.
pub(super) fn sound(ui: &mut egui::Ui, form: &mut Form, own_key: Option<&str>) {
    let mut choices: Vec<(Option<String>, String)> =
        vec![(None, "Nobody: each PC uses its own speakers".into())];
    if let Some(key) = own_key {
        choices.push((Some(key.to_string()), format!("This PC ({})", form.name)));
    }
    for pc in &form.pcs {
        if let Some(key) = &pc.key {
            choices.push((Some(key.clone()), pc.name.clone()));
        }
    }
    let name_of = |key: &Option<String>| {
        choices
            .iter()
            .find(|(choice, _)| choice == key)
            .map_or("a PC not in this list".to_string(), |(_, name)| {
                name.clone()
            })
    };
    let shown = name_of(&form.listener);
    ui.strong("Shared sound");
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.label("Where are your headphones?");
        egui::ComboBox::from_id_salt("listener")
            .selected_text(&shown)
            .width(260.0)
            .show_ui(ui, |ui| {
                for (key, name) in choices.clone() {
                    ui.selectable_value(&mut form.listener, key, name);
                }
            });
    });
    // What happens with these choices, in one sentence.
    let here = form.listener.is_some() && form.listener.as_deref() == own_key;
    let now = match (&form.listener, here, form.mode) {
        (None, _, _) => "Each PC plays its own sound on its own speakers.".to_string(),
        (Some(_), true, _) => "Every PC's sound plays here, on this PC's headphones.".to_string(),
        (Some(_), false, SoundMode::Here) => "This PC's sound stays here.".to_string(),
        (Some(_), false, SoundMode::Both) => {
            format!("This PC's sound plays on {shown}, and here too.")
        }
        (Some(_), false, SoundMode::Listener) => {
            format!("This PC's sound plays on {shown}; this PC stays quiet.")
        }
    };
    ui.label(egui::RichText::new(format!("●  Now: {now}")).color(crate::theme::palette::HERE));
    ui.weak("Picked on any PC, it changes on every PC.");
    ui.add_space(6.0);
    ui.label("When another PC has the headphones, this PC's sound:");
    for (mode, text) in [
        (SoundMode::Listener, "goes there only (this PC goes quiet)"),
        (SoundMode::Both, "goes there and plays here too"),
        (SoundMode::Here, "stays here"),
    ] {
        ui.radio_value(&mut form.mode, mode, text);
    }
    if let Some(key) = own_key {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label("Volume of this PC's sound there");
            volume(ui, key, &mut form.volumes, &mut form.muted);
        });
    }
}
