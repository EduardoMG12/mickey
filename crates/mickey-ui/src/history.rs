//! `mickey-ui history`: Mickey's clipboard history (PRD 0011), newest
//! first, with a search box. Clicking an entry (or Enter on the first one
//! found) copies it again and closes the window; Esc closes it.

use eframe::egui;

#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    id: u64,
    kind: String,
    ago: u64,
    preview: String,
}

/// Parses `mickey history --lines`: id, kind, seconds ago, preview.
fn parse(text: &str) -> Vec<Entry> {
    text.lines()
        .filter_map(|line| {
            let [id, kind, ago, preview] = line.splitn(4, '\t').collect::<Vec<_>>()[..] else {
                return None;
            };
            Some(Entry {
                id: id.parse().ok()?,
                kind: kind.to_string(),
                ago: ago.parse().ok()?,
                preview: preview.to_string(),
            })
        })
        .collect()
}

fn ago(seconds: u64) -> String {
    match seconds {
        0..60 => format!("{seconds} s"),
        60..3600 => format!("{} min", seconds / 60),
        _ => format!("{} h", seconds / 3600),
    }
}

struct History {
    entries: Result<Vec<Entry>, String>,
    search: String,
}

impl eframe::App for History {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        egui::CentralPanel::default().show(ui, |ui| {
            use crate::theme::{icon, palette};
            ui.label(
                egui::RichText::new(format!("{}  Clipboard history", icon::CLIPBOARD))
                    .size(17.0)
                    .strong(),
            );
            ui.add_space(4.0);
            let search = ui.add(
                egui::TextEdit::singleline(&mut self.search)
                    .hint_text(format!("{}  Search", icon::SEARCH))
                    .desired_width(f32::INFINITY),
            );
            search.request_focus();
            let entries = match &self.entries {
                Ok(entries) => entries,
                Err(err) => {
                    ui.colored_label(ui.visuals().warn_fg_color, err);
                    return;
                }
            };
            let needle = self.search.to_lowercase();
            let shown: Vec<&Entry> = entries
                .iter()
                .filter(|entry| entry.preview.to_lowercase().contains(&needle))
                .collect();
            let enter = ui.input(|input| input.key_pressed(egui::Key::Enter));
            let mut picked = enter.then(|| shown.first().map(|entry| entry.id)).flatten();
            ui.add_space(4.0);
            if shown.is_empty() {
                ui.weak("Nothing copied yet");
            }
            egui::ScrollArea::vertical().show(ui, |ui| {
                for entry in shown {
                    let (kind, text) = match entry.kind.as_str() {
                        "image" => (icon::IMAGE, format!("Image {}", entry.preview)),
                        "files" => (icon::FILE, entry.preview.clone()),
                        // Only spaces or line breaks: still something copied.
                        _ if entry.preview.is_empty() => (icon::CLIPBOARD, "(blank)".to_string()),
                        _ => (icon::CLIPBOARD, entry.preview.clone()),
                    };
                    // A row is a small card that lights up under the mouse.
                    let row = egui::Frame::new()
                        .fill(palette::GRAPHITE)
                        .corner_radius(8)
                        .inner_margin(egui::Margin::symmetric(10, 7))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new(kind).color(palette::FUR));
                                ui.add(egui::Label::new(text).truncate());
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        ui.weak(ago(entry.ago));
                                    },
                                );
                            });
                        })
                        .response
                        .interact(egui::Sense::click());
                    if row.hovered() {
                        crate::theme::stitch(ui, row.rect.shrink(3.0), palette::BRUISE);
                    }
                    if row.clicked() {
                        picked = Some(entry.id);
                    }
                }
            });
            if let Some(id) = picked {
                let _ = crate::mickey::run(&["history", "copy", &id.to_string()]);
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        });
    }
}

pub fn window() -> std::process::ExitCode {
    let entries = crate::mickey::run(&["history", "--lines"]).map(|text| parse(&text));
    // A small floating window in the middle, not a tile (Hyprland).
    crate::place::ask_compositor(crate::place::Corner::Center, "mickey-history");
    let options = eframe::NativeOptions {
        viewport: crate::theme::viewport()
            .with_title("Mickey: clipboard history")
            .with_app_id("mickey-history")
            .with_inner_size([460.0, 420.0])
            .with_always_on_top(),
        ..Default::default()
    };
    let result = eframe::run_native(
        "mickey-history",
        options,
        Box::new(|creation| {
            crate::theme::apply(&creation.egui_ctx);
            Ok(Box::new(History {
                entries,
                search: String::new(),
            }))
        }),
    );
    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("mickey-ui: cannot open a window: {err}");
            std::process::ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_become_entries_and_bad_ones_are_skipped() {
        let text = "7\ttext\t3\thello there\n5\timage\t70\t800x600\nnonsense\n";
        let entries = parse(text);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[1].kind, "image");
        assert_eq!(
            (ago(3), ago(70), ago(7200)),
            ("3 s".into(), "1 min".into(), "2 h".into())
        );
    }
}
