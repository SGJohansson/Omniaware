//! Fonts, colours, small shared widgets, Swedish date formatting.

use chrono::{Datelike, NaiveDate};
use egui::{Color32, FontFamily, RichText, Stroke, Ui};
use std::sync::Arc;

pub const ACCENT: Color32 = Color32::from_rgb(38, 166, 154);
pub const ACCENT_DIM: Color32 = Color32::from_rgb(22, 64, 61);
pub const BG: Color32 = Color32::from_rgb(23, 24, 27);
pub const BG_SIDE: Color32 = Color32::from_rgb(19, 20, 23);
pub const BG_FIELD: Color32 = Color32::from_rgb(28, 29, 33);
pub const LINE: Color32 = Color32::from_rgb(44, 46, 52);
pub const TEXT: Color32 = Color32::from_rgb(222, 224, 228);
pub const WEAK: Color32 = Color32::from_rgb(128, 132, 140);
pub const DIM: Color32 = Color32::from_rgb(84, 88, 96);
pub const OK: Color32 = Color32::from_rgb(110, 196, 132);
pub const WARN: Color32 = Color32::from_rgb(230, 180, 90);
pub const ERR: Color32 = Color32::from_rgb(232, 110, 100);

pub fn medium() -> FontFamily {
    FontFamily::Name("medium".into())
}

pub fn install(ctx: &egui::Context, size: f32) {
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "jbm".into(),
        Arc::new(egui::FontData::from_static(include_bytes!("../assets/fonts/JetBrainsMono-Regular.ttf"))),
    );
    fonts.font_data.insert(
        "jbm-medium".into(),
        Arc::new(egui::FontData::from_static(include_bytes!("../assets/fonts/JetBrainsMono-Medium.ttf"))),
    );
    for fam in [FontFamily::Proportional, FontFamily::Monospace] {
        fonts.families.entry(fam).or_default().insert(0, "jbm".into());
    }
    let fallback = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    fonts.families.insert(medium(), std::iter::once("jbm-medium".to_string()).chain(fallback).collect());
    ctx.set_fonts(fonts);

    let mut v = egui::Visuals::dark();
    v.panel_fill = BG;
    v.window_fill = BG;
    v.extreme_bg_color = BG_FIELD;
    v.faint_bg_color = BG_FIELD;
    v.override_text_color = Some(TEXT);
    v.hyperlink_color = ACCENT;
    v.warn_fg_color = WARN;
    v.error_fg_color = ERR;
    v.selection.bg_fill = ACCENT_DIM;
    v.selection.stroke = Stroke::new(1.0, ACCENT);
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, LINE);
    v.widgets.inactive.weak_bg_fill = BG_FIELD;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, LINE);
    v.widgets.hovered.weak_bg_fill = Color32::from_rgb(36, 38, 43);
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, Color32::from_rgb(70, 74, 82));
    v.widgets.active.weak_bg_fill = ACCENT_DIM;
    v.widgets.active.bg_stroke = Stroke::new(1.0, ACCENT);
    v.text_cursor.stroke = Stroke::new(2.0, ACCENT);
    ctx.set_visuals_of(egui::Theme::Dark, v);
    ctx.options_mut(|o| o.theme_preference = egui::ThemePreference::Dark);

    ctx.all_styles_mut(|s| {
        use egui::TextStyle::*;
        for (ts, f) in s.text_styles.iter_mut() {
            f.size = match ts {
                Heading => size + 4.0,
                Small => size - 2.5,
                _ => size,
            };
        }
        s.spacing.item_spacing = egui::vec2(8.0, 6.0);
        s.spacing.button_padding = egui::vec2(8.0, 3.0);
    });
}

/// `[Ctrl] [S] namnge` — keycaps followed by a weak label.
pub fn hint(ui: &mut Ui, keys: &[&str], label: &str) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 3.0;
        for k in keys {
            keycap(ui, k);
        }
        ui.add_space(4.0);
        ui.label(RichText::new(label).color(WEAK).size(12.0));
    });
    ui.add_space(9.0);
}

pub fn keycap(ui: &mut Ui, k: &str) {
    egui::Frame::new()
        .fill(BG_FIELD)
        .stroke(Stroke::new(1.0, LINE))
        .corner_radius(4)
        .inner_margin(egui::Margin::symmetric(5, 1))
        .show(ui, |ui| ui.label(RichText::new(k).size(11.5).color(TEXT)));
}

/// "super+alt+KeyV" → ["Win", "Alt", "V"].
pub fn hotkey_caps(spec: &str) -> Vec<String> {
    spec.split('+')
        .map(|p| match p.trim().to_ascii_lowercase().as_str() {
            "super" | "cmd" | "command" | "win" => "Win".into(),
            "alt" | "option" => "Alt".into(),
            "ctrl" | "control" => "Ctrl".into(),
            "shift" => "Shift".into(),
            _ => {
                let p = p.trim();
                p.strip_prefix("Key").or_else(|| p.strip_prefix("Digit")).unwrap_or(p).to_string()
            }
        })
        .collect()
}

const WD_SHORT: [&str; 7] = ["mån", "tis", "ons", "tor", "fre", "lör", "sön"];
const WD_LONG: [&str; 7] = ["Måndag", "Tisdag", "Onsdag", "Torsdag", "Fredag", "Lördag", "Söndag"];
const MONTHS: [&str; 12] = [
    "januari", "februari", "mars", "april", "maj", "juni", "juli", "augusti", "september", "oktober", "november",
    "december",
];

pub fn month_name(m: u32) -> &'static str {
    MONTHS[(m as usize).saturating_sub(1) % 12]
}

/// "sön 4 okt"
pub fn day_short(d: NaiveDate) -> String {
    let m: String = month_name(d.month()).chars().take(3).collect();
    format!("{} {} {}", WD_SHORT[d.weekday().num_days_from_monday() as usize], d.day(), m)
}

/// "Söndag 4 oktober" (+ year if not current)
pub fn day_long(d: NaiveDate, today: NaiveDate) -> String {
    let base = format!(
        "{} {} {}",
        WD_LONG[d.weekday().num_days_from_monday() as usize],
        d.day(),
        month_name(d.month())
    );
    if d.year() == today.year() { base } else { format!("{base} {}", d.year()) }
}
