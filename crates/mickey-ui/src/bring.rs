//! `mickey-ui bring <who> <bytes> <what> <held> <folder> <items>`: asks
//! whether to bring files `who` offered, over `[interface] bring_over_gb`
//! (PRD 0015); `<what>` is one line per copied file or folder. When
//! `<folder>`, where they go, is worth cleaning first, `<held>` is how
//! many bytes it holds (`0` when not), and `<items>` its biggest things,
//! one per line: `<name>\t<bytes>\t<days since it arrived>\t<days since
//! it changed>\t<1 if incomplete>`. Bring (or Enter) exits 0; Ignore, Esc
//! or closing, 1.
//!
//! It is a notice about files arriving, so it appears where the owner
//! wants transfers shown (`Corner::for_transfers`), with an app id of its
//! own for the compositor's rules.

use crate::i18n;
use crate::place::{self, Corner};
use crate::theme::{self, icon};
use eframe::egui;
use std::cell::Cell;
use std::process::ExitCode;
use std::rc::Rc;

/// What the owner is asked about.
pub struct Asked {
    pub who: String,
    pub bytes: u64,
    pub what: String,
    /// What the folder already holds, when worth cleaning first.
    pub held: Option<u64>,
    pub folder: String,
    /// Its biggest things, to say how old each is.
    pub items: Vec<Item>,
}

/// One thing in the folder, as `mickey` measured it.
#[derive(Debug, PartialEq)]
pub struct Item {
    pub name: String,
    pub bytes: u64,
    pub arrived_days: u64,
    pub changed_days: u64,
    pub incomplete: bool,
}

/// The `<items>` argument, skipping a line it does not understand.
pub fn items(lines: &str) -> Vec<Item> {
    lines
        .lines()
        .filter_map(|line| {
            let [name, bytes, arrived, changed, incomplete] =
                line.split('\t').collect::<Vec<_>>()[..]
            else {
                return None;
            };
            Some(Item {
                name: name.to_string(),
                bytes: bytes.parse().ok()?,
                arrived_days: arrived.parse().ok()?,
                changed_days: changed.parse().ok()?,
                incomplete: incomplete == "1",
            })
        })
        .collect()
}

/// How old `item` is, in words: "arrived 47 days ago, never changed".
fn age(item: &Item) -> String {
    if item.incomplete {
        return i18n::t("bring.item_incomplete").to_string();
    }
    let arrived = match item.arrived_days {
        0 => i18n::t("bring.item_arrived_today").to_string(),
        1 => i18n::t("bring.item_arrived_yesterday").to_string(),
        days => i18n::tf("bring.item_arrived", &[("days", &days.to_string())]),
    };
    let changed = match item.changed_days {
        _ if item.changed_days >= item.arrived_days => {
            i18n::t("bring.item_never_changed").to_string()
        }
        0 => i18n::t("bring.item_changed_today").to_string(),
        1 => i18n::t("bring.item_changed_yesterday").to_string(),
        days => i18n::tf("bring.item_changed", &[("days", &days.to_string())]),
    };
    format!("{arrived}, {changed}")
}

struct Bring {
    asked: Asked,
    answer: Rc<Cell<bool>>,
    /// The corner the window still has to move itself to, where the
    /// compositor does not place it (Windows, X11).
    place: Option<Corner>,
}

impl eframe::App for Bring {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if let Some(corner) = self.place
            && place::move_to(ui.ctx(), corner)
        {
            self.place = None;
        }
        let close = |ui: &egui::Ui| ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        egui::CentralPanel::default().show(ui, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(6.0);
                let question = i18n::tf("bring.question", &[("name", &self.asked.who)]);
                ui.label(
                    egui::RichText::new(format!("{}  {question}", icon::ARROW_DOWN))
                        .size(16.0)
                        .strong(),
                );
                ui.add_space(6.0);
                theme::card(ui, |ui| {
                    for line in self.asked.what.lines() {
                        ui.label(format!("{}  {line}", icon::FILE));
                    }
                });
                ui.add_space(6.0);
                let size = crate::units::size(self.asked.bytes);
                ui.label(i18n::tf("bring.size", &[("size", &size)]));
                ui.weak(i18n::tf("bring.keep", &[("name", &self.asked.who)]));
                if let Some(held) = self.asked.held {
                    ui.add_space(6.0);
                    let held = crate::units::size(held);
                    ui.label(i18n::tf("bring.worth_cleaning", &[("size", &held)]));
                    for item in &self.asked.items {
                        let size = crate::units::size(item.bytes);
                        ui.weak(format!("{}  ·  {size}  ·  {}", item.name, age(item)));
                    }
                    if ui
                        .button(format!("{}  {}", icon::FILE, i18n::t("progress.open")))
                        .clicked()
                    {
                        crate::progress::open_folder(&self.asked.folder);
                    }
                }
                ui.add_space(10.0);
                let keys = ui.input(|input| {
                    if input.key_pressed(egui::Key::Enter) {
                        Some(true)
                    } else if input.key_pressed(egui::Key::Escape) {
                        Some(false)
                    } else {
                        None
                    }
                });
                let clicked = theme::yes_no(ui, i18n::t("bring.bring"), i18n::t("bring.ignore"));
                if let Some(bring) = keys.or(clicked) {
                    self.answer.set(bring);
                    close(ui);
                }
            });
        });
    }
}

pub fn window(asked: Asked) -> ExitCode {
    const APP_ID: &str = "mickey-bring";
    let corner = Corner::for_transfers();
    let placed_by_compositor = place::ask_compositor(corner, APP_ID);
    // Room for each line of what is brought (five at most), and for the
    // cleaning hint.
    let hint = if asked.held.is_some() {
        70.0 + 20.0 * asked.items.len() as f32
    } else {
        0.0
    };
    let height = 200.0 + 28.0 * asked.what.lines().count() as f32 + hint;
    let options = eframe::NativeOptions {
        viewport: theme::viewport()
            .with_title(i18n::t("bring.window_title"))
            .with_app_id(APP_ID)
            .with_inner_size([420.0, height])
            .with_resizable(false)
            .with_always_on_top(),
        ..Default::default()
    };
    let answer = Rc::new(Cell::new(false));
    let window = Bring {
        asked,
        answer: Rc::clone(&answer),
        place: (!placed_by_compositor).then_some(corner),
    };
    let result = eframe::run_native(
        APP_ID,
        options,
        Box::new(|creation| {
            theme::apply(&creation.egui_ctx);
            Ok(Box::new(window))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_items_mickey_measured_are_read() {
        let read = items("fotos\t9000\t47\t47\t0\nbroken line\nx.zip\t5\t1\t0\t1");
        assert_eq!(
            read,
            [
                Item {
                    name: "fotos".into(),
                    bytes: 9000,
                    arrived_days: 47,
                    changed_days: 47,
                    incomplete: false,
                },
                Item {
                    name: "x.zip".into(),
                    bytes: 5,
                    arrived_days: 1,
                    changed_days: 0,
                    incomplete: true,
                },
            ]
        );
    }
}
