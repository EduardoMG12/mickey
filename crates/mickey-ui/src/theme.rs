//! How Mickey's windows look: the mascot's colours (a patched-up grey rat
//! with a bone-white belly, ink outlines and purple shadows under its
//! eyes), rounded corners, stitch-like dashed borders, and line icons.

use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, Stroke, Vec2,
};
use std::sync::Arc;

/// The mascot's colours.
pub mod palette {
    use eframe::egui::Color32;
    /// Outlines and the darkest background.
    pub const INK: Color32 = Color32::from_rgb(0x14, 0x14, 0x16);
    /// The window behind everything.
    pub const CHARCOAL: Color32 = Color32::from_rgb(0x1f, 0x1f, 0x23);
    /// Cards and panels: the rat's darker fur.
    pub const GRAPHITE: Color32 = Color32::from_rgb(0x2c, 0x2c, 0x31);
    /// Widgets and stitches: the rat's lighter fur.
    pub const ASH: Color32 = Color32::from_rgb(0x5c, 0x5c, 0x63);
    /// Secondary text.
    pub const FUR: Color32 = Color32::from_rgb(0x9a, 0x9a, 0xa0);
    /// Text: the rat's belly.
    pub const BONE: Color32 = Color32::from_rgb(0xec, 0xeb, 0xe6);
    /// The accent: the shadows under its eyes.
    pub const BRUISE: Color32 = Color32::from_rgb(0x7a, 0x62, 0x8a);
    /// The same as the tray dot: this PC, another PC.
    pub const HERE: Color32 = Color32::from_rgb(0x2e, 0xc4, 0x4a);
    pub const AWAY: Color32 = Color32::from_rgb(0xe5, 0x39, 0x35);
}

/// Line icons from the Phosphor font (see `assets/README.md`).
pub mod icon {
    pub const CHECK: &str = "\u{E182}";
    pub const CLIPBOARD: &str = "\u{E198}";
    pub const DESKTOP: &str = "\u{E560}";
    pub const FILE: &str = "\u{E230}";
    pub const IMAGE: &str = "\u{E2CA}";
    pub const KEYBOARD: &str = "\u{E2D8}";
    pub const LAPTOP: &str = "\u{E586}";
    pub const LINK: &str = "\u{E2E6}";
    pub const MONITOR: &str = "\u{E32E}";
    pub const PALETTE: &str = "\u{E6C8}";
    pub const PLUS: &str = "\u{E3D4}";
    pub const SAVE: &str = "\u{E248}";
    pub const SEARCH: &str = "\u{E30C}";
    pub const SHIELD: &str = "\u{E40C}";
    pub const TRASH: &str = "\u{E4A6}";
    pub const UNDO: &str = "\u{E038}";
    pub const X: &str = "\u{E4F6}";
}

/// Applies the look to a window. Call once, when it is created.
pub fn apply(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        "phosphor".into(),
        Arc::new(FontData::from_static(include_bytes!(
            "../assets/Phosphor.ttf"
        ))),
    );
    // A fallback: icons can sit inside any label.
    if let Some(family) = fonts.families.get_mut(&FontFamily::Proportional) {
        family.push("phosphor".into());
    }
    ctx.set_fonts(fonts);

    ctx.set_theme(egui::Theme::Dark);
    ctx.set_visuals_of(egui::Theme::Dark, visuals());
    ctx.all_styles_mut(|style| {
        style.spacing.item_spacing = Vec2::new(8.0, 7.0);
        style.spacing.button_padding = Vec2::new(12.0, 6.0);
        style.spacing.interact_size.y = 26.0;
    });
}

fn visuals() -> egui::Visuals {
    use palette::*;
    let mut visuals = egui::Visuals::dark();
    let round = CornerRadius::same(8);
    visuals.override_text_color = Some(BONE);
    visuals.panel_fill = CHARCOAL;
    visuals.window_fill = CHARCOAL;
    visuals.window_stroke = Stroke::new(1.0, INK);
    visuals.window_corner_radius = CornerRadius::same(12);
    visuals.extreme_bg_color = INK;
    visuals.faint_bg_color = GRAPHITE;
    visuals.selection.bg_fill = BRUISE;
    visuals.selection.stroke = Stroke::new(1.0, BONE);
    visuals.hyperlink_color = BRUISE;
    // A slider's rail would vanish on a card: the part travelled is filled.
    visuals.slider_trailing_fill = true;
    let widgets = &mut visuals.widgets;
    widgets.noninteractive.bg_fill = GRAPHITE;
    widgets.noninteractive.bg_stroke = Stroke::new(1.0, ASH);
    widgets.noninteractive.fg_stroke = Stroke::new(1.0, BONE);
    for (state, fill) in [
        (&mut widgets.inactive, GRAPHITE),
        (&mut widgets.hovered, ASH),
        (&mut widgets.active, BRUISE),
        (&mut widgets.open, ASH),
    ] {
        state.bg_fill = fill;
        state.weak_bg_fill = fill;
        state.bg_stroke = Stroke::new(1.0, INK);
        state.fg_stroke = Stroke::new(1.5, BONE);
        state.corner_radius = round;
    }
    widgets.noninteractive.corner_radius = round;
    visuals
}

