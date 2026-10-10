//! Fonts, colours, small shared widgets, Swedish date formatting.

use chrono::{Datelike, NaiveDate};
use egui::{Color32, FontFamily, RichText, Stroke, Ui};
use std::sync::Arc;

/// Every colour the UI uses. One palette per theme; `p()` returns the active one.
pub struct Palette {
    /// egui's base visuals (dark or light) under our colours.
    pub dark: bool,
    pub accent: Color32,
    pub accent_dim: Color32,
    /// Primary button and selection-bar fill.
    pub accent_fill: Color32,
    pub bg: Color32,
    pub bg_side: Color32,
    pub bg_field: Color32,
    pub line: Color32,
    pub text: Color32,
    /// Secondary text.
    pub weak: Color32,
    /// Between `weak` and `text`: search snippets, notice details.
    pub soft: Color32,
    /// Shortcut labels, quiet links.
    pub label: Color32,
    /// Brackets, separators, Markdown syntax characters.
    pub faint: Color32,
    pub ok: Color32,
    pub warn: Color32,
    pub err: Color32,
    /// Hover fill for list rows, calendar days and tabs.
    pub hover: Color32,
    /// Hover fill for small painted buttons.
    pub tint: Color32,
    pub hover_stroke: Color32,
    /// Outline of keycaps and checkboxes.
    pub key_stroke: Color32,
    /// Ticked list rows.
    pub row_sel: Color32,
    /// Days outside the month in the calendar.
    pub off_month: Color32,
    pub heading: Color32,
    pub code: Color32,
    /// A 2 px stripe down the left edge of the text panels (VoidFlow).
    pub edge: Option<Color32>,
}

const fn rgb(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

pub const DARK: Palette = Palette {
    dark: true,
    accent: rgb(38, 166, 154),
    accent_dim: rgb(22, 64, 61),
    accent_fill: rgb(26, 44, 43),
    bg: rgb(23, 24, 27),
    bg_side: rgb(19, 20, 23),
    bg_field: rgb(28, 29, 33),
    line: rgb(44, 46, 52),
    text: rgb(222, 224, 228),
    weak: rgb(128, 132, 140),
    soft: rgb(150, 154, 162),
    label: rgb(104, 108, 116),
    faint: rgb(78, 82, 92),
    ok: rgb(110, 196, 132),
    warn: rgb(230, 180, 90),
    err: rgb(232, 110, 100),
    hover: rgb(34, 36, 41),
    tint: rgb(30, 32, 36),
    hover_stroke: rgb(70, 74, 82),
    key_stroke: rgb(52, 55, 62),
    row_sel: rgb(24, 46, 45),
    off_month: rgb(70, 73, 80),
    heading: rgb(240, 242, 245),
    code: rgb(159, 225, 203),
    edge: None,
};

pub const LIGHT: Palette = Palette {
    dark: false,
    accent: rgb(0, 122, 112),
    accent_dim: rgb(206, 233, 229),
    accent_fill: rgb(220, 239, 236),
    bg: rgb(250, 250, 248),
    bg_side: rgb(241, 241, 238),
    bg_field: rgb(255, 255, 255),
    line: rgb(219, 220, 223),
    text: rgb(30, 32, 36),
    weak: rgb(100, 104, 112),
    soft: rgb(84, 88, 96),
    label: rgb(112, 116, 124),
    faint: rgb(162, 166, 174),
    ok: rgb(34, 136, 68),
    warn: rgb(170, 112, 10),
    err: rgb(192, 56, 46),
    hover: rgb(233, 234, 236),
    tint: rgb(236, 237, 239),
    hover_stroke: rgb(176, 180, 188),
    key_stroke: rgb(204, 206, 211),
    row_sel: rgb(214, 236, 232),
    off_month: rgb(188, 191, 197),
    heading: rgb(8, 10, 12),
    code: rgb(0, 112, 92),
    edge: None,
};

/// After voidflow.tech: near-black panels, phosphor-green text, crimson structure lines.
pub const VOIDFLOW: Palette = Palette {
    dark: true,
    accent: rgb(0, 255, 65),     // --rad-green
    accent_dim: rgb(4, 55, 17),  // rad-green at 20 % (--border-dim)
    accent_fill: rgb(4, 30, 11), // rad-green at 10 %
    bg: rgb(10, 10, 11),         // --panel-bg
    bg_side: rgb(5, 5, 5),       // --void-bg
    bg_field: rgb(8, 8, 8),      // .sys-frame
    line: rgb(4, 55, 17),
    text: rgb(0, 230, 59), // rad-green at 90 %, as in .post-content p
    weak: rgb(74, 85, 104), // --text-muted
    soft: rgb(110, 122, 140),
    label: rgb(74, 85, 104),
    faint: rgb(48, 56, 68),
    ok: rgb(0, 255, 65),
    warn: rgb(200, 200, 0),
    err: rgb(215, 10, 83), // --crit-red
    hover: rgb(4, 30, 11),
    tint: rgb(4, 30, 11),
    hover_stroke: rgb(0, 140, 36),
    key_stroke: rgb(4, 55, 17),
    row_sel: rgb(4, 55, 17),
    off_month: rgb(48, 56, 68),
    heading: rgb(255, 255, 255), // --text-main
    code: rgb(255, 255, 255),
    edge: Some(rgb(215, 10, 83)),
};

/// Theme names as written in config.toml.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ThemeChoice {
    Dark,
    Light,
    System,
    VoidFlow,
}

