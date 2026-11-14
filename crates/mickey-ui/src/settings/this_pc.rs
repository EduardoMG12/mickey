//! The This PC tab: its name, whether it may be controlled, its sound.

use super::Settings;
use super::sound::sound;
use crate::i18n;
use eframe::egui;

impl Settings {
    /// This PC: its name, port, clipboard and sound.
    pub(super) fn this_pc_tab(&mut self, ui: &mut egui::Ui) {
        use crate::theme::icon;
        let form = &mut self.form;
        egui::Grid::new("this-pc")
            .num_columns(2)
            .spacing([16.0, 12.0])
            .show(ui, |ui| {
                ui.label(format!("{}  {}", icon::LAPTOP, i18n::t("this_pc.name")));
                ui.add(
                    egui::TextEdit::singleline(&mut form.name)
                        .hint_text(i18n::t("this_pc.name_hint"))
                        .desired_width(260.0),
                );
                ui.end_row();
                ui.label(format!(
                    "{}  {}",
                    icon::SHIELD,
                    i18n::t("this_pc.fingerprint")
                ));
                ui.label(egui::RichText::new(&self.fingerprint).monospace().weak());
                ui.end_row();
            });
        ui.add_space(10.0);
        ui.checkbox(
            &mut form.controls_others,
            i18n::t("this_pc.controls_others"),
        );
        ui.checkbox(&mut form.controlled, i18n::t("this_pc.controlled"));
        let mut wanted = self.autostart;
        let busy = self.autostart_change.is_some();
        let changed = ui
            .add_enabled(
                !busy,
                egui::Checkbox::new(&mut wanted, i18n::t("this_pc.autostart")),
            )
            .changed();
        if changed {
            // Applied at once, not on Save; on a thread, since
            // Windows may show its administrator prompt.
            let (send, answers) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let word = if wanted { "on" } else { "off" };
                let _ = send.send(crate::mickey::ask(&["autostart", word]));
            });
            self.autostart_change = Some(answers);
        }
        if let Some(answer) = self
            .autostart_change
            .as_ref()
            .and_then(|answers| answers.try_recv().ok())
        {
            self.autostart_change = None;
            self.autostart = crate::mickey::ask(&["autostart", "status"])
                .is_ok_and(|answer| answer.trim() == "on");
            if let Err(err) = answer {
                self.message = i18n::tf("settings.autostart_failed", &[("err", &err.to_string())]);
            }
        }
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().slider_width = 220.0;
            ui.label(format!(
                "{}  {}",
                icon::LAPTOP,
                i18n::t("this_pc.touchpad_speed")
            ));
            ui.add(egui::Slider::new(&mut form.speed, 1..=200));
        });
        ui.add_space(12.0);
        sound(ui, form, self.own_key.as_deref());
        ui.add_space(10.0);
        egui::CollapsingHeader::new(i18n::t("this_pc.advanced")).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(format!("{}  {}", icon::LINK, i18n::t("this_pc.listen_on")));
                ui.add(egui::TextEdit::singleline(&mut form.listen).desired_width(200.0));
            });
            ui.weak(i18n::t("this_pc.listen_hint"));
        });
    }
}
