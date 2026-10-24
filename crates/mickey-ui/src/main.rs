//! Mickey's windows: the switcher (one card per PC, pick where the
//! keyboard and mouse go; see `choice.rs` for how it is started), the
//! settings (`settings`), the clipboard history (`history`) and the
//! pairing question (`pair`).

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
const CARD_SIZE: egui::Vec2 = egui::vec2(96.0, 22.0);
const CARD_MARGIN: f32 = 7.0;
/// A copy closes by itself after this, should Mickey's "close" never come.
const MIRROR_LIFETIME: std::time::Duration = std::time::Duration::from_secs(30);

struct Switcher {
    choice: Choice,
    /// Whether the window still has to move itself to its corner (not when
    /// the compositor places it, as on Hyprland).
    needs_placing: bool,
    /// Shared with `main`, which prints it after the window closes.
    answer: Rc<Cell<Option<usize>>>,
    /// A copy shown on the other PCs: it takes no keys or clicks, and Mickey
    /// closes it once the owner picks on the PC in use.
    mirror: Option<std::time::Instant>,
    /// Each card's least inner width, from `measure`: equal in a column.
    card_width: f32,
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
        if let Some(opened) = self.mirror {
            if opened.elapsed() > MIRROR_LIFETIME {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            // Wakes up to check, with no input of its own.
            ctx.request_repaint_after(std::time::Duration::from_secs(1));
        }
        let (next, previous, enter, escape, digit) = ctx.input(|input| {
            (
                input.key_pressed(egui::Key::ArrowRight)
                    || input.key_pressed(egui::Key::ArrowDown)
                    || input.key_pressed(egui::Key::Tab),
                input.key_pressed(egui::Key::ArrowLeft) || input.key_pressed(egui::Key::ArrowUp),
                input.key_pressed(egui::Key::Enter),
                input.key_pressed(egui::Key::Escape),
                // Card 0 is this PC, card 1 the first other PC, and so on.
                DIGITS.iter().position(|&key| input.key_pressed(key)),
            )
        });
        // A copy only shows; the keys belong to the PC in use.
        let (next, previous, enter, escape, digit) = if self.mirror.is_some() {
            (false, false, false, false, None)
        } else {
            (next, previous, enter, escape, digit)
        };
        if next {
            self.choice.move_by(1);
        }
        if previous {
            self.choice.move_by(-1);
        }
        let mut clicked = None;
        egui::CentralPanel::default().show(ui, |ui| {
            clicked = cards(ui, &self.choice, self.card_width).0;
        });
        if self.mirror.is_some() {
            clicked = None;
        }

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
fn card(
    ui: &mut egui::Ui,
    number: usize,
    pc: &choice::Pc,
    width: f32,
    selected: bool,
    focused: bool,
) -> egui::Response {
    use theme::palette;
    // Text that can be selected takes the click for itself, before the card.
    ui.style_mut().interaction.selectable_labels = false;
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
            ui.set_min_size(egui::vec2(width, CARD_SIZE.y));
            // One line: its number (the key that picks it), a status dot
            // (green in use, grey online, red offline) and its name.
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(number.to_string())
                        .small()
                        .color(palette::FUR),
                );
                let dot = match (pc.online, focused) {
                    (_, true) => palette::HERE,
                    (true, false) => palette::FUR,
                    (false, false) => palette::AWAY,
                };
                let (spot, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
                ui.painter().circle_filled(spot.center(), 3.5, dot);
                let name = egui::RichText::new(&pc.name);
                ui.label(if pc.online {
                    name.strong()
                } else {
                    name.color(palette::FUR)
                });
            });
        })
        .response
        .interact(egui::Sense::click());
    if selected {
        theme::stitch(ui, response.rect.shrink(3.0), palette::BONE);
    }
    response
}

/// The window size that fits every card exactly. The row is laid out once,
/// off screen, with the same theme and `card`: a window that resizes itself
/// after opening is not honoured by every compositor (Hyprland ignores it),
/// and names differ in length from PC to PC.
fn measure(choice: &Choice) -> (egui::Vec2, f32) {
    let ctx = egui::Context::default();
    theme::apply(&ctx);
    let (mut row, mut width) = (egui::Rect::NOTHING, CARD_SIZE.x);
    // Fonts only exist from the first pass on; the third lays a column out
    // with every card as wide as the widest.
    for _ in 0..3 {
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                let (_, rect, widest) = cards(ui, choice, width);
                row = rect;
                if choice.vertical {
                    width = width.max(widest - 2.0 * CARD_MARGIN - 2.0);
                }
            });
        });
        // Nothing is drawn: the font textures it made are dropped on purpose.
        output.textures_delta.clear();
    }
    let size = egui::vec2(row.right() + 10.0, row.bottom() + 10.0).max(egui::vec2(160.0, 52.0));
    (size, width)
}

/// Draws every card, in a row or a column, each at least `width` wide.
/// Returns the card clicked (if online), the cards' rectangle, and the
/// widest card.
fn cards(ui: &mut egui::Ui, choice: &Choice, width: f32) -> (Option<usize>, egui::Rect, f32) {
    let (mut clicked, mut widest) = (None, 0.0f32);
    let mut draw = |ui: &mut egui::Ui| {
        for (index, pc) in choice.pcs.iter().enumerate() {
            let selected = index == choice.selected;
            let response = card(ui, index, pc, width, selected, index == choice.focused);
            widest = widest.max(response.rect.width());
            if pc.online && response.clicked() {
                clicked = Some(index);
            }
        }
    };
    let rect = if choice.vertical {
        ui.vertical(|ui| draw(ui)).response.rect
    } else {
        ui.horizontal(|ui| draw(ui)).response.rect
    };
    (clicked, rect, widest)
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
        return pair::window(who, pair::Ask::Pair { code: code.clone() });
    }
    if let [command, who, by] = args.as_slice()
        && command == "join"
    {
        return pair::window(who, pair::Ask::Join { by: by.clone() });
    }
    // `switcher --mirror …`: the copy for the PCs not in use.
    let mirror = args.iter().any(|arg| arg == "--mirror");
    let args: Vec<String> = args.into_iter().filter(|arg| arg != "--mirror").collect();
    let choice = match Choice::from_args(&args) {
        Ok(choice) => choice,
        Err(err) => {
            eprintln!("mickey-ui: {err}");
            return ExitCode::from(2);
        }
    };
    let (size, card_width) = measure(&choice);
    let options = eframe::NativeOptions {
        viewport: crate::theme::viewport()
            .with_title("Mickey")
            .with_app_id(APP_ID)
            .with_inner_size(size)
            .with_always_on_top()
            // No title bar where the window draws its own (GNOME, Plasma);
            // Hyprland draws none anyway. Windows keeps its look.
            .with_decorations(cfg!(windows))
            // A switcher is not an app that was opened: no taskbar button.
            .with_taskbar(false)
            // Not mouse passthrough for the copy: on Windows that makes the
            // whole window transparent.
            .with_active(!mirror),
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
                mirror: mirror.then(std::time::Instant::now),
                card_width,
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
        viewport: crate::theme::viewport()
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
