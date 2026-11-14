//! Sound in the settings: who listens, and each PC's volume there.

use super::Form;
use crate::i18n;
use eframe::egui;
use mickey_config::SoundMode;

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
        if ui.checkbox(&mut silent, i18n::t("sound.mute")).changed() {
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
    let mut choices: Vec<(Option<String>, String)> = vec![(None, i18n::t("sound.nobody").into())];
    if let Some(key) = own_key {
        choices.push((
            Some(key.to_string()),
            i18n::tf("sound.this_pc_named", &[("name", &form.name)]),
        ));
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
            .map_or(i18n::t("sound.a_pc_not_listed").to_string(), |(_, name)| {
                name.clone()
            })
    };
    let shown = name_of(&form.listener);
    ui.strong(i18n::t("sound.title"));
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.label(i18n::t("sound.where"));
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
        (None, _, _) => i18n::t("sound.now_each_own").to_string(),
        (Some(_), true, _) => i18n::t("sound.now_here").to_string(),
        (Some(_), false, SoundMode::Here) => i18n::t("sound.now_stays_here").to_string(),
        (Some(_), false, SoundMode::Both) => i18n::tf("sound.now_both", &[("where", &shown)]),
        (Some(_), false, SoundMode::Listener) => {
            i18n::tf("sound.now_listener_only", &[("where", &shown)])
        }
    };
    ui.label(
        egui::RichText::new(format!("●  {}: {now}", i18n::t("sound.now_label")))
            .color(crate::theme::palette::HERE),
    );
    ui.weak(i18n::t("sound.picked_anywhere"));
    ui.add_space(6.0);
    ui.label(i18n::t("sound.when_other"));
    for (mode, text) in [
        (SoundMode::Listener, i18n::t("sound.mode_listener")),
        (SoundMode::Both, i18n::t("sound.mode_both")),
        (SoundMode::Here, i18n::t("sound.mode_here")),
    ] {
        ui.radio_value(&mut form.mode, mode, text);
    }
    if let Some(key) = own_key {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(i18n::t("sound.volume_there"));
            volume(ui, key, &mut form.volumes, &mut form.muted);
        });
    }
}
