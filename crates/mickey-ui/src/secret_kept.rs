//! "A password stayed on this PC" (#425): shown once, the first time a
//! copy its app marked secret did not go to another PC, so the owner
//! knows why nothing arrived, and where to change it.

use crate::i18n;
use eframe::egui;
use std::process::ExitCode;

pub fn window() -> ExitCode {
    let options = eframe::NativeOptions {
        viewport: crate::theme::viewport()
            .with_title(i18n::t("secret_kept.window_title"))
            .with_app_id("mickey-secret-kept")
            .with_inner_size([440.0, 190.0])
            .with_resizable(false)
            .with_always_on_top(),
        ..Default::default()
    };
    let result = eframe::run_native(
        "mickey-secret-kept",
        options,
        Box::new(|creation| {
            crate::theme::apply(&creation.egui_ctx);
            Ok(Box::new(SecretKept))
        }),
    );
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("mickey-ui: cannot open a window: {err}");
            ExitCode::from(2)
        }
    }
}

struct SecretKept;

impl eframe::App for SecretKept {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ui, |ui| {
            ui.heading(i18n::t("secret_kept.heading"));
            ui.add_space(8.0);
            ui.label(i18n::t("secret_kept.body"));
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                let close = ui.button(i18n::t("secret_kept.ok")).clicked()
                    || ui.input(|input| input.key_pressed(egui::Key::Enter));
                if ui.button(i18n::t("secret_kept.settings")).clicked() {
                    open_settings();
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
                if close {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
        });
    }
}

/// The settings window, where the option is (General).
fn open_settings() {
    let started = std::env::current_exe()
        .and_then(|exe| std::process::Command::new(exe).arg("settings").spawn());
    if let Err(err) = started {
        eprintln!("mickey-ui: cannot open the settings: {err}");
    }
}
