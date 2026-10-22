//! `mickey-ui pair <who> <code>`: asks whether to pair with another PC
//! (PRD 0010). `mickey` starts it when a PC it does not know yet connects,
//! or when it connects to one; both PCs show the same code unless a
//! machine sits in between. Exit code 0 means *Pair*, 1 anything else.

use eframe::egui;
use std::cell::Cell;
use std::process::ExitCode;
use std::rc::Rc;

struct Pair {
    who: String,
    code: String,
    answer: Rc<Cell<bool>>,
}

impl eframe::App for Pair {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ui, |ui| {
            ui.vertical_centered(|ui| {
                use crate::theme::{self, icon};
                ui.add_space(6.0);
                ui.label(
                    egui::RichText::new(format!("{}  Connect with {}?", icon::LINK, self.who))
                        .size(16.0)
                        .strong(),
                );
                ui.add_space(6.0);
                theme::card(ui, |ui| {
                    ui.label(
                        egui::RichText::new(&self.code)
                            .size(36.0)
                            .monospace()
                            .strong(),
                    );
                });
                ui.add_space(6.0);
                ui.weak(format!(
                    "{}  Pair only if the other PC shows this same code.",
                    icon::SHIELD
                ));
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    // Centre the two buttons.
                    ui.add_space((ui.available_width() - 190.0).max(0.0) / 2.0);
                    let close =
                        |ui: &egui::Ui| ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                    if ui
                        .add_sized([90.0, 30.0], theme::primary(icon::CHECK, "Pair"))
                        .clicked()
                    {
                        self.answer.set(true);
                        close(ui);
                    }
                    if ui
                        .add_sized(
                            [90.0, 28.0],
                            egui::Button::new(format!("{}  Refuse", icon::X)),
                        )
                        .clicked()
                    {
                        close(ui);
                    }
                });
            });
        });
    }
}

pub fn window(who: &str, code: &str) -> ExitCode {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Mickey: pair with another PC")
            .with_app_id("mickey-pair")
            .with_inner_size([380.0, 240.0])
            .with_resizable(false)
            .with_always_on_top(),
        ..Default::default()
    };
    let answer = Rc::new(Cell::new(false));
    let window_answer = Rc::clone(&answer);
    let (who, code) = (who.to_string(), code.to_string());
    let result = eframe::run_native(
        "mickey-pair",
        options,
        Box::new(|creation| {
            crate::theme::apply(&creation.egui_ctx);
            Ok(Box::new(Pair {
                who,
                code,
                answer: window_answer,
            }))
        }),
    );
    match (result, answer.get()) {
        (Ok(()), true) => ExitCode::SUCCESS,
        (Ok(()), false) => ExitCode::from(1),
        (Err(err), _) => {
            eprintln!("mickey-ui: cannot open a window: {err}");
            ExitCode::from(2)
        }
    }
}
