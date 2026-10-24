//! `mickey-ui pair <who> <code>`: asks whether to pair with another PC
//! (PRD 0010). `mickey` starts it when a PC it does not know yet connects,
//! or when it connects to one; both PCs show the same code unless a
//! machine sits in between.
//!
//! `mickey-ui join <who> <by>`: asks whether to let in a PC that `by`, a
//! paired PC, introduced to the group (#184).
//!
//! Exit code 0 means yes, 1 anything else.

use eframe::egui;
use std::cell::Cell;
use std::process::ExitCode;
use std::rc::Rc;

/// What the window asks.
pub enum Ask {
    /// Pair with a PC that connected: both PCs show this code.
    Pair { code: String },
    /// Let in a PC that `by` introduced through the group.
    Join { by: String },
}

struct Pair {
    who: String,
    ask: Ask,
    answer: Rc<Cell<bool>>,
}

impl eframe::App for Pair {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ui, |ui| {
            ui.vertical_centered(|ui| {
                use crate::theme::{self, icon};
                ui.add_space(6.0);
                let (title, yes) = match &self.ask {
                    Ask::Pair { .. } => (format!("Connect with {}?", self.who), "Pair"),
                    Ask::Join { .. } => (format!("Let {} in?", self.who), "Let in"),
                };
                ui.label(
                    egui::RichText::new(format!("{}  {title}", icon::LINK))
                        .size(16.0)
                        .strong(),
                );
                ui.add_space(6.0);
                match &self.ask {
                    Ask::Pair { code } => {
                        theme::card(ui, |ui| {
                            ui.label(egui::RichText::new(code).size(36.0).monospace().strong());
                        });
                        ui.add_space(6.0);
                        ui.weak(format!(
                            "{}  Pair only if the other PC shows this same code.",
                            icon::SHIELD
                        ));
                    }
                    Ask::Join { by } => {
                        theme::card(ui, |ui| {
                            ui.label(format!("{by} says {} is one of your PCs.", self.who));
                        });
                        ui.add_space(6.0);
                        ui.weak(format!(
                            "{}  Once in, it can use this PC's keyboard and mouse.",
                            icon::SHIELD
                        ));
                    }
                }
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    // Centre the two buttons.
                    ui.add_space((ui.available_width() - 190.0).max(0.0) / 2.0);
                    let close =
                        |ui: &egui::Ui| ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                    if ui
                        .add_sized([90.0, 30.0], theme::primary(icon::CHECK, yes))
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

pub fn window(who: &str, ask: Ask) -> ExitCode {
    let options = eframe::NativeOptions {
        viewport: crate::theme::viewport()
            .with_title("Mickey: pair with another PC")
            .with_app_id("mickey-pair")
            .with_inner_size([380.0, 240.0])
            .with_resizable(false)
            .with_always_on_top(),
        ..Default::default()
    };
    let answer = Rc::new(Cell::new(false));
    let window_answer = Rc::clone(&answer);
    let who = who.to_string();
    let result = eframe::run_native(
        "mickey-pair",
        options,
        Box::new(|creation| {
            crate::theme::apply(&creation.egui_ctx);
            Ok(Box::new(Pair {
                who,
                ask,
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
