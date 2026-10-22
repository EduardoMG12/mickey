//! Mickey's windows. Today only the switcher: one card per PC, pick where
//! the keyboard and mouse go. See `choice.rs` for how it is started.

// No console window next to the GUI on Windows.
#![windows_subsystem = "windows"]

mod choice;
mod history;
mod layout;
mod pair;
mod place;
mod settings;
mod snap;
mod theme;

use choice::Choice;
use eframe::egui;
use std::cell::Cell;
use std::process::ExitCode;
use std::rc::Rc;

/// The window's app id; Hyprland window rules match it (see the README).
const APP_ID: &str = "mickey-ui";
const CARD_SIZE: egui::Vec2 = egui::vec2(118.0, 42.0);
const CARD_MARGIN: f32 = 6.0;

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
        const DIGITS: [egui::Key; 10] = [
            egui::Key::Num0,
            egui::Key::Num1,
            egui::Key::Num2,
            egui::Key::Num3,
            egui::Key::Num4,
            egui::Key::Num5,
            egui::Key::Num6,
            egui::Key::Num7,
            egui::Key::Num8,
            egui::Key::Num9,
        ];
        let (next, previous, enter, escape, digit) = ctx.input(|input| {
            (
                input.key_pressed(egui::Key::ArrowRight) || input.key_pressed(egui::Key::Tab),
                input.key_pressed(egui::Key::ArrowLeft),
                input.key_pressed(egui::Key::Enter),
                input.key_pressed(egui::Key::Escape),
                // Card 0 is this PC, card 1 the first other PC, and so on.
                DIGITS.iter().position(|&key| input.key_pressed(key)),
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
            ui.horizontal(|ui| {
                for (index, pc) in self.choice.pcs.iter().enumerate() {
                    if card(
                        ui,
                        index,
                        pc,
                        index == self.choice.selected,
                        index == self.choice.focused,
                    ) {
                        clicked = Some(index);
                    }
                }
            });
            ui.small("number or arrows · Enter: go · Esc: cancel");
        });

        let answer = match (clicked.or(digit), enter) {
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

/// One PC as a card: the chosen one filled with the accent and stitched
/// around, like the mascot's patches. Returns whether it was clicked.
fn card(ui: &mut egui::Ui, number: usize, pc: &choice::Pc, selected: bool, focused: bool) -> bool {
    use theme::{icon, palette};
    let fill = if selected {
        palette::BRUISE
    } else {
        palette::GRAPHITE
    };
    let response = egui::Frame::NONE
        .fill(fill)
        .stroke(egui::Stroke::new(1.0, palette::INK))
        .corner_radius(8.0)
        .inner_margin(CARD_MARGIN)
        .show(ui, |ui| {
            ui.set_min_size(CARD_SIZE);
            ui.vertical(|ui| {
                // Card 0 is always this PC.
                let kind = if number == 0 {
                    icon::LAPTOP
                } else {
                    icon::DESKTOP
                };
                let title = egui::RichText::new(format!("{kind}  {}", pc.name));
                let (dot, state) = match (pc.online, focused) {
                    (_, true) => (palette::HERE, "in use"),
                    (true, false) => (palette::FUR, "online"),
                    (false, false) => (palette::AWAY, "offline"),
                };
                if pc.online {
                    ui.label(title.strong());
                } else {
                    ui.label(title.color(palette::FUR));
                }
                ui.horizontal(|ui| {
                    let (spot, _) =
                        ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
                    ui.painter().circle_filled(spot.center(), 3.5, dot);
                    ui.small(format!("{number} · {state}"));
                });
            });
        })
        .response
        .interact(egui::Sense::click());
    if selected {
        theme::stitch(ui, response.rect.shrink(3.0), palette::BONE);
    }
    pc.online && response.clicked()
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("settings") {
        return settings_window();
    }
    if args.first().map(String::as_str) == Some("history") {
        return history::window();
    }
    if let [command, who, code] = args.as_slice()
        && command == "pair"
    {
        return pair::window(who, code);
    }
    let choice = match Choice::from_args(&args) {
        Ok(choice) => choice,
        Err(err) => {
            eprintln!("mickey-ui: {err}");
            return ExitCode::from(2);
        }
    };
    let card = CARD_SIZE.x + 2.0 * CARD_MARGIN + 8.0;
    let width = choice.pcs.len() as f32 * card + 16.0;
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Mickey")
            .with_app_id(APP_ID)
            .with_inner_size([width.max(220.0), 100.0])
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
        Box::new(|creation| {
            crate::theme::apply(&creation.egui_ctx);
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

/// `mickey-ui settings`: the settings window (`mickey settings` starts it).
fn settings_window() -> ExitCode {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Mickey settings")
            .with_app_id("mickey-settings")
            .with_inner_size([720.0, 500.0]),
        ..Default::default()
    };
    let result = eframe::run_native(
        "mickey-settings",
        options,
        Box::new(|creation| {
            theme::apply(&creation.egui_ctx);
            Ok(Box::new(settings::Settings::load()))
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
