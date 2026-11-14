//! What the settings window asks the running Mickey about other PCs: the
//! ones heard on the network, and the paired ones as it sees them.

use super::sound::volume;
use super::{PcRow, Settings};
use crate::i18n;
use eframe::egui;
use mickey_config as config;
use mickey_config::peers::Peers;
use std::time::Instant;

/// A PC running Mickey heard on the network.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Nearby {
    pub(super) name: String,
    pub(super) address: String,
    pub(super) fingerprint: String,
}

/// `mickey discover --lines`: name, address, fingerprint per line. Empty
/// when Mickey cannot say.
pub(super) fn load_nearby() -> Vec<Nearby> {
    parse_nearby(&crate::mickey::ask(&["discover", "--lines"]).unwrap_or_default())
}

pub(super) fn parse_nearby(text: &str) -> Vec<Nearby> {
    text.lines()
        .filter_map(|line| {
            let [name, address, fingerprint] = line.split('\t').collect::<Vec<_>>()[..] else {
                return None;
            };
            Some(Nearby {
                name: name.to_string(),
                address: address.to_string(),
                fingerprint: fingerprint.to_string(),
            })
        })
        .collect()
}

/// The PC named in `mickey connect`'s answer (its `mickey: ` already
/// taken off), when a pairing started.
pub(super) fn pairing_with(answer: &str) -> Option<String> {
    let rest = answer.strip_prefix("pairing with ")?;
    let (name, _) = rest.split_once(": ")?;
    Some(name.to_string())
}

pub(super) fn load_peers() -> Result<Peers, String> {
    let text = crate::mickey::ask(&["peers"])?;
    Peers::from_text(&text).ok_or_else(|| format!("mickey peers answered `{text}`"))
}

