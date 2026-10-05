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
        "jbm-bold".into(),
        Arc::new(egui::FontData::from_static(include_bytes!("../assets/fonts/JetBrainsMono-Bold.ttf"))),
    );
    fonts.font_data.insert(
        "jbm-medium".into(),
        Arc::new(egui::FontData::from_static(include_bytes!("../assets/fonts/JetBrainsMono-Medium.ttf"))),
    );
    for fam in [FontFamily::Proportional, FontFamily::Monospace] {
        fonts.families.entry(fam).or_default().insert(0, "jbm".into());
    }
    let fallback = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    fonts.families.insert(medium(), std::iter::once("jbm-medium".to_string()).chain(fallback.clone()).collect());
    fonts.families.insert(crate::markup::bold_family(), std::iter::once("jbm-bold".to_string()).chain(fallback).collect());
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

use crate::text::{self as t, Group};

pub fn month_name(m: u32) -> &'static str {
    t::MONTHS[(m as usize).saturating_sub(1) % 12]
}

/// "Mon 5 Oct"
pub fn day_short(d: NaiveDate) -> String {
    let m: String = month_name(d.month()).chars().take(3).collect();
    format!("{} {} {}", t::WD_SHORT[d.weekday().num_days_from_monday() as usize], d.day(), m)
}

/// "Monday 5 October" (+ year if not the current one)
pub fn day_long(d: NaiveDate, today: NaiveDate) -> String {
    let base = format!("{} {} {}", t::WD_LONG[d.weekday().num_days_from_monday() as usize], d.day(), month_name(d.month()));
    if d.year() == today.year() { base } else { format!("{base} {}", d.year()) }
}

/// Grouped shortcut list (keycaps column + description column).
pub fn shortcut_groups(ui: &mut Ui, groups: &[Group]) {
    for (title, rows) in groups {
        ui.label(RichText::new(*title).family(medium()).size(12.5).color(ACCENT));
        ui.add_space(2.0);
        egui::Grid::new(("keys", *title)).num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
            for (keys, what) in rows.iter() {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 3.0;
                    for k in keys.iter() {
                        keycap(ui, k);
                    }
                });
                ui.label(RichText::new(*what).color(WEAK).size(12.0));
                ui.end_row();
            }
        });
        ui.add_space(10.0);
    }
}

// ---------- buttons, hints, info chips ----------

/// Brackets and separators in data chips.
pub const FAINT: Color32 = Color32::from_rgb(78, 82, 92);
pub const SITE_URL: &str = "https://voidflow.tech/";

/// Primary action: lowercase label in accent, dim fill, 2 px accent underline.
pub fn primary_button(ui: &mut Ui, text: &str) -> egui::Response {
    let r = ui.add(
        egui::Button::new(RichText::new(text).color(ACCENT))
            .fill(Color32::from_rgb(26, 44, 43))
            .stroke(Stroke::NONE)
            .corner_radius(4),
    );
    let x = egui::Rangef::new(r.rect.left() + 3.0, r.rect.right() - 3.0);
    ui.painter().hline(x, r.rect.bottom() - 1.0, Stroke::new(2.0, if r.hovered() { ACCENT } else { ACCENT.gamma_multiply(0.7) }));
    r
}

/// Secondary action: lowercase, grey, thin outline; a red underline on hover.
pub fn quiet_button(ui: &mut Ui, text: &str) -> egui::Response {
    let r = ui.add(
        egui::Button::new(RichText::new(text).color(WEAK))
            .fill(Color32::TRANSPARENT)
            .stroke(Stroke::new(1.0, LINE))
            .corner_radius(4),
    );
    if r.hovered() {
        let x = egui::Rangef::new(r.rect.left() + 3.0, r.rect.right() - 3.0);
        ui.painter().hline(x, r.rect.bottom() - 1.0, Stroke::new(2.0, ERR.gamma_multiply(0.8)));
    }
    r
}

/// Tiny dim keycaps ("enter" "esc") for corner hints.
pub fn key_hint(ui: &mut Ui, keys: &[&str]) {
    ui.spacing_mut().item_spacing.x = 4.0;
    for k in keys {
        egui::Frame::new()
            .fill(BG_SIDE)
            .stroke(Stroke::new(1.0, Color32::from_rgb(52, 55, 62)))
            .corner_radius(3)
            .inner_margin(egui::Margin::symmetric(4, 0))
            .show(ui, |ui| ui.label(RichText::new(*k).size(10.0).color(WEAK)));
    }
}

/// Multi-colour monospace text as one galley.
pub fn runs(ui: &Ui, parts: &[(&str, Color32)], size: f32) -> Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::default();
    for (t, c) in parts {
        job.append(t, 0.0, egui::TextFormat::simple(egui::FontId::monospace(size), *c));
    }
    ui.fonts_mut(|f| f.layout_job(job))
}

