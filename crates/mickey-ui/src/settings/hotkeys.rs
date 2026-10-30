//! The Hotkeys tab: one row per hotkey, each typed or recorded.

use super::{Form, Settings};
use eframe::egui;
use mickey_config as config;
use std::time::Duration;

/// The Hotkeys tab's rows: what each is for, and what empty means.
pub(super) const HOTKEY_ROWS: [(&str, &str); 6] = [
    ("Next PC / switcher", config::DEFAULT_NEXT),
    ("Back to this PC", "the hotkey above + 0"),
    ("Open the switcher", config::DEFAULT_SWITCHER),
    ("Next PC (one hand)", config::DEFAULT_FORWARD),
    ("Previous PC (one hand)", config::DEFAULT_BACK),
    ("Double tap (empty: off)", ""),
];

pub(super) const DOUBLE_TAP_ROW: usize = 5;

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
        Err(err) => return format!("Could not record: {err}"),
    };
    let heard = answer.trim().strip_prefix("ok: ").map(str::to_string);
    match (heard, row == DOUBLE_TAP_ROW) {
        (Some(heard), true) => match heard.strip_prefix("double ") {
            Some(key) => {
                *hotkey_field(form, row) = key.to_string();
                format!("Recorded: double tap {key}")
            }
            None => "That was keys together; tap one key twice here.".into(),
        },
        (Some(heard), false) if heard.starts_with("double ") => {
            "That was a double tap; it belongs in the Double tap row.".into()
        }
        (Some(heard), false) => {
            *hotkey_field(form, row) = heard.clone();
            format!("Recorded: {heard}")
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
        ui.weak("Record, then press the keys together, or tap one key twice for the double tap. You can also type the names.");
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(format!("{}  Always home:", crate::theme::icon::SHIELD))
                    .strong(),
            );
            ui.weak("hold");
            crate::theme::keycaps(ui, "RightShift", false, false);
            ui.weak("alone 3 s, or with");
            crate::theme::keycaps(ui, "AltGr", false, false);
            ui.weak("2 s, and let go: works whatever is set here");
        });
        ui.checkbox(
            &mut form.share_hotkeys,
            "Share these hotkeys with the other PCs, and ask before using theirs",
        )
        .on_hover_text("Off: this PC keeps its own hotkeys, for a keyboard unlike the others'.");
        let held = game_keys(form);
        if !held.is_empty() {
            ui.colored_label(
                crate::theme::palette::AWAY,
                format!(
                    "{}  {} in a hotkey: while another PC has the keyboard, Mickey holds {} back until the next key, so games that use {} (crouch, walk, jump) may feel late there.",
                    crate::theme::icon::SHIELD,
                    held.join(", "),
                    if held.len() == 1 { "it" } else { "them" },
                    if held.len() == 1 { "it" } else { "them" },
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
                for (row, (label, default)) in HOTKEY_ROWS.iter().enumerate() {
                    ui.label(egui::RichText::new(*label).strong());
                    let double = row == DOUBLE_TAP_ROW;
                    let value = hotkey_field(form, row);
                    if self.typing == Some(row) {
                        let hint = if default.is_empty() { "none" } else { default };
                        let field = ui.add(
                            egui::TextEdit::singleline(value)
                                .hint_text(hint)
                                .desired_width(260.0),
                        );
                        field.request_focus();
                        if ui.button(format!("{}  Done", icon::CHECK)).clicked()
                            || ui.input(|input| input.key_pressed(egui::Key::Enter))
                        {
                            self.typing = None;
                        }
                    } else {
                        // The keys as keys; an empty setting shows its default, faded.
                        let shown = if value.trim().is_empty() && !double {
                            default
                        } else {
                            value.trim()
                        };
                        theme::keycaps(ui, shown, double, value.trim().is_empty());
                        if ui.button("Type").clicked() {
                            self.typing = Some(row);
                        }
                    }
                    if busy == Some(row) {
                        ui.colored_label(theme::palette::BRUISE, "Press the keys…");
                        ui.ctx().request_repaint_after(Duration::from_millis(100));
                    } else if ui
                        .add_enabled(
                            busy.is_none(),
                            egui::Button::new(format!("{}  Record", icon::KEYBOARD)),
                        )
                        .clicked()
                    {
                        let (send, answer) = std::sync::mpsc::channel();
                        std::thread::spawn(move || {
                            let _ = send.send(crate::mickey::ask(&["record"]));
                        });
                        self.recording = Some((row, answer));
                    }
                    if ui.button(format!("{}  Default", icon::UNDO)).clicked() {
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
            ui.label("Double tap: both taps within");
            ui.add(
                egui::Slider::new(&mut form.double_tap_ms, config::DOUBLE_TAP_MS)
                    .suffix(" ms")
                    .step_by(50.0),
            );
        });
        ui.weak("Only keys nobody types alone can be a double tap: Shift, Ctrl, Alt, AltGr, Super, F1–F12.");
        ui.add_space(6.0);
        ui.weak("Each PC also gets the next number: the hotkey + 1, + 2… goes straight to it.");
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
        assert!(said.contains("Double tap row"));
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
