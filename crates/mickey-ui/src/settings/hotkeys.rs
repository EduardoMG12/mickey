//! The Hotkeys tab: one row per hotkey, each typed or recorded.

use super::{Form, Settings};
use crate::i18n;
use eframe::egui;
use mickey_config as config;
use std::time::Duration;

/// The Hotkeys tab's rows: what each is for. Each row's catalogue key is
/// listed once here, but the label is fetched through `hotkey_label` with
/// a literal `t()` call so the catalogue test can see it is used.
pub(super) const HOTKEY_ROW_COUNT: usize = 6;

pub(super) const DOUBLE_TAP_ROW: usize = 5;

/// The label for a row, fetched through a literal catalogue key so the
/// test sees each key is used.
pub(super) fn hotkey_label(row: usize) -> &'static str {
    match row {
        0 => i18n::t("hotkeys.next_pc"),
        1 => i18n::t("hotkeys.back_home"),
        2 => i18n::t("hotkeys.open_switcher"),
        3 => i18n::t("hotkeys.next_one_hand"),
        4 => i18n::t("hotkeys.prev_one_hand"),
        _ => i18n::t("hotkeys.double_tap"),
    }
}

pub(super) fn hotkey_field(form: &mut Form, row: usize) -> &mut String {
    match row {
        0 => &mut form.next,
        1 => &mut form.home,
        2 => &mut form.switcher,
        3 => &mut form.forward,
        4 => &mut form.back,
        _ => &mut form.double_tap,
    }
}

/// Puts what `mickey record` heard into the row, and says how it went.
pub(super) fn recorded(form: &mut Form, row: usize, result: Result<String, String>) -> String {
    let answer = match result {
        Ok(answer) => answer,
        Err(err) => return i18n::tf("hotkey_messages.could_not_record", &[("err", &err)]),
    };
    let heard = answer.trim().strip_prefix("ok: ").map(str::to_string);
    match (heard, row == DOUBLE_TAP_ROW) {
        (Some(heard), true) => match heard.strip_prefix("double ") {
            Some(key) => {
                *hotkey_field(form, row) = key.to_string();
                i18n::tf("hotkey_messages.recorded_double", &[("key", key)])
            }
            None => i18n::t("hotkey_messages.not_keys_double").into(),
        },
        (Some(heard), false) if heard.starts_with("double ") => {
            i18n::t("hotkey_messages.double_in_chord").into()
        }
        (Some(heard), false) => {
            *hotkey_field(form, row) = heard.clone();
            i18n::tf("hotkey_messages.recorded", &[("value", &heard)])
        }
        (None, _) => answer.trim().trim_start_matches("error: ").to_string(),
    }
}

/// Keys games use all the time that appear in a hotkey (#188). A key that
/// may start a hotkey is held back on the way to another PC until the
/// next key shows whether the hotkey is meant, which games feel.
pub(super) fn game_keys(form: &Form) -> Vec<&'static str> {
    const GAME_KEYS: [&str; 5] = ["LeftCtrl", "LeftShift", "LeftAlt", "Space", "Tab"];
    let mut texts = vec![
        &form.next,
        &form.home,
        &form.switcher,
        &form.forward,
        &form.back,
        &form.double_tap,
    ];
    texts.extend(form.pcs.iter().map(|pc| &pc.hotkey));
    GAME_KEYS
        .into_iter()
        .filter(|key| {
            texts.iter().any(|text| {
                text.split('+')
                    .any(|part| part.trim().eq_ignore_ascii_case(key))
            })
        })
        .collect()
}

