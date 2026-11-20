//! `mickey-ui progress <who>`: how a transfer of files from `who` is going
//! (PRD 0015), in the corner the Bring question uses. `mickey` writes one
//! line on this window's input whenever there is news, fields separated by
//! tabs:
//!
//! ```text
//! <stage>  <done bytes>  <total bytes>  <bytes a second>  <seconds left>
//! skipped  <why>         <name>
//! done     <files>       <bytes>        <folder>
//! failed   <why>
//! ```
//!
//! `skipped` lines come before `done`, which lists them: one per file
//! that did not come, with why. `changed` or `gone` on the other PC while
//! it was fetched; or never offered: `link`, `name` (Windows refuses it),
//! `case` (another name differs only in case), `not-a-file`, `unreadable`.
//!
//! The stage is `queued`, `space`, `rereading`, `transferring` or `paused`; speed and
//! time left may be empty while not known. Cancel prints `cancel` and
//! exits 1; closing after `done` exits 0.

use crate::i18n;
use crate::units;
use eframe::egui;
use std::io::BufRead;
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// What the window shows: the last line `mickey` sent.
#[derive(Debug, Clone, PartialEq)]
enum State {
    Starting,
    Going {
        stage: String,
        done: u64,
        total: u64,
        speed: Option<f64>,
        left: Option<u64>,
    },
    Done {
        files: u64,
        bytes: u64,
        folder: String,
        /// Files not sent: why (`changed`, `gone`) and the name.
        skipped: Vec<(String, String)>,
    },
    Failed(String),
}

/// A line from `mickey`: how the transfer is now, or one skipped file.
#[derive(Debug, PartialEq)]
enum Line {
    State(State),
    Skipped { why: String, name: String },
}

/// One line from `mickey`; `None` for one it does not understand.
fn parse(line: &str) -> Option<Line> {
    // Only the line's end: trailing tabs are empty fields.
    let fields: Vec<&str> = line.trim_end_matches(['\n', '\r']).split('\t').collect();
    Some(Line::State(match fields.as_slice() {
        ["skipped", why, name] => {
            return Some(Line::Skipped {
                why: why.to_string(),
                name: name.to_string(),
            });
        }
        ["done", files, bytes, folder] => State::Done {
            files: files.parse().ok()?,
            bytes: bytes.parse().ok()?,
            folder: folder.to_string(),
            skipped: Vec::new(),
        },
        ["failed", why] => State::Failed(why.to_string()),
        [stage, done, total, speed, left] => State::Going {
            stage: stage.to_string(),
            done: done.parse().ok()?,
            total: total.parse().ok()?,
            speed: speed.parse().ok(),
            left: left.parse().ok(),
        },
        _ => return None,
    }))
}

struct Progress {
    who: String,
    state: Arc<Mutex<State>>,
    place: Option<crate::place::Corner>,
    /// Set by Cancel; read once the window has closed.
    cancelled: Arc<AtomicBool>,
    /// The Ratão, running while files come.
    running: Option<crate::theme::Running>,
}