impl Settings {
    /// PCs: the paired PCs, their volumes, and the PCs nearby to connect.
    pub(super) fn pcs_tab(&mut self, ui: &mut egui::Ui) {
        use crate::theme::{self, icon};
        let form = &mut self.form;
        // What the window last heard: who is online, their monitors.
        let online = |name: &str| {
            self.peers
                .as_ref()
                .ok()
                .and_then(|peers| peers.peers.iter().find(|peer| peer.name == name))
        };
        ui.label(
            egui::RichText::new(format!("{}  {}", icon::DESKTOP, i18n::t("pcs.your_pcs")))
                .size(16.0)
                .strong(),
        );
        if form.pcs.is_empty() {
            ui.weak(i18n::t("pcs.none_yet"));
        }
        let mut remove = None;
        let confirm = &mut self.confirm_remove;
        egui::Grid::new("pcs")
            .num_columns(6)
            .spacing([10.0, 8.0])
            .show(ui, |ui| {
                for (i, pc) in form.pcs.iter_mut().enumerate() {
                    let info = online(&pc.name);
                    let up = info.is_some_and(|info| info.online);
                    ui.horizontal(|ui| {
                        let (spot, _) =
                            ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                        let colour = if up {
                            theme::palette::HERE
                        } else {
                            theme::palette::FUR
                        };
                        ui.painter().circle_filled(spot.center(), 4.0, colour);
                        ui.add_sized([140.0, 24.0], egui::TextEdit::singleline(&mut pc.name));
                    });
                    if info.is_some_and(|info| info.reinstalled) {
                        ui.colored_label(theme::palette::AWAY, i18n::t("pcs.new_identity"))
                            .on_hover_text(i18n::tf(
                                "pcs.new_identity_hover",
                                &[("name", &pc.address)],
                            ));
                    } else {
                        ui.weak(format!("{}  {}", icon::LINK, pc.address));
                    }
                    ui.horizontal(|ui| {
                        let monitors_text = match info.map(|info| info.monitors.len()) {
                            Some(0) | None => i18n::t("pcs.monitors_unknown").to_string(),
                            Some(1) => format!("{}  {}", icon::MONITOR, i18n::t("pcs.one_monitor")),
                            Some(n) => i18n::tf("pcs.n_monitors", &[("n", &n.to_string())]),
                        };
                        ui.weak(monitors_text);
                        // Its Mickey version (PRD 0013): one behind this PC
                        // stands out, since releases fix security and speed.
                        if let Some(version) = info.and_then(|info| info.version.as_deref()) {
                            if mickey_config::version::is_newer(
                                mickey_config::version::THIS,
                                version,
                            ) {
                                ui.colored_label(
                                    theme::palette::AWAY,
                                    i18n::tf("pcs.version_update", &[("version", version)]),
                                )
                                .on_hover_text(i18n::tf(
                                    "pcs.version_update_hover",
                                    &[("this", mickey_config::version::THIS), ("name", &pc.name)],
                                ));
                            } else {
                                ui.weak(version);
                            }
                        }
                    });
                    ui.add_sized(
                        [170.0, 24.0],
                        egui::TextEdit::singleline(&mut pc.hotkey)
                            .hint_text(i18n::t("pcs.own_hotkey")),
                    );
                    match &pc.key {
                        Some(key) => volume(ui, key, &mut form.volumes, &mut form.muted),
                        None => {
                            ui.weak(i18n::t("pcs.volume_once_connected"));
                        }
                    }
                    // Removing stops trusting it too (on Save): asked twice.
                    if confirm.as_deref() == Some(pc.name.as_str()) {
                        ui.horizontal(|ui| {
                            if ui
                                .button(format!(
                                    "{}  {}",
                                    icon::TRASH,
                                    i18n::t("pcs.remove_confirm")
                                ))
                                .clicked()
                            {
                                remove = Some(i);
                                *confirm = None;
                            }
                            if ui.button(i18n::t("pcs.cancel")).clicked() {
                                *confirm = None;
                            }
                        });
                    } else if ui
                        .button(format!("{}  {}", icon::TRASH, i18n::t("pcs.remove")))
                        .clicked()
                    {
                        *confirm = Some(pc.name.clone());
                    }
                    ui.end_row();
                }
            });
        if let Some(i) = remove {
            form.pcs.remove(i);
        }

        ui.add_space(12.0);
        ui.label(
            egui::RichText::new(format!(
                "{}  {}",
                icon::SEARCH,
                i18n::t("pcs.on_this_network")
            ))
            .size(16.0)
            .strong(),
        );
        ui.horizontal(|ui| {
            if ui
                .button(format!("{}  {}", icon::LINK, i18n::t("pcs.let_pair")))
                .clicked()
            {
                self.message = match crate::mickey::ask(&["pairing"]) {
                    Ok(_) => i18n::t("pcs.pairing_open").into(),
                    Err(err) => i18n::tf("pcs.pairing_failed", &[("err", &err)]),
                };
            }
            ui.weak(i18n::t("pcs.needed_here"));
        });
        let known: Vec<&str> = form.pcs.iter().map(|pc| pc.address.as_str()).collect();
        let nearby: Vec<&Nearby> = self
            .nearby
            .iter()
            .filter(|pc| !known.contains(&pc.address.as_str()))
            .collect();
        if nearby.is_empty() {
            ui.weak(i18n::t("pcs.none_nearby"));
        }
        let mut connect = None;
        egui::Grid::new("nearby")
            .num_columns(4)
            .spacing([10.0, 8.0])
            .show(ui, |ui| {
                for pc in nearby {
                    ui.label(
                        egui::RichText::new(format!("{}  {}", icon::LAPTOP, pc.name)).strong(),
                    );
                    ui.weak(&pc.address);
                    ui.weak(pc.fingerprint.get(..9).unwrap_or(&pc.fingerprint));
                    if ui
                        .add(theme::primary(icon::LINK, i18n::t("pcs.connect")))
                        .clicked()
                    {
                        connect = Some((pc.address.clone(), pc.name.clone()));
                    }
                    ui.end_row();
                }
            });
        if let Some((address, name)) = connect {
            self.message = match crate::mickey::ask(&["connect", &address, &name]) {
                Ok(answer) => {
                    let answer = answer.trim().trim_start_matches("mickey: ").to_string();
                    self.pairing = pairing_with(&answer).map(|name| (name, Instant::now()));
                    answer
                }
                Err(err) => i18n::tf("pcs.pairing_failed", &[("err", &err)]),
            };
        }

        ui.add_space(12.0);
        egui::CollapsingHeader::new(i18n::t("pcs.add_by_address")).show(ui, |ui| {
            ui.weak(i18n::t("pcs.add_by_address_hint"));
            if ui
                .button(format!("{}  {}", icon::PLUS, i18n::t("pcs.add_a_pc")))
                .clicked()
            {
                form.pcs.push(PcRow {
                    name: i18n::t("pcs.new_pc").into(),
                    address: format!("192.168.0.2:{}", config::DEFAULT_PORT),
                    hotkey: String::new(),
                    position: None,
                    key: None,
                });
            }
            ui.weak(i18n::t("pcs.then_edit"));
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pairing_pc_is_read_from_mickeys_answer() {
        // What `mickey connect` prints, without its "mickey: ".
        assert_eq!(
            pairing_with("pairing with Desk PC: check that both PCs show the same code").as_deref(),
            Some("Desk PC")
        );
        assert_eq!(
            pairing_with("192.0.2.10:4242 is already one of this PC's PCs"),
            None
        );
    }

    #[test]
    fn nearby_lines_become_pcs() {
        let nearby = parse_nearby("Desk\t192.0.2.64:4242\tVK2A-L4MJ\nnonsense\n");
        assert_eq!(nearby.len(), 1);
        assert_eq!(nearby[0].address, "192.0.2.64:4242");
    }
}