impl ThemeChoice {
    pub const ALL: [ThemeChoice; 4] = [ThemeChoice::Dark, ThemeChoice::Light, ThemeChoice::System, ThemeChoice::VoidFlow];

    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "light" => Self::Light,
            "system" | "auto" => Self::System,
            "voidflow" | "void" => Self::VoidFlow,
            _ => Self::Dark,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Dark => "dark",
            Self::Light => "light",
            Self::System => "system",
            Self::VoidFlow => "voidflow",
        }
    }

    pub fn next(self) -> Self {
        let i = Self::ALL.iter().position(|&t| t == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }

    /// The palette this choice shows; `system_light` is what Windows apps are set to.
    pub fn resolve(self, system_light: bool) -> &'static Palette {
        match self {
            Self::Dark => &DARK,
            Self::Light => &LIGHT,
            Self::System if system_light => &LIGHT,
            Self::System => &DARK,
            Self::VoidFlow => &VOIDFLOW,
        }
    }
}

static ACTIVE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
const PALETTES: [&Palette; 3] = [&DARK, &LIGHT, &VOIDFLOW];

/// The active palette (any thread; the notice window reads it too).
pub fn p() -> &'static Palette {
    PALETTES[ACTIVE.load(std::sync::atomic::Ordering::Relaxed) as usize % PALETTES.len()]
}

/// Makes `pal` the active palette and rebuilds egui's visuals from it. Cheap; returns whether
/// anything changed.
pub fn apply(ctx: &egui::Context, pal: &'static Palette) -> bool {
    let i = PALETTES.iter().position(|q| std::ptr::eq(*q, pal)).unwrap_or(0) as u8;
    if ACTIVE.swap(i, std::sync::atomic::Ordering::Relaxed) == i && ctx.style_of(egui::Theme::Dark).visuals.panel_fill == pal.bg {
        return false;
    }
    set_visuals(ctx, pal);
    true
}

/// Colour as an RGB triple (for GDI drawing).
#[cfg_attr(not(windows), allow(dead_code))]
pub fn rgb3(c: Color32) -> [u8; 3] {
    [c.r(), c.g(), c.b()]
}

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

    set_visuals(ctx, p());

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