impl Settings {
    /// Hotkeys: the keys that move the keyboard between PCs.
    pub(super) fn hotkeys_tab(&mut self, ui: &mut egui::Ui) {
        use crate::theme::{self, icon};
        let form = &mut self.form;
        ui.weak(i18n::t("hotkeys.record_hint"));
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(format!(
                    "{}  {}:",
                    crate::theme::icon::SHIELD,
                    i18n::t("hotkeys.always_home")
                ))
                .strong(),
            );
            ui.weak(i18n::t("hotkeys.hold"));
            crate::theme::keycaps(ui, "RightShift", false, false);
            ui.weak(i18n::t("hotkeys.alone_3s"));
            crate::theme::keycaps(ui, "AltGr", false, false);
            ui.weak(i18n::t("hotkeys.and_2s"));
        });
        ui.checkbox(&mut form.share_hotkeys, i18n::t("hotkeys.share_hotkeys"))
            .on_hover_text(i18n::t("hotkeys.share_hover"));
        let held = game_keys(form);
        if !held.is_empty() {
            let keys = held.join(", ");
            let them = if held.len() == 1 { "it" } else { "them" };
            ui.colored_label(
                crate::theme::palette::AWAY,
                i18n::tf(
                    "hotkeys.game_keys_warning",
                    &[
                        ("icon", crate::theme::icon::SHIELD),
                        ("keys", &keys),
                        ("them", them),
                    ],
                ),
            );
        }
        ui.add_space(4.0);
        // What the recording in progress gave, if it is done.
        if let Some((row, answer)) = &self.recording
            && let Ok(result) = answer.try_recv()
        {
            let row = *row;
            self.recording = None;
            self.message = recorded(form, row, result);
        }
        let busy = self.recording.as_ref().map(|(row, _)| *row);
        egui::Grid::new("hotkeys")
            .num_columns(5)
            .min_col_width(60.0)
            .spacing([10.0, 12.0])
            .show(ui, |ui| {
                for row in 0..HOTKEY_ROW_COUNT {
                    let label = hotkey_label(row);
                    ui.label(egui::RichText::new(label).strong());
                    let double = row == DOUBLE_TAP_ROW;
                    let value = hotkey_field(form, row);
                    if self.typing == Some(row) {
                        let default = hotkey_default(row);
                        let hint = if default.is_empty() {
                            i18n::t("hotkeys.double_tap")
                        } else {
                            default
                        };
                        let field = ui.add(
                            egui::TextEdit::singleline(value)
                                .hint_text(hint)
                                .desired_width(260.0),
                        );
                        field.request_focus();
                        if ui
                            .button(format!("{}  {}", icon::CHECK, i18n::t("hotkeys.done")))
                            .clicked()
                            || ui.input(|input| input.key_pressed(egui::Key::Enter))
                        {
                            self.typing = None;
                        }
                    } else {
                        // The keys as keys; an empty setting shows its default, faded.
                        let shown = if value.trim().is_empty() && !double {
                            hotkey_default(row)
                        } else {
                            value.trim()
                        };
                        theme::keycaps(ui, shown, double, value.trim().is_empty());
                        if ui.button(i18n::t("hotkeys.type")).clicked() {
                            self.typing = Some(row);
                        }
                    }
                    if busy == Some(row) {
                        ui.colored_label(theme::palette::BRUISE, i18n::t("hotkeys.press_keys"));
                        ui.ctx().request_repaint_after(Duration::from_millis(100));
                    } else if ui
                        .add_enabled(
                            busy.is_none(),
                            egui::Button::new(format!(
                                "{}  {}",
                                icon::KEYBOARD,
                                i18n::t("hotkeys.record")
                            )),
                        )
                        .clicked()
                    {
                        let (send, answer) = std::sync::mpsc::channel();
                        std::thread::spawn(move || {
                            let _ = send.send(crate::mickey::ask(&["record"]));
                        });
                        self.recording = Some((row, answer));
                    }
                    if ui
                        .button(format!("{}  {}", icon::UNDO, i18n::t("hotkeys.default")))
                        .clicked()
                    {
                        // Empty means the default, except the double tap,
                        // where empty turns it off.
                        *hotkey_field(form, row) = if row == DOUBLE_TAP_ROW {
                            config::DEFAULT_DOUBLE_TAP.to_string()
                        } else {
                            String::new()
                        };
                    }
                    ui.end_row();
                }
            });
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().slider_width = 220.0;
            ui.label(i18n::t("hotkeys.double_tap_within"));
            ui.add(
                egui::Slider::new(&mut form.double_tap_ms, config::DOUBLE_TAP_MS)
                    .suffix(i18n::t("hotkeys.ms"))
                    .step_by(50.0),
            );
        });
        ui.weak(i18n::t("hotkeys.double_tap_hint"));
        ui.add_space(6.0);
        ui.weak(i18n::t("hotkeys.number_hint"));
    }
}

/// The default for a hotkey row, by its row number. Empty for the
/// double tap row (it has its own default in `config`).
fn hotkey_default(row: usize) -> &'static str {
    match row {
        0 => config::DEFAULT_NEXT,
        1 => "",
        2 => config::DEFAULT_SWITCHER,
        3 => config::DEFAULT_FORWARD,
        4 => config::DEFAULT_BACK,
        _ => config::DEFAULT_DOUBLE_TAP,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_games_use_are_found_in_hotkeys() {
        let mut form = Form::from_config(&config::Config::empty());
        assert!(
            game_keys(&form).is_empty(),
            "the defaults hold back nothing games use"
        );
        form.double_tap = "LeftCtrl".into();
        form.home = "LeftShift+AltGr+H".into();
        assert_eq!(game_keys(&form), ["LeftCtrl", "LeftShift"]);
    }

    #[test]
    fn a_recording_lands_in_the_right_row_or_says_why_not() {
        let mut form = Form::from_config(&config::Config::empty());
        recorded(&mut form, 3, Ok("ok: AltGr+Right\n".into()));
        assert_eq!(form.forward, "AltGr+Right");
        recorded(&mut form, DOUBLE_TAP_ROW, Ok("ok: double LeftShift".into()));
        assert_eq!(form.double_tap, "LeftShift");
        let before = form.next.clone();
        let said = recorded(&mut form, 0, Ok("ok: double LeftShift".into()));
        assert!(said.contains("double tap"));
        assert_eq!(
            form.next, before,
            "a double tap does not go into a chord row"
        );
        assert_eq!(
            recorded(&mut form, 0, Ok("error: no keys were pressed".into())),
            "no keys were pressed"
        );
    }
}