/// A card: the darker fur, rounded, with a dashed line inside its edge
/// like the stitches on the mascot's patches.
pub fn card<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let response = egui::Frame::new()
        .fill(palette::GRAPHITE)
        .stroke(Stroke::new(1.0, palette::INK))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(12)
        .show(ui, add);
    stitch(ui, response.response.rect.shrink(4.0), palette::ASH);
    response.inner
}

/// A dashed rectangle, square-cornered like a seam.
pub fn stitch(ui: &egui::Ui, rect: egui::Rect, colour: Color32) {
    let corners = [
        rect.left_top(),
        rect.right_top(),
        rect.right_bottom(),
        rect.left_bottom(),
        rect.left_top(),
    ];
    ui.painter().extend(egui::Shape::dashed_line(
        &corners,
        Stroke::new(1.0, colour),
        5.0,
        4.0,
    ));
}

/// The mascot, loaded once per window from the PNG built in.
pub fn mascot(ctx: &egui::Context) -> Option<egui::TextureHandle> {
    let mut reader = png::Decoder::new(std::io::Cursor::new(include_bytes!(
        "../assets/mascot-128.png"
    )))
    .read_info()
    .ok()?;
    let mut rgba = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut rgba).ok()?;
    if info.color_type != png::ColorType::Rgba {
        return None;
    }
    let size = [info.width as usize, info.height as usize];
    let image = egui::ColorImage::from_rgba_unmultiplied(size, &rgba[..info.buffer_size()]);
    Some(ctx.load_texture("mascot", image, egui::TextureOptions::LINEAR))
}

/// A tab: an icon and a word, the chosen one filled with the accent.
pub fn tab<T: PartialEq>(ui: &mut egui::Ui, current: &mut T, value: T, icon: &str, text: &str) {
    let chosen = *current == value;
    let label = egui::RichText::new(format!("{icon}  {text}")).color(if chosen {
        palette::BONE
    } else {
        palette::FUR
    });
    let button = egui::Button::new(label)
        .fill(if chosen {
            palette::BRUISE
        } else {
            Color32::TRANSPARENT
        })
        .stroke(Stroke::NONE)
        .corner_radius(CornerRadius::same(8));
    if ui.add(button).clicked() {
        *current = value;
    }
}

/// A hotkey drawn as keys: `AltGr` + `RightShift`, or `RightShift` ×2 for a
/// double tap. `faded`: the default, shown while the setting is empty.
pub fn keycaps(ui: &mut egui::Ui, text: &str, double: bool, faded: bool) {
    let (fill, colour) = if faded {
        (palette::CHARCOAL, palette::FUR)
    } else {
        (palette::ASH, palette::BONE)
    };
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let keys: Vec<&str> = text
            .split('+')
            .map(str::trim)
            .filter(|key| !key.is_empty())
            .collect();
        if keys.is_empty() {
            ui.label(egui::RichText::new("off").color(palette::FUR).italics());
            return;
        }
        for (i, key) in keys.iter().enumerate() {
            if i > 0 {
                ui.label(egui::RichText::new("+").color(palette::FUR));
            }
            egui::Frame::new()
                .fill(fill)
                .stroke(Stroke::new(1.0, palette::INK))
                .corner_radius(CornerRadius::same(5))
                .inner_margin(egui::Margin::symmetric(7, 3))
                .show(ui, |ui| {
                    ui.label(egui::RichText::new(*key).monospace().color(colour));
                });
        }
        if double {
            ui.label(
                egui::RichText::new("×2")
                    .size(16.0)
                    .strong()
                    .color(palette::BONE),
            );
        }
    });
}

/// The main button of a window: filled with the accent.
pub fn primary(icon: &str, text: &str) -> egui::Button<'static> {
    egui::Button::new(egui::RichText::new(format!("{icon}  {text}")).strong()).fill(palette::BRUISE)
}