impl eframe::App for Progress {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if let Some(corner) = self.place
            && crate::place::move_to(ui.ctx(), corner)
        {
            self.place = None;
        }
        let state = self
            .state
            .lock()
            .map(|state| state.clone())
            .unwrap_or(State::Starting);
        egui::CentralPanel::default().show(ui, |ui| {
            use crate::theme::{self, icon};
            ui.add_space(6.0);
            ui.label(
                egui::RichText::new(format!(
                    "{}  {}",
                    icon::ARROW_DOWN,
                    i18n::tf("progress.from", &[("name", &self.who)])
                ))
                .size(16.0)
                .strong(),
            );
            ui.add_space(8.0);
            let close = |ui: &egui::Ui| ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            match &state {
                State::Starting => {
                    ui.label(i18n::t("progress.starting"));
                }
                State::Going {
                    stage,
                    done,
                    total,
                    speed,
                    left,
                } => {
                    let fraction = if *total == 0 {
                        1.0
                    } else {
                        *done as f32 / *total as f32
                    };
                    let mut line = format!("{} / {}", units::size(*done), units::size(*total));
                    if let Some(speed) = speed {
                        line += &format!(" · {}/s", units::size(*speed as u64));
                    }
                    if let Some(left) = left {
                        line += &format!(
                            " · {}",
                            i18n::tf("progress.left", &[("time", &units::time(*left))])
                        );
                    }
                    ui.horizontal(|ui| {
                        // It runs while files come, not while paused.
                        if let Some(running) = &self.running
                            && stage != "paused"
                        {
                            running.show(ui, 64.0);
                        }
                        ui.vertical(|ui| {
                            ui.add(egui::ProgressBar::new(fraction).show_percentage());
                            ui.add_space(4.0);
                            ui.label(line);
                            ui.weak(stage_text(stage));
                        });
                    });
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui
                            .button(format!("{}  {}", icon::X, i18n::t("progress.cancel")))
                            .clicked()
                        {
                            // `mickey` reads this and stops; what arrived
                            // stays, to resume.
                            println!("cancel");
                            self.cancelled.store(true, Ordering::SeqCst);
                            close(ui);
                        }
                        // Out of the way; the transfer goes on.
                        if ui.button(i18n::t("progress.minimize")).clicked() {
                            ui.ctx()
                                .send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                        }
                    });
                }
                State::Done {
                    files,
                    bytes,
                    folder,
                    skipped,
                } => {
                    ui.label(i18n::tf(
                        "progress.done",
                        &[
                            ("files", &files.to_string()),
                            ("size", &units::size(*bytes)),
                        ],
                    ));
                    ui.weak(folder.as_str());
                    if !skipped.is_empty() {
                        ui.add_space(4.0);
                        let count = skipped.len().to_string();
                        ui.label(i18n::tf("progress.skipped", &[("count", &count)]));
                        egui::ScrollArea::vertical()
                            .max_height(90.0)
                            .show(ui, |ui| {
                                for (why, name) in skipped {
                                    let why = match why.as_str() {
                                        "changed" => i18n::t("progress.skipped_changed"),
                                        "link" => i18n::t("progress.skipped_link"),
                                        "name" => i18n::t("progress.skipped_name"),
                                        "case" => i18n::t("progress.skipped_case"),
                                        "not-a-file" => i18n::t("progress.skipped_not_a_file"),
                                        "unreadable" => i18n::t("progress.skipped_unreadable"),
                                        _ => i18n::t("progress.skipped_gone"),
                                    };
                                    ui.weak(format!("{name}  ({why})"));
                                }
                            });
                    }
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui
                            .add(theme::primary(icon::FILE, i18n::t("progress.open")))
                            .clicked()
                        {
                            open_folder(folder);
                            close(ui);
                        }
                        if ui
                            .button(format!("{}  {}", icon::CHECK, i18n::t("progress.close")))
                            .clicked()
                        {
                            close(ui);
                        }
                    });
                }
                State::Failed(why) => {
                    ui.label(i18n::t("progress.failed"));
                    ui.weak(why.as_str());
                    ui.add_space(8.0);
                    if ui
                        .button(format!("{}  {}", icon::X, i18n::t("progress.close")))
                        .clicked()
                    {
                        close(ui);
                    }
                }
            }
        });
    }
}

/// The stage's words for the owner.
fn stage_text(stage: &str) -> &'static str {
    match stage {
        "queued" => i18n::t("progress.stage_queued"),
        "space" => i18n::t("progress.stage_space"),
        "rereading" => i18n::t("progress.stage_rereading"),
        "paused" => i18n::t("progress.stage_paused"),
        _ => i18n::t("progress.stage_transferring"),
    }
}