fn set_visuals(ctx: &egui::Context, pal: &Palette) {
    let mut v = if pal.dark { egui::Visuals::dark() } else { egui::Visuals::light() };
    v.panel_fill = pal.bg;
    v.window_fill = pal.bg;
    v.extreme_bg_color = pal.bg_field;
    v.faint_bg_color = pal.bg_field;
    v.override_text_color = Some(pal.text);
    v.hyperlink_color = pal.accent;
    v.warn_fg_color = pal.warn;
    v.error_fg_color = pal.err;
    v.selection.bg_fill = pal.accent_dim;
    v.selection.stroke = Stroke::new(1.0, pal.accent);
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, pal.line);
    v.widgets.inactive.weak_bg_fill = pal.bg_field;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, pal.line);
    v.widgets.hovered.weak_bg_fill = pal.hover;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, pal.hover_stroke);
    v.widgets.active.weak_bg_fill = pal.accent_dim;
    v.widgets.active.bg_stroke = Stroke::new(1.0, pal.accent);
    v.text_cursor.stroke = Stroke::new(2.0, pal.accent);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, pal.text);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, pal.text);
    v.widgets.inactive.bg_fill = pal.bg_field;
    v.widgets.hovered.fg_stroke = Stroke::new(1.0, pal.heading);
    v.widgets.active.fg_stroke = Stroke::new(1.0, pal.heading);
    v.window_stroke = Stroke::new(1.0, pal.line);
    v.code_bg_color = pal.bg_side;
    // egui's own light/dark switch must not override ours: both slots get the same visuals.
    ctx.set_visuals_of(egui::Theme::Dark, v.clone());
    ctx.set_visuals_of(egui::Theme::Light, v);
    ctx.options_mut(|o| o.theme_preference = egui::ThemePreference::Dark);

}

pub fn keycap(ui: &mut Ui, k: &str) {
    egui::Frame::new()
        .fill(p().bg_field)
        .stroke(Stroke::new(1.0, p().line))
        .corner_radius(4)
        .inner_margin(egui::Margin::symmetric(5, 1))
        .show(ui, |ui| ui.label(RichText::new(k).size(11.5).color(p().text)));
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
        ui.label(RichText::new(*title).family(medium()).size(12.5).color(p().accent));
        ui.add_space(2.0);
        egui::Grid::new(("keys", *title)).num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
            for (keys, what) in rows.iter() {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 3.0;
                    for k in keys.iter() {
                        keycap(ui, k);
                    }
                });
                ui.label(RichText::new(*what).color(p().weak).size(12.0));
                ui.end_row();
            }
        });
        ui.add_space(10.0);
    }
}

// ---------- buttons, hints, info chips ----------

pub const SITE_URL: &str = "https://voidflow.tech/";

/// Primary action: lowercase label in accent, dim fill, 2 px accent underline.
pub fn primary_button(ui: &mut Ui, text: &str) -> egui::Response {
    let r = ui.add(
        egui::Button::new(RichText::new(text).color(p().accent))
            .fill(p().accent_fill)
            .stroke(Stroke::NONE)
            .corner_radius(4),
    );
    let x = egui::Rangef::new(r.rect.left() + 3.0, r.rect.right() - 3.0);
    ui.painter().hline(x, r.rect.bottom() - 1.0, Stroke::new(2.0, if r.hovered() { p().accent } else { p().accent.gamma_multiply(0.7) }));
    r
}

/// Secondary action: lowercase, grey, thin outline; a red underline on hover.
pub fn quiet_button(ui: &mut Ui, text: &str) -> egui::Response {
    let r = ui.add(
        egui::Button::new(RichText::new(text).color(p().weak))
            .fill(Color32::TRANSPARENT)
            .stroke(Stroke::new(1.0, p().line))
            .corner_radius(4),
    );
    if r.hovered() {
        let x = egui::Rangef::new(r.rect.left() + 3.0, r.rect.right() - 3.0);
        ui.painter().hline(x, r.rect.bottom() - 1.0, Stroke::new(2.0, p().err.gamma_multiply(0.8)));
    }
    r
}

/// Tiny dim keycaps ("enter" "esc") for corner hints.
pub fn key_hint(ui: &mut Ui, keys: &[&str]) {
    ui.spacing_mut().item_spacing.x = 4.0;
    for k in keys {
        egui::Frame::new()
            .fill(p().bg_side)
            .stroke(Stroke::new(1.0, p().key_stroke))
            .corner_radius(3)
            .inner_margin(egui::Margin::symmetric(4, 0))
            .show(ui, |ui| ui.label(RichText::new(*k).size(10.0).color(p().weak)));
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
            ("[ ", p().faint),
            (&w, p().accent),
            ("×", p().text),
            (&h, p().accent),
            (" px", p().weak),
            (" │ ", p().faint),
            (size.0, p().accent),
            (" ", p().weak),
            (size.1, p().weak),
            (" ]", p().faint),
        ],
        font,
    );
    let (rect, _) = ui.allocate_exact_size(g.size(), egui::Sense::hover());
    ui.painter().galley(rect.min, g, p().text);
}

