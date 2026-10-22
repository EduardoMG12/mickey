//! `mickey-ui history`: Mickey's clipboard history (PRD 0011), newest
//! first, with a search box. Clicking an entry (or Enter on the first one
//! found) copies it again and closes the window; Esc closes it.

use eframe::egui;
use std::process::Command;

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

/// Runs `mickey <args>` from next to this program.
fn mickey(args: &[&str]) -> Result<String, String> {
    let exe = std::env::current_exe().map_err(|err| err.to_string())?;
    let mut command =
        Command::new(exe.with_file_name(format!("mickey{}", std::env::consts::EXE_SUFFIX)));
    command.args(args);
    // mickey.exe is a console program: no console flashing over the window.
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    let output = command.output().map_err(|err| err.to_string())?;
    match output.status.success() {
        true => Ok(String::from_utf8_lossy(&output.stdout).into_owned()),
        false => Err(String::from_utf8_lossy(&output.stderr).trim().to_string()),
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
            let search = ui.add(
                egui::TextEdit::singleline(&mut self.search)
                    .hint_text("Search")
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
            ui.separator();
            if shown.is_empty() {
                ui.weak("Nothing copied yet");
            }
            egui::ScrollArea::vertical().show(ui, |ui| {
                for entry in shown {
                    let label = match entry.kind.as_str() {
                        "image" => format!("🖼  Image {}", entry.preview),
                        // Only spaces or line breaks: still something copied.
                        _ if entry.preview.is_empty() => "(blank)".to_string(),
                        _ => entry.preview.clone(),
                    };
                    let row = ui.horizontal(|ui| {
                        ui.weak(format!("{:>6}", ago(entry.ago)));
                        ui.add(
                            egui::Label::new(label)
                                .truncate()
                                .sense(egui::Sense::click()),
                        )
                    });
                    if row.inner.clicked() {
                        picked = Some(entry.id);
                    }
                }
            });
            if let Some(id) = picked {
                let _ = mickey(&["history", "copy", &id.to_string()]);
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        });
    }
}

pub fn window() -> std::process::ExitCode {
    let entries = mickey(&["history", "--lines"]).map(|text| parse(&text));
    // A small floating window in the middle, not a tile (Hyprland).
    crate::place::ask_compositor(crate::place::Corner::Center, "mickey-history");
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Mickey: clipboard history")
            .with_app_id("mickey-history")
            .with_inner_size([460.0, 420.0])
            .with_always_on_top(),
        ..Default::default()
    };
    let result = eframe::run_native(
        "mickey-history",
        options,
        Box::new(|_| {
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