/// Opens `folder` in the system's file manager.
pub fn open_folder(folder: &str) {
    let program = if cfg!(windows) {
        "explorer"
    } else {
        "xdg-open"
    };
    if let Err(err) = std::process::Command::new(program).arg(folder).spawn() {
        eprintln!("mickey-ui: cannot open {folder}: {err}");
    }
}

pub fn window(who: &str) -> ExitCode {
    let corner = crate::place::Corner::for_transfers();
    let placed_by_compositor = crate::place::ask_compositor(corner, "mickey-progress");
    let options = eframe::NativeOptions {
        viewport: crate::theme::viewport()
            .with_title(i18n::t("progress.window_title"))
            .with_app_id("mickey-progress")
            .with_inner_size([420.0, 200.0])
            .with_resizable(false)
            .with_always_on_top(),
        ..Default::default()
    };
    let state = Arc::new(Mutex::new(State::Starting));
    let cancelled = Arc::new(AtomicBool::new(false));
    let (who, window_state, window_cancelled) = (who.to_string(), state.clone(), cancelled.clone());
    let result = eframe::run_native(
        "mickey-progress",
        options,
        Box::new(move |creation| {
            crate::theme::apply(&creation.egui_ctx);
            // Reads `mickey`'s lines on a thread, waking the window for each.
            let (ctx, reading) = (creation.egui_ctx.clone(), window_state.clone());
            std::thread::spawn(move || {
                // Skipped files come before `done`, which lists them.
                let mut skipped = Vec::new();
                for line in std::io::stdin().lock().lines().map_while(Result::ok) {
                    let new = match parse(&line) {
                        Some(Line::Skipped { why, name }) => {
                            skipped.push((why, name));
                            continue;
                        }
                        Some(Line::State(State::Done {
                            files,
                            bytes,
                            folder,
                            ..
                        })) => State::Done {
                            files,
                            bytes,
                            folder,
                            skipped: std::mem::take(&mut skipped),
                        },
                        Some(Line::State(new)) => new,
                        None => continue,
                    };
                    if let Ok(mut state) = reading.lock() {
                        *state = new;
                    }
                    ctx.request_repaint();
                }
            });
            Ok(Box::new(Progress {
                who,
                state: window_state,
                place: (!placed_by_compositor).then_some(corner),
                cancelled: window_cancelled,
                running: crate::theme::Running::load(&creation.egui_ctx),
            }))
        }),
    );
    match (result, cancelled.load(Ordering::SeqCst)) {
        (Ok(()), false) => ExitCode::SUCCESS,
        (Ok(()), true) => ExitCode::from(1),
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
    fn mickeys_lines_are_read() {
        let state = |line| match parse(line) {
            Some(Line::State(state)) => Some(state),
            _ => None,
        };
        assert_eq!(
            state("transferring\t500\t1000\t25000000\t20\n"),
            Some(State::Going {
                stage: "transferring".into(),
                done: 500,
                total: 1000,
                speed: Some(25e6),
                left: Some(20),
            })
        );
        assert_eq!(
            state("space\t0\t1000\t\t"),
            Some(State::Going {
                stage: "space".into(),
                done: 0,
                total: 1000,
                speed: None,
                left: None,
            })
        );
        assert_eq!(
            state("done\t3\t1300000000\t/home/ana/Downloads/Mickey"),
            Some(State::Done {
                files: 3,
                bytes: 1_300_000_000,
                folder: "/home/ana/Downloads/Mickey".into(),
                skipped: Vec::new(),
            })
        );
        assert_eq!(
            parse("skipped\tchanged\tfotos/a.jpg"),
            Some(Line::Skipped {
                why: "changed".into(),
                name: "fotos/a.jpg".into(),
            })
        );
        assert_eq!(
            state("failed\tno room"),
            Some(State::Failed("no room".into()))
        );
        assert_eq!(state("transferring\tmany\t1\t\t"), None);
        assert_eq!(state(""), None);
    }
}