/// voidflow badge + "voidflow.tech ↗" as one clickable unit (painted, so it works in any layout).
pub fn site_link(ui: &mut Ui) -> bool {
    let icon = egui::Image::from_bytes("bytes://voidflow.png", include_bytes!("../assets/brand/voidflow.png"));
    let g = ui.fonts_mut(|f| f.layout_no_wrap("voidflow.tech ↗".into(), egui::FontId::monospace(11.5), p().weak));
    let (isz, gap) = (20.0, 6.0);
    let size = egui::vec2(isz + gap + g.size().x, isz.max(g.size().y));
    let (rect, r) = ui.allocate_exact_size(size, egui::Sense::click());
    let hot = r.hovered();
    icon.paint_at(ui, egui::Rect::from_min_size(rect.min + egui::vec2(0.0, (size.y - isz) / 2.0), egui::vec2(isz, isz)));
    let tp = egui::pos2(rect.left() + isz + gap, rect.center().y - g.size().y / 2.0);
    let gw = g.size().x;
    ui.painter().galley(tp, g, if hot { p().text } else { p().label });
    if hot {
        ui.painter().hline(egui::Rangef::new(tp.x, tp.x + gw), rect.bottom(), Stroke::new(1.0, p().accent.gamma_multiply(0.6)));
    }
    r.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(SITE_URL).clicked()
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

const KEY_PAD: f32 = 4.0;
const KEY_GAP: f32 = 3.0;
const KEY_H: f32 = 17.0;

/// Keycap and label galleys for a key button, laid out with a placeholder colour
/// (the colour is chosen when painting) plus the button's total size.
fn key_button_parts(ui: &Ui, keys: &[&str], label: &str) -> (Vec<Arc<egui::Galley>>, Arc<egui::Galley>, egui::Vec2) {
    let font = egui::FontId::monospace(10.0);
    let lab_font = egui::FontId::monospace(11.0);
    let ph = Color32::PLACEHOLDER;
    let caps: Vec<_> = keys.iter().map(|k| ui.fonts_mut(|f| f.layout_no_wrap(k.to_string(), font.clone(), ph))).collect();
    let lab = ui.fonts_mut(|f| f.layout_no_wrap(label.to_string(), lab_font, ph));
    let caps_w: f32 = caps.iter().map(|g| g.size().x + 2.0 * KEY_PAD + KEY_GAP).sum();
    let lab_w = if label.is_empty() { 0.0 } else { 3.0 + lab.size().x + 4.0 };
    (caps, lab, egui::vec2(caps_w + lab_w, KEY_H))
}

/// Width `key_button` will take for these keys and label.
pub fn key_button_width(ui: &Ui, keys: &[&str], label: &str) -> f32 {
    key_button_parts(ui, keys, label).2.x
}

/// One entry in a `key_row`. Higher `keep` survives longer when the row is too narrow.
pub struct KeyItem<'a> {
    pub keys: &'a [&'a str],
    pub label: &'a str,
    pub keep: u8,
}

/// Which items of a row fit in `avail`: drops the lowest `keep` first (the later one on a tie)
/// until everything, plus `reserved` on the right, fits. Returns indices in display order.
pub fn fit_items(widths: &[f32], keep: &[u8], spacing: f32, reserved: f32, avail: f32) -> Vec<usize> {
    let mut on: Vec<usize> = (0..widths.len()).collect();
    let total = |on: &[usize]| on.iter().map(|&i| widths[i] + spacing).sum::<f32>() + reserved;
    while !on.is_empty() && total(&on) > avail {
        let drop = on.iter().enumerate().min_by_key(|&(pos, &i)| (keep[i], std::cmp::Reverse(pos))).map(|(pos, _)| pos);
        if let Some(pos) = drop {
            on.remove(pos);
        }
    }
    on
}