/// "[ 265×114 px │ 5.8 kB ]": numbers in accent, units dim, brackets faint.
pub fn info_chip(ui: &mut Ui, w: u32, h: u32, size: (&str, &str), font: f32) {
    let (w, h) = (w.to_string(), h.to_string());
    let g = runs(
        ui,
        &[
            ("[ ", FAINT),
            (&w, ACCENT),
            ("×", TEXT),
            (&h, ACCENT),
            (" px", WEAK),
            (" │ ", FAINT),
            (size.0, ACCENT),
            (" ", WEAK),
            (size.1, WEAK),
            (" ]", FAINT),
        ],
        font,
    );
    let (rect, _) = ui.allocate_exact_size(g.size(), egui::Sense::hover());
    ui.painter().galley(rect.min, g, TEXT);
}

/// voidflow badge + "voidflow.tech ↗"; the whole row is the link. Returns true when clicked.
pub fn site_link(ui: &mut Ui) -> bool {
    let icon = egui::Image::from_bytes("bytes://voidflow.png", include_bytes!("../assets/brand/voidflow.png"))
        .fit_to_exact_size(egui::vec2(20.0, 20.0));
    let r = ui
        .horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.add(icon);
            ui.label(RichText::new("voidflow.tech ↗").size(11.0).color(FAINT));
        })
        .response
        .interact(egui::Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(SITE_URL);
    if r.hovered() {
        let x = egui::Rangef::new(r.rect.left() + 26.0, r.rect.right());
        ui.painter().hline(x, r.rect.bottom(), Stroke::new(1.0, ACCENT.gamma_multiply(0.6)));
    }
    r.clicked()
}

/// Opens a URL in the default browser.
pub fn open_url(url: &str) {
    let r = if cfg!(windows) {
        std::process::Command::new("explorer.exe").arg(url).spawn()
    } else {
        std::process::Command::new("xdg-open").arg(url).spawn()
    };
    if let Err(e) = r {
        crate::log::error(format!("open {url}: {e}"));
    }
}

/// Clickable shortcut: mini keycaps + a short label, painted as one unit (hover lights it up).
pub fn key_button(ui: &mut Ui, keys: &[&str], label: &str) -> egui::Response {
    let font = egui::FontId::monospace(10.0);
    let lab_font = egui::FontId::monospace(11.0);
    let caps: Vec<_> = keys.iter().map(|k| ui.fonts_mut(|f| f.layout_no_wrap(k.to_string(), font.clone(), WEAK))).collect();
    let lab = ui.fonts_mut(|f| f.layout_no_wrap(label.to_string(), lab_font.clone(), WEAK));
    let (pad, gap, h) = (4.0, 3.0, 17.0);
    let caps_w: f32 = caps.iter().map(|g| g.size().x + 2.0 * pad + gap).sum();
    let size = egui::vec2(caps_w + 3.0 + lab.size().x + 4.0, h);
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    let hot = resp.hovered();
    let p = ui.painter();
    if hot {
        p.rect_filled(rect.expand2(egui::vec2(3.0, 1.0)), 4.0, Color32::from_rgb(30, 32, 36));
    }
    let mut x = rect.left();
    for g in caps {
        let r = egui::Rect::from_min_size(egui::pos2(x, rect.top() + 1.0), egui::vec2(g.size().x + 2.0 * pad, h - 2.0));
        p.rect(r, 3.0, BG_SIDE, Stroke::new(1.0, if hot { ACCENT_DIM } else { Color32::from_rgb(52, 55, 62) }), egui::StrokeKind::Inside);
        p.galley(egui::pos2(r.left() + pad, r.center().y - g.size().y / 2.0), g, if hot { TEXT } else { WEAK });
        x = r.right() + gap;
    }
    let lab_col = if hot { TEXT } else { Color32::from_rgb(104, 108, 116) };
    p.galley(egui::pos2(x + 3.0, rect.center().y - lab.size().y / 2.0), lab, lab_col);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// What the status dot's colours mean (shown in the shortcut overlays).
pub fn status_legend(ui: &mut Ui) {
    ui.label(RichText::new(t::LEGEND_TITLE).family(medium()).size(12.5).color(ACCENT));
    ui.add_space(2.0);
    for (c, what) in [
        (WEAK, t::LEGEND_EMPTY),
        (OK, t::LEGEND_SAVED),
        (WARN, t::LEGEND_WAITING),
        (ERR, t::LEGEND_ERROR),
    ] {
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::hover());
            ui.painter().circle_filled(r.center(), 4.0, c);
            ui.label(RichText::new(what).color(WEAK).size(12.0));
        });
    }
    ui.label(RichText::new(t::LEGEND_RING).color(FAINT).size(11.0));
    ui.add_space(10.0);
}
