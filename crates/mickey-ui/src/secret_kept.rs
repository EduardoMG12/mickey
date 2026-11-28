//! "A password stayed on this PC" (#425): shown when a copy its app marked
//! secret did not go to another PC, so the owner knows why nothing arrived
//! and where to change it. "Don't show again" writes the file `mickey`
//! passes, and it is not shown again.

use crate::i18n;
use eframe::egui;
use std::path::PathBuf;
use std::process::ExitCode;

/// The window; `off` is the file to write once the owner ticks "Don't
/// show again".
pub fn window(off: PathBuf) -> ExitCode {
    let options = eframe::NativeOptions {
        viewport: crate::theme::viewport()
            .with_title(i18n::t("secret_kept.window_title"))
            .with_app_id("mickey-secret-kept")
            .with_inner_size([440.0, 210.0])
            .with_resizable(false)
            .with_always_on_top()
            // Shown just after the keyboard crossed, while the owner may be
            // typing: it must not take their keys (a Space ticked "Don't show
            // again" while testing it). Windows only: Wayland and X11 ignore
            // it, so the box below also never keeps the keyboard.
            .with_active(false),
        ..Default::default()
    };
    let result = eframe::run_native(
        "mickey-secret-kept",
        options,
        Box::new(|creation| {
            crate::theme::apply(&creation.egui_ctx);
            Ok(Box::new(SecretKept { off, never: false }))
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

struct SecretKept {
    off: PathBuf,
    /// "Don't show again", ticked.
    never: bool,
}

impl eframe::App for SecretKept {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // Closed with the window's X, ticked: that counts too.
        if self.never && ui.ctx().input(|input| input.viewport().close_requested()) {
            turn_off(&self.off);
        }
        egui::CentralPanel::default().show(ui, |ui| {
            ui.heading(i18n::t("secret_kept.heading"));
            ui.add_space(8.0);
            ui.label(i18n::t("secret_kept.body"));
            ui.add_space(8.0);
            let never = ui.checkbox(&mut self.never, i18n::t("secret_kept.never"));
            // Only a click ticks it: a Space typed for another window must
            // not.
            if never.has_focus() {
                ui.memory_mut(|memory| memory.surrender_focus(never.id));
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                // No Enter shortcut: a key meant for another window must
                // not close this one.
                let ok = ui.button(i18n::t("secret_kept.ok")).clicked();
                let settings = ui.button(i18n::t("secret_kept.settings")).clicked();
                if settings {
                    open_settings();
                }
                if ok || settings {
                    if self.never {
                        turn_off(&self.off);
                    }
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
        });
    }
}

/// Writes `off`, so `mickey` does not show this again.
fn turn_off(off: &std::path::Path) {
    let written = off
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| std::fs::write(off, ""));
    if let Err(err) = written {
        eprintln!("mickey-ui: cannot remember not to show this again: {err}");
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