/// A row of clickable shortcuts that never overlaps: `items` left to right, `right` pinned to
/// the right edge. When the row is too narrow, items are dropped by priority and, as a last
/// resort, the right one loses its label. Returns the index of a clicked item
/// (`items.len()` for `right`).
pub fn key_row(ui: &mut Ui, items: &[KeyItem], right: &KeyItem, right_tip: &str) -> Option<usize> {
    let spacing = ui.spacing().item_spacing.x;
    let avail = ui.available_width();
    let mut right_label = right.label;
    let mut right_w = key_button_width(ui, right.keys, right_label);
    if right_w > avail {
        right_label = "";
        right_w = key_button_width(ui, right.keys, right_label);
    }
    let widths: Vec<f32> = items.iter().map(|it| key_button_width(ui, it.keys, it.label)).collect();
    let keep: Vec<u8> = items.iter().map(|it| it.keep).collect();
    let shown = fit_items(&widths, &keep, spacing, right_w, avail);
    let mut clicked = None;
    for &i in &shown {
        if key_button(ui, items[i].keys, items[i].label).clicked() {
            clicked = Some(i);
        }
    }
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        let mut r = key_button(ui, right.keys, right_label);
        if !right_tip.is_empty() {
            r = r.on_hover_text(right_tip);
        }
        if r.clicked() {
            clicked = Some(items.len());
        }
    });
    clicked
}

/// Clickable shortcut: mini keycaps + a short label, painted as one unit (hover lights it up).
pub fn key_button(ui: &mut Ui, keys: &[&str], label: &str) -> egui::Response {
    let (caps, lab, size) = key_button_parts(ui, keys, label);
    let (pad, gap, h) = (KEY_PAD, KEY_GAP, KEY_H);
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    let hot = resp.hovered();
    let pal = self::p();
    let p = ui.painter();
    if hot {
        p.rect_filled(rect.expand2(egui::vec2(3.0, 1.0)), 4.0, pal.tint);
    }
    let mut x = rect.left();
    for g in caps {
        let r = egui::Rect::from_min_size(egui::pos2(x, rect.top() + 1.0), egui::vec2(g.size().x + 2.0 * pad, h - 2.0));
        p.rect(r, 3.0, pal.bg_side, Stroke::new(1.0, if hot { pal.accent_dim } else { pal.key_stroke }), egui::StrokeKind::Inside);
        p.galley(egui::pos2(r.left() + pad, r.center().y - g.size().y / 2.0), g, if hot { pal.text } else { pal.weak });
        x = r.right() + gap;
    }
    if !label.is_empty() {
        let lab_col = if hot { pal.text } else { pal.label };
        p.galley(egui::pos2(x + 3.0, rect.center().y - lab.size().y / 2.0), lab, lab_col);
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// What the status dot's colours mean (shown in the shortcut overlays).
pub fn status_legend(ui: &mut Ui) {
    ui.label(RichText::new(t::LEGEND_TITLE).family(medium()).size(12.5).color(p().accent));
    ui.add_space(2.0);
    for (c, what) in [
        (p().weak, t::LEGEND_EMPTY),
        (p().ok, t::LEGEND_SAVED),
        (p().warn, t::LEGEND_WAITING),
        (p().err, t::LEGEND_ERROR),
    ] {
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::hover());
            ui.painter().circle_filled(r.center(), 4.0, c);
            ui.label(RichText::new(what).color(p().weak).size(12.0));
        });
    }
    ui.label(RichText::new(t::LEGEND_RING).color(p().faint).size(11.0));
    ui.add_space(10.0);
}

#[cfg(test)]
mod tests {
    use super::fit_items;

    #[test]
    fn everything_fits() {
        assert_eq!(fit_items(&[50.0, 50.0, 50.0], &[1, 2, 3], 10.0, 40.0, 500.0), vec![0, 1, 2]);
    }

    #[test]
    fn drops_lowest_priority_first_keeps_order() {
        // 3×(50+10) + 40 = 220 > 170 → drop keep=1 (index 1) → 160 fits.
        assert_eq!(fit_items(&[50.0, 50.0, 50.0], &[3, 1, 2], 10.0, 40.0, 170.0), vec![0, 2]);
    }

    #[test]
    fn ties_drop_the_later_item() {
        assert_eq!(fit_items(&[50.0, 50.0, 50.0], &[1, 1, 1], 10.0, 0.0, 130.0), vec![0, 1]);
    }

    #[test]
    fn nothing_fits() {
        assert!(fit_items(&[50.0, 50.0], &[1, 2], 10.0, 100.0, 90.0).is_empty());
    }
}
