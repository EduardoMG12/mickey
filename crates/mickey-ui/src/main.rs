//! Mickey's windows. Today only the switcher: one card per PC, pick where
//! the keyboard and mouse go. See `choice.rs` for how it is started.

// No console window next to the GUI on Windows.
#![windows_subsystem = "windows"]

mod choice;
mod place;

use choice::Choice;
use eframe::egui;
use std::cell::Cell;
use std::process::ExitCode;
use std::rc::Rc;

/// The window's app id; Hyprland window rules match it (see the README).
const APP_ID: &str = "mickey-ui";
const CARD_SIZE: egui::Vec2 = egui::vec2(150.0, 90.0);

struct Switcher {
    choice: Choice,
    /// Whether the window still has to move itself to its corner (not when
    /// the compositor places it, as on Hyprland).
    needs_placing: bool,
    /// Shared with `main`, which prints it after the window closes.
    answer: Rc<Cell<Option<usize>>>,
}

impl eframe::App for Switcher {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if self.needs_placing {
            let (screen, window) = ctx.input(|input| {
                let viewport = input.viewport();
                (
                    viewport.monitor_size,
                    viewport.outer_rect.map(|rect| rect.size()),
                )
            });
            if let (Some(screen), Some(window)) = (screen, window) {
                let [x, y] = self
                    .choice
                    .corner
                    .position([screen.x, screen.y], [window.x, window.y]);
                ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(x, y)));
                self.needs_placing = false;
            }
        }
        let (next, previous, enter, escape) = ctx.input(|input| {
            (
                input.key_pressed(egui::Key::ArrowRight) || input.key_pressed(egui::Key::Tab),
                input.key_pressed(egui::Key::ArrowLeft),
                input.key_pressed(egui::Key::Enter),
                input.key_pressed(egui::Key::Escape),
            )
        });
        if next {
            self.choice.move_by(1);
        }
        if previous {
            self.choice.move_by(-1);
        }
        let mut clicked = None;
        egui::CentralPanel::default().show(ui, |ui| {
            ui.heading("Where should the keyboard and mouse go?");
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                for (index, pc) in self.choice.pcs.iter().enumerate() {
                    if card(
                        ui,
                        pc,
                        index == self.choice.selected,
                        index == self.choice.focused,
                    ) {
                        clicked = Some(index);
                    }
                }
            });
            ui.add_space(8.0);
            // Plain ASCII: the built-in fonts have no arrow or bullet glyphs.
            ui.weak("Left/Right: choose   Enter: go   Esc: cancel");
        });

        let answer = match (clicked, enter) {
            (Some(index), _) => self.choice.pick(index),
            (None, true) => self.choice.pick(self.choice.selected),
            (None, false) => None,
        };
        if answer.is_some() || escape {
            self.answer.set(answer);
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}

/// One PC as a card. Returns whether it was clicked.
fn card(ui: &mut egui::Ui, pc: &choice::Pc, selected: bool, focused: bool) -> bool {
    let visuals = ui.visuals();
    let fill = if selected {
        visuals.selection.bg_fill
    } else {
        visuals.faint_bg_color
    };
    let response = egui::Frame::NONE
        .fill(fill)
        .corner_radius(8.0)
        .inner_margin(10.0)
        .show(ui, |ui| {
            ui.set_min_size(CARD_SIZE);
            ui.vertical(|ui| {
                if pc.online {
                    ui.strong(&pc.name);
                    ui.label("online");
                } else {
                    ui.weak(&pc.name);
                    ui.weak("offline");
                }
                if focused {
                    ui.label("(in use)");
                }
            });
        })
        .response
        .interact(egui::Sense::click());
    pc.online && response.clicked()
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let choice = match Choice::from_args(&args) {
        Ok(choice) => choice,
        Err(err) => {
            eprintln!("mickey-ui: {err}");
            return ExitCode::from(2);
        }
    };
    let width = choice.pcs.len() as f32 * (CARD_SIZE.x + 28.0) + 24.0;
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Mickey")
            .with_app_id(APP_ID)
            .with_inner_size([width.max(360.0), 200.0])
            .with_resizable(false)
            .with_always_on_top(),
        ..Default::default()
    };
    let placed_by_compositor = place::ask_compositor(choice.corner, APP_ID);
    let answer = Rc::new(Cell::new(None));
    let window_answer = Rc::clone(&answer);
    let result = eframe::run_native(
        APP_ID,
        options,
        Box::new(|_| {
            Ok(Box::new(Switcher {
                choice,
                needs_placing: !placed_by_compositor,
                answer: window_answer,
            }))
        }),
    );
    if let Err(err) = result {
        eprintln!("mickey-ui: cannot open a window: {err}");
        return ExitCode::from(2);
    }
    match answer.get() {
        Some(index) => {
            println!("{index}");
            ExitCode::SUCCESS
        }
        None => ExitCode::from(1),
    }
}
