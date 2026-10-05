//! Main window: sidebar (search, nav, month), timeline per day, named, trash, search, editor.

use crate::app::App;
use crate::db::Item;
use crate::doc::Doc;
use crate::text as t;
use crate::theme;
use chrono::{Datelike, Duration, Local, NaiveDate, TimeZone};
use egui::{Align, Align2, Color32, FontId, Id, Key, Layout, Margin, Modifiers, RichText, Sense, Ui, ViewportCommand};
use std::collections::HashSet;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum View {
    Timeline,
    Named,
    Trash,
    Search,
}

pub struct MainState {
    pub view: View,
    /// Open entry; drawn on top of `view` until closed.
    pub editor: Option<Doc>,
    pub day: NaiveDate,
    pub month: NaiveDate,
    pub stale: bool,
    items: Vec<Item>,
    dots: HashSet<u32>,
    list: Vec<Item>,
    query: String,
    results: Vec<Item>,
    sel: usize,
    focus_search: bool,
    confirm_purge: Option<i64>,
}

impl MainState {
    pub fn new() -> Self {
        let today = Local::now().date_naive();
        Self {
            view: View::Timeline,
            editor: None,
            day: today,
            month: today.with_day(1).unwrap_or(today),
            stale: true,
            items: Vec::new(),
            dots: HashSet::new(),
            list: Vec::new(),
            query: String::new(),
            results: Vec::new(),
            sel: 0,
            focus_search: false,
            confirm_purge: None,
        }
    }
}

fn local_ms(d: NaiveDate) -> i64 {
    d.and_hms_opt(0, 0, 0)
        .and_then(|t| Local.from_local_datetime(&t).earliest())
        .map(|t| t.timestamp_millis())
        .unwrap_or(0)
}

fn month_end(m: NaiveDate) -> NaiveDate {
    let (y, mo) = if m.month() == 12 { (m.year() + 1, 1) } else { (m.year(), m.month() + 1) };
    NaiveDate::from_ymd_opt(y, mo, 1).unwrap_or(m)
}

fn hhmm(ms: i64) -> String {
    Local.timestamp_millis_opt(ms).single().map(|t| t.format("%H:%M").to_string()).unwrap_or_default()
}

fn date_of(ms: i64) -> Option<NaiveDate> {
    Local.timestamp_millis_opt(ms).single().map(|t| t.date_naive())
}

/// Clickable full-width row painted directly (time | glyph | name chip | preview).
const META_W: f32 = 146.0;

/// "2 images │ 312 kB" (unique files, so copies add nothing) or "48 words", plus "│ 2 links".
fn row_meta(ui: &Ui, dir: &std::path::Path, it: &Item) -> std::sync::Arc<egui::Galley> {
    let (acc, dim, faint) = (theme::ACCENT, theme::WEAK, theme::FAINT);
    if it.blobs.is_empty() {
        let n = it.words.to_string();
        let l = it.links.to_string();
        let mut parts: Vec<(&str, Color32)> = if it.words == 0 {
            vec![("–", faint)]
        } else {
            vec![(&n, acc), (" ", dim), (t::plural(it.words, "word", "words"), dim)]
        };
        if it.links > 0 {
            parts.extend([(" │ ", faint), (&l, acc), (" ", dim), (t::plural(it.links, "link", "links"), dim)]);
        }
        return theme::runs(ui, &parts, 11.0);
    }
    let mut seen = std::collections::HashSet::new();
    let bytes: u64 = it
        .blobs
        .iter()
        .filter(|b| seen.insert(b.as_str()))
        .filter_map(|b| crate::doc::meta(dir, b))
        .map(|m| m.bytes)
        .sum();
    let n = it.blobs.len();
    let (v, u) = crate::doc::fmt_size(bytes);
    let cnt = n.to_string();
    let l = it.links.to_string();
    let mut parts = vec![(cnt.as_str(), acc), (" ", dim), (t::plural(n, "image", "images"), dim), (" │ ", faint), (&v, acc), (" ", dim), (u, dim)];
    if it.links > 0 {
        parts.extend([(" │ ", faint), (&l, acc), (" ", dim), (t::plural(it.links, "link", "links"), dim)]);
    }
    theme::runs(ui, &parts, 11.0)
}

fn row(ui: &mut Ui, dir: &std::path::Path, it: &Item, time: &str, selected: bool) -> egui::Response {
    let time_w = if time.len() > 5 { 92.0 } else { 52.0 };
    let h = 32.0;
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), h), Sense::click());
    let bg = if it.is_event {
        Some(theme::ACCENT_DIM)
    } else if selected || resp.hovered() {
        Some(Color32::from_rgb(34, 36, 41))
    } else {
        None
    };
    if let Some(bg) = bg {
        ui.painter_at(rect).rect_filled(rect, 5.0, bg);
    }
    // Thumbnail of the first image at the right end; text is clipped before it.
    let mut text_right = rect.right() - 8.0;
    if let Some(t) = &it.thumb {
        let tr = egui::Rect::from_min_size(egui::pos2(rect.right() - 52.0, rect.top() + 3.0), egui::vec2(44.0, h - 6.0));
        egui::Image::new(crate::doc::blob_uri(dir, t))
            .fit_to_exact_size(tr.size())
            .maintain_aspect_ratio(true)
            .corner_radius(3)
            .paint_at(ui, tr);
        ui.painter().rect_stroke(tr, 3.0, egui::Stroke::new(1.0, theme::LINE), egui::StrokeKind::Outside);
        text_right = tr.left() - 8.0;
    }
    let clip = egui::Rect::from_min_max(rect.min, egui::pos2(text_right, rect.max.y));
    let p = ui.painter_at(clip);
    let font = FontId::monospace(13.0);
    let small = FontId::monospace(12.0);
    let fg = if it.is_event { theme::ACCENT } else { theme::TEXT };
    let mut x = rect.left() + 10.0;
    let cy = rect.center().y;
    p.text(egui::pos2(x, cy), Align2::LEFT_CENTER, time, small.clone(), theme::WEAK);
    x += time_w;
    // Summary column: images and their size on disk, or the word count for text-only entries.
    let meta = row_meta(ui, dir, it);
    let mw = meta.size().x;
    p.galley(egui::pos2(x, cy - meta.size().y / 2.0), meta, theme::WEAK);
    x += META_W.max(mw + 16.0);
    let glyph = if it.is_event { "◆" } else if it.name.is_some() { "#" } else { "·" };
    p.text(egui::pos2(x, cy), Align2::LEFT_CENTER, glyph, font.clone(), if it.is_event { theme::ACCENT } else { theme::WEAK });
    x += 20.0;
    if let Some(n) = &it.name {
        let g = p.layout_no_wrap(n.clone(), small.clone(), theme::TEXT);
        let chip = egui::Rect::from_min_size(egui::pos2(x, cy - 10.0), egui::vec2(g.size().x + 12.0, 20.0));
        p.rect_filled(chip, 4.0, theme::BG_FIELD);
        p.rect_stroke(chip, 4.0, egui::Stroke::new(1.0, theme::LINE), egui::StrokeKind::Inside);
        p.galley(egui::pos2(x + 6.0, cy - g.size().y / 2.0), g, theme::TEXT);
        x += chip.width() + 8.0;
    }
    let (text, col) = match (it.preview.is_empty(), it.images) {
        (false, _) => (it.preview.clone(), fg),
        (true, 0) => (t::ROW_EMPTY.to_string(), theme::WEAK),
        (true, 1) => (t::ROW_IMAGE.to_string(), theme::WEAK),
        (true, _) => (t::ROW_IMAGES.to_string(), theme::WEAK),
    };
    p.text(egui::pos2(x, cy), Align2::LEFT_CENTER, text, font, col);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn empty_note(ui: &mut Ui, text: &str) {
    ui.add_space(24.0);
    ui.label(RichText::new(text).color(theme::WEAK));
}

impl App {
    fn reload(&mut self) {
        let m = &mut self.main;
        m.stale = false;
        m.confirm_purge = None;
        let from = local_ms(m.day);
        let to = local_ms(m.day + Duration::days(1));
        m.items = self.db.day_items(from, to).unwrap_or_else(|e| {
            crate::log::error(format!("timeline: {e}"));
            Vec::new()
        });
        let (mf, mt) = (local_ms(m.month), local_ms(month_end(m.month)));
        let month = m.month.month();
        m.dots = self
            .db
            .stamps_between(mf, mt)
            .unwrap_or_default()
            .into_iter()
            .filter_map(date_of)
            .filter(|d| d.month() == month)
            .map(|d| d.day())
            .collect();
        m.list = match m.view {
            View::Named => self.db.named().unwrap_or_default(),
            View::Trash => self.db.trash().unwrap_or_default(),
            _ => Vec::new(),
        };
        if m.view == View::Search {
            m.results = self.db.search(&m.query).unwrap_or_default();
            m.sel = m.sel.min(m.results.len().saturating_sub(1));
        }
    }

    fn set_view(&mut self, v: View) {
        self.close_editor();
        self.main.view = v;
        if v == View::Search {
            self.main.focus_search = true;
        }
        self.main.stale = true;
    }

    fn set_day(&mut self, d: NaiveDate) {
        self.close_editor();
        self.main.day = d;
        self.main.month = d.with_day(1).unwrap_or(d);
        self.main.view = View::Timeline;
        self.main.stale = true;
    }

    fn open_entry(&mut self, id: i64) {
        self.close_editor();
        match self.db.get(id) {
            Ok(e) => self.main.editor = Some(Doc::open(e, &format!("ed-{id}"))),
            Err(e) => crate::log::error(format!("open {id}: {e}")),
        }
    }

    fn new_entry(&mut self) {
        self.close_editor();
        self.main.editor = Some(Doc::new(&format!("ed-new-{}", crate::db::now_ms())));
    }

    fn close_editor(&mut self) {
        if let Some(mut d) = self.main.editor.take() {
            d.commit_name(&self.db);
            d.close(&self.db);
            self.main.stale = true;
        }
    }

    fn paste_entry(&mut self, ctx: &egui::Context, id: i64) {
        if let Ok(e) = self.db.get(id) {
            self.paste_back(ctx, e.body);
        }
    }

    pub(crate) fn main_ui(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        if self.main.stale {
            self.reload();
        }

        // ---- global keys ----
        let nothing_focused = ctx.memory(|m| m.focused().is_none());
        let esc_used = self.selection_keys(&ctx);
        if self.lightbox.is_some() || self.dup_prompt.is_some() || esc_used {
            // the lightbox / image selection owns Esc
        } else if ctx.input(|i| i.key_pressed(Key::Escape)) {
            if self.main.editor.is_some() {
                self.close_editor();
            } else if self.main.view != View::Timeline {
                self.set_view(View::Timeline);
            } else {
                return self.hide(&ctx);
            }
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::CTRL, Key::K)) {
            self.set_view(View::Search);
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::CTRL, Key::N)) {
            self.new_entry();
        }
        if let Some(ed) = self.main.editor.as_mut() {
            if ctx.input_mut(|i| i.consume_key(Modifiers::CTRL, Key::E)) {
                ed.preview = !ed.preview;
                ed.focus = !ed.preview;
            }
            if ctx.input_mut(|i| i.consume_key(Modifiers::CTRL, Key::S)) {
                ed.save_now(&self.db);
                self.main.stale = true;
            }
            if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::F2)) {
                ctx.memory_mut(|m| m.request_focus(Id::new("ed_name")));
            }
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::F1)) {
            self.toggle_shortcuts();
        }
        if self.main.editor.is_none() && self.main.view == View::Timeline && nothing_focused {
            let (l, r, t) = ctx.input(|i| (i.key_pressed(Key::ArrowLeft), i.key_pressed(Key::ArrowRight), i.key_pressed(Key::T)));
            if l {
                self.set_day(self.main.day - Duration::days(1));
            } else if r {
                self.set_day(self.main.day + Duration::days(1));
            } else if t {
                self.set_day(Local::now().date_naive());
            }
        }
        if self.main.stale {
            self.reload();
        }

        self.top_bar(ui);
        self.side_bar(ui);
        self.shortcut_panel(ui);
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(theme::BG).inner_margin(Margin::symmetric(20, 14)))
            .show(ui, |ui| {
                if self.main.editor.is_some() {
                    self.editor_view(ui);
                } else {
                    match self.main.view {
                        View::Timeline => self.timeline_view(ui),
                        View::Named => self.named_view(ui),
                        View::Trash => self.trash_view(ui),
                        View::Search => self.search_view(ui),
                    }
                }
            });
        resize_grip(ui);
    }

    fn toggle_shortcuts(&mut self) {
        self.cfg.window.shortcuts = !self.cfg.window.shortcuts;
        crate::config::save(&self.dir, &self.cfg);
    }

    /// Right edge: a slim "?" tab, or the full shortcut list (F1 toggles; remembered in config).
    fn shortcut_panel(&mut self, ui: &mut Ui) {
        let open = self.cfg.window.shortcuts;
        let mut toggle = false;
        egui::Panel::right("m_keys")
            .resizable(false)
            .exact_size(if open { 300.0 } else { 26.0 })
            .frame(egui::Frame::new().fill(theme::BG_SIDE).inner_margin(Margin::symmetric(if open { 12 } else { 0 }, 10)))
            .show(ui, |ui| {
                if open {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(t::SHORTCUTS).family(theme::medium()));
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            toggle = ui.add(egui::Button::new("›").frame(false)).on_hover_text(t::TIP_COLLAPSE).clicked();
                            ui.label(RichText::new("F1").color(theme::WEAK).size(11.5));
                        });
                    });
                    ui.add_space(8.0);
                    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                        theme::shortcut_groups(ui, t::MAIN_KEYS);
                        theme::status_legend(ui);
                    });
                } else {
                    // Whole strip is the tab; vertical label reads bottom-to-top.
                    let rect = ui.max_rect();
                    let resp = ui.interact(rect, Id::new("keys_tab"), Sense::click()).on_hover_text(t::TIP_SHORTCUTS);
                    if resp.hovered() {
                        ui.painter().rect_filled(rect, 0.0, Color32::from_rgb(30, 32, 36));
                    }
                    let g = ui.painter().layout_no_wrap(t::SHORTCUTS_TAB.into(), FontId::monospace(11.5), theme::WEAK);
                    let pos = egui::pos2(rect.center().x - g.size().y / 2.0, rect.top() + 16.0 + g.size().x);
                    ui.painter().add(egui::epaint::TextShape::new(pos, g, theme::WEAK).with_angle(-std::f32::consts::FRAC_PI_2));
                    toggle = resp.clicked();
                }
            });
        if toggle {
            self.toggle_shortcuts();
        }
    }

    fn top_bar(&mut self, ui: &mut Ui) {
        let section = if self.main.editor.is_some() {
            t::ENTRY
        } else {
            match self.main.view {
                View::Timeline => t::TIMELINE,
                View::Named => t::NAMED,
                View::Trash => t::BIN,
                View::Search => t::SEARCH,
            }
        };
        let mut close = false;
        let mut new = false;
        egui::Panel::top("m_top")
            .frame(egui::Frame::new().fill(theme::BG_SIDE).inner_margin(Margin { left: 14, right: 12, top: 11, bottom: 10 }))
            .show(ui, |ui| {
                if ui.interact(ui.max_rect(), Id::new("m_drag"), Sense::drag()).drag_started() {
                    ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
                }
                ui.horizontal(|ui| {
                        // Wordmark: bold name, version right after it (baseline-aligned), then the section.
                        ui.spacing_mut().item_spacing.x = 7.0;
                        ui.label(RichText::new("Omniaware").family(crate::markup::bold_family()).size(19.0).color(theme::ACCENT));
                        ui.with_layout(Layout::left_to_right(Align::Max), |ui| {
                            ui.spacing_mut().item_spacing.x = 7.0;
                            ui.add_space(-2.0);
                            ui.label(RichText::new(concat!("v", env!("CARGO_PKG_VERSION"))).size(11.5).color(theme::FAINT));
                            ui.add_space(10.0);
                            ui.label(RichText::new(format!("/  {section}")).color(theme::WEAK));
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                close = ui.add(egui::Button::new("✕").frame(false)).on_hover_text(t::TIP_CLOSE).clicked();
                                ui.add_space(6.0);
                                new = ui.button(t::BTN_NEW).on_hover_text("Ctrl+N").clicked();
                            });
                        });
                    })
;
            });
        if close {
            let ctx = ui.ctx().clone();
            self.hide(&ctx);
        }
        if new {
            self.new_entry();
        }
    }

    fn side_bar(&mut self, ui: &mut Ui) {
        let mut goto: Option<View> = None;
        let mut pick_day: Option<NaiveDate> = None;
        let mut month_step = 0i32;
        let today = Local::now().date_naive();
        egui::Panel::left("m_side")
            .resizable(false)
            .exact_size(214.0)
            .frame(egui::Frame::new().fill(theme::BG_SIDE).inner_margin(Margin::symmetric(10, 10)))
            .show(ui, |ui| {
                // search pill
                let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 28.0), Sense::click());
                let p = ui.painter_at(rect);
                p.rect(rect, 5.0, theme::BG_FIELD, egui::Stroke::new(1.0, theme::LINE), egui::StrokeKind::Inside);
                p.text(rect.left_center() + egui::vec2(10.0, 0.0), Align2::LEFT_CENTER, t::SEARCH, FontId::monospace(13.0), theme::WEAK);
                p.text(rect.right_center() - egui::vec2(10.0, 0.0), Align2::RIGHT_CENTER, "Ctrl K", FontId::monospace(11.0), theme::WEAK);
                if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    goto = Some(View::Search);
                }
                ui.add_space(10.0);

                let cur = if self.main.editor.is_some() { None } else { Some(self.main.view) };
                for (v, label) in [(View::Timeline, t::NAV_TIMELINE), (View::Named, t::NAV_NAMED), (View::Trash, t::NAV_BIN)] {
                    let on = cur == Some(v);
                    let txt = RichText::new(label).color(if on { theme::TEXT } else { theme::WEAK });
                    let b = egui::Button::new(txt)
                        .fill(if on { Color32::from_rgb(34, 36, 41) } else { Color32::TRANSPARENT })
                        .stroke(egui::Stroke::NONE)
                        .min_size(egui::vec2(ui.available_width(), 26.0));
                    if ui.add(b).clicked() {
                        goto = Some(v);
                    }
                }

                ui.add_space(14.0);
                ui.separator();
                ui.add_space(6.0);

                // month header
                let m = self.main.month;
                ui.horizontal(|ui| {
                    if ui.add(egui::Button::new("‹").frame(false)).clicked() {
                        month_step = -1;
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.add(egui::Button::new("›").frame(false)).clicked() {
                            month_step = 1;
                        }
                        ui.with_layout(Layout::centered_and_justified(egui::Direction::LeftToRight), |ui| {
                            ui.label(RichText::new(format!("{} {}", theme::month_name(m.month()), m.year())).size(12.5));
                        });
                    });
                });

                // grid
                let cell = (ui.available_width() / 7.0).floor();
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    for wd in t::WD_LETTER {
                        let (r, _) = ui.allocate_exact_size(egui::vec2(cell, 18.0), Sense::hover());
                        ui.painter().text(r.center(), Align2::CENTER_CENTER, wd, FontId::monospace(11.0), theme::WEAK);
                    }
                });
                let offset = m.weekday().num_days_from_monday() as i64;
                let first = m - Duration::days(offset);
                for w in 0..6 {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
                        for dcol in 0..7 {
                            let d = first + Duration::days(w * 7 + dcol);
                            let (r, resp) = ui.allocate_exact_size(egui::vec2(cell, cell - 2.0), Sense::click());
                            let p = ui.painter();
                            let inside = d.month() == m.month();
                            if d == self.main.day {
                                p.rect_filled(r.shrink(1.5), 4.0, theme::ACCENT_DIM);
                            } else if resp.hovered() {
                                p.rect_filled(r.shrink(1.5), 4.0, Color32::from_rgb(34, 36, 41));
                            }
                            let col = if d == today {
                                theme::ACCENT
                            } else if inside {
                                theme::TEXT
                            } else {
                                Color32::from_rgb(70, 73, 80)
                            };
                            p.text(r.center() - egui::vec2(0.0, 2.0), Align2::CENTER_CENTER, d.day().to_string(), FontId::monospace(11.5), col);
                            if inside && self.main.dots.contains(&d.day()) {
                                p.circle_filled(egui::pos2(r.center().x, r.bottom() - 4.0), 1.8, theme::ACCENT);
                            }
                            if resp.clicked() {
                                pick_day = Some(d);
                            }
                        }
                    });
                }
                ui.add_space(6.0);
                if ui.add(egui::Button::new(RichText::new(t::BTN_TODAY).size(12.0)).min_size(egui::vec2(ui.available_width(), 24.0))).clicked() {
                    pick_day = Some(today);
                }
                ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
                    if theme::site_link(ui) {
                        theme::open_url(theme::SITE_URL);
                    }
                });
            });
        if month_step != 0 {
            let m = self.main.month;
            self.main.month = if month_step < 0 {
                (m - Duration::days(1)).with_day(1).unwrap_or(m)
            } else {
                month_end(m)
            };
            self.reload();
        }
        if let Some(v) = goto {
            self.set_view(v);
        }
        if let Some(d) = pick_day {
            self.set_day(d);
        }
    }

    fn timeline_view(&mut self, ui: &mut Ui) {
        let dir = self.dir.clone();
        let today = Local::now().date_naive();
        let day = self.main.day;
        let mut step = 0i64;
        ui.horizontal(|ui| {
            ui.label(RichText::new(theme::day_long(day, today)).family(theme::medium()).size(18.0));
            let events = self.main.items.iter().filter(|i| i.is_event).count();
            let meta = t::day_meta(self.main.items.len() - events, events);
            ui.label(RichText::new(meta).color(theme::WEAK).size(12.0));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button("›").clicked() {
                    step = 1;
                }
                if ui.button("‹").clicked() {
                    step = -1;
                }
            });
        });
        ui.add_space(8.0);
        if step != 0 {
            self.set_day(day + Duration::days(step));
            return;
        }
        if self.main.items.is_empty() {
            let caps = theme::hotkey_caps(&self.cfg.hotkeys.capture).join("+");
            empty_note(
                ui,
                &t::empty_day(&caps),

            );
            return;
        }
        let mut open = None;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            for it in &self.main.items {
                if row(ui, &dir, it, &hhmm(it.time), false).clicked() {
                    open = Some(it.id);
                }
            }
        });
        if let Some(id) = open {
            self.open_entry(id);
        }
    }

    fn named_view(&mut self, ui: &mut Ui) {
        let dir = self.dir.clone();
        ui.label(RichText::new(t::NAMED).family(theme::medium()).size(18.0));
        ui.add_space(8.0);
        if self.main.list.is_empty() {
            return empty_note(ui, t::EMPTY_NAMED);
        }
        let mut open = None;
        let mut copy = None;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            for it in &self.main.list {
                ui.horizontal(|ui| {
                    if ui.small_button(t::BTN_COPY).clicked() {
                        copy = Some(it.id);
                    }
                    let d = date_of(it.time).map(theme::day_short).unwrap_or_default();
                    if row(ui, &dir, it, &d, false).clicked() {
                        open = Some(it.id);
                    }
                });
            }
        });
        if let Some(id) = copy
            && let Ok(e) = self.db.get(id)
        {
            ui.ctx().copy_text(e.body);
        }
        if let Some(id) = open {
            self.open_entry(id);
        }
    }

    fn trash_view(&mut self, ui: &mut Ui) {
        let dir = self.dir.clone();
        ui.label(RichText::new(t::BIN).family(theme::medium()).size(18.0));
        ui.add_space(8.0);
        if self.main.list.is_empty() {
            return empty_note(ui, t::EMPTY_BIN);
        }
        let mut restore = None;
        let mut purge = None;
        let confirm = self.main.confirm_purge;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            for it in &self.main.list {
                ui.horizontal(|ui| {
                    if ui.small_button(t::BTN_RESTORE).clicked() {
                        restore = Some(it.id);
                    }
                    let label = if confirm == Some(it.id) { t::BTN_SURE } else { t::BTN_DELETE };
                    if ui.small_button(RichText::new(label).color(theme::ERR)).clicked() {
                        purge = Some(it.id);
                    }
                    let d = date_of(it.time).map(theme::day_short).unwrap_or_default();
                    row(ui, &dir, it, &d, false);
                });
            }
        });
        if let Some(id) = restore {
            if let Err(e) = self.db.restore(id) {
                crate::log::error(format!("restore: {e}"));
            }
            self.main.stale = true;
        }
        if let Some(id) = purge {
            if confirm == Some(id) {
                if let Err(e) = self.db.delete_hard(id) {
                    crate::log::error(format!("delete: {e}"));
                }
                self.main.stale = true;
            } else {
                self.main.confirm_purge = Some(id);
            }
        }
    }

    fn search_view(&mut self, ui: &mut Ui) {
        let dir = self.dir.clone();
        let ctx = ui.ctx().clone();
        let r = ui.add(
            egui::TextEdit::singleline(&mut self.main.query)
                .id(Id::new("m_search"))
                .hint_text(t::SEARCH_HINT)
                .desired_width(f32::INFINITY)
                .margin(Margin::symmetric(10, 6)),
        );
        if std::mem::take(&mut self.main.focus_search) {
            r.request_focus();
        }
        if r.changed() {
            self.main.sel = 0;
            self.main.results = self.db.search(&self.main.query).unwrap_or_default();
        }
        let n = self.main.results.len();
        let (up, down, enter, shift) = ctx.input(|i| {
            (i.key_pressed(Key::ArrowUp), i.key_pressed(Key::ArrowDown), i.key_pressed(Key::Enter), i.modifiers.shift)
        });
        if n > 0 {
            if down {
                self.main.sel = (self.main.sel + 1).min(n - 1);
            }
            if up {
                self.main.sel = self.main.sel.saturating_sub(1);
            }
        }
        ui.add_space(8.0);
        if n == 0 {
            if !self.main.query.trim().is_empty() {
                empty_note(ui, t::NO_MATCHES);
            }
            return;
        }
        let sel = self.main.sel;
        let mut open = None;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            for (i, it) in self.main.results.iter().enumerate() {
                let d = date_of(it.time).map(theme::day_short).unwrap_or_default();
                let resp = row(ui, &dir, it, &d, i == sel);
                if i == sel && (up || down) {
                    resp.scroll_to_me(None);
                }
                if resp.clicked() {
                    open = Some(it.id);
                }
            }
        });
        let picked = self.main.results.get(sel).map(|i| i.id);
        if enter && let Some(id) = picked {
            if shift {
                return self.paste_entry(&ctx, id);
            }
            open = Some(id);
        }
        if let Some(id) = open {
            self.open_entry(id);
        }
    }

    fn editor_view(&mut self, ui: &mut Ui) {
        let today = Local::now().date_naive();
        let dir = self.dir.clone();
        let mut back = false;
        let mut discard = false;
        let mut commit = false;
        let mut img_act = None;
        let Some(ed) = self.main.editor.as_mut() else { return };
        // Row 1: navigation + name on the left, actions on the right. Row 2: timestamp.
        // Buttons are placed first (right-to-left) so a narrow window shrinks the name field, not them.
        ui.horizontal(|ui| {
            back = ui.button(t::BTN_BACK).clicked();
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                discard = ui.button(RichText::new(t::BTN_DISCARD).color(theme::ERR)).clicked();
                let lbl = if ed.preview { t::BTN_EDIT } else { t::BTN_PREVIEW };
                if ui.button(lbl).on_hover_text("Ctrl+E").clicked() {
                    ed.preview = !ed.preview;
                    ed.focus = !ed.preview;
                }
                ui.add_space(4.0);
                ed.indicator(ui);
                ui.add_space(8.0);
                let w = (ui.available_width() - 4.0).clamp(80.0, 260.0);
                let r = ui.add(
                    egui::TextEdit::singleline(&mut ed.name)
                        .id(Id::new("ed_name"))
                        .hint_text(t::NAME_HINT_MAIN)
                        .desired_width(w)
                        .margin(Margin::symmetric(8, 4)),
                );
                if r.changed() {
                    ed.name_takeover = None;
                    ed.name_msg = None;
                }
                if r.lost_focus() {
                    commit = true;
                }
            });
        });
        let when = date_of(ed.created)
            .map(|d| format!("{} {}", theme::day_long(d, today), hhmm(ed.created)))
            .unwrap_or_default();
        ui.label(RichText::new(when).color(theme::WEAK).size(12.0));
        if let Some(m) = &ed.name_msg {
            ui.label(RichText::new(m).color(theme::WARN).size(12.0));
        }
        ui.add_space(6.0);
        if ed.preview {
            ed.preview_ui(ui, &mut self.md_cache, &dir);
        } else {
            // Images sit above the text. An image-only entry shows its first image large,
            // with the text field below acting as a caption.
            let image_only = ed.body.trim().is_empty() && !ed.images.is_empty();
            if image_only {
                let h = (ui.available_height() * 0.62).max(160.0);
                egui::Panel::top("ed_hero")
                    .frame(egui::Frame::new().inner_margin(Margin::symmetric(0, 4)))
                    .show_separator_line(false)
                    .exact_size(h + 8.0)
                    .show(ui, |ui| {
                        let max = egui::vec2(ui.available_width() * 0.7, h - 10.0);
                        img_act = ed.hero(ui, &dir, max);
                    });
            } else if !ed.images.is_empty() {
                egui::Panel::top("ed_gallery")
                    .frame(egui::Frame::new().inner_margin(Margin::symmetric(0, 4)))
                    .show_separator_line(false)
                    .show(ui, |ui| ed.thumbs(ui, &dir, crate::doc::THUMB_LARGE))
                    .inner
                    .map(|a| img_act = Some(a));
            }
            let hint = if image_only { t::CAPTION_HINT } else { t::EDITOR_HINT_MAIN };
            egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| {
                ed.editor(ui, hint);
            });
        }
        if let Some(a) = img_act {
            self.image_action(a);
            return;
        }
        if commit {
            // Enter applies the name and returns to the text; Tab / a click go where the user went.
            if ed.commit_name(&self.db) && ui.input(|i| i.key_pressed(Key::Enter)) {
                ed.focus = true;
            }
            self.main.stale = true;
        }
        if discard {
            if let Some(mut d) = self.main.editor.take() {
                d.discard(&self.db);
            }
            self.main.stale = true;
        } else if back {
            self.close_editor();
        }
    }
}

/// Bottom-right resize handle for the undecorated window.
fn resize_grip(ui: &mut Ui) {
    let screen = ui.ctx().content_rect();
    let r = egui::Rect::from_min_size(screen.max - egui::vec2(16.0, 16.0), egui::vec2(16.0, 16.0));
    let resp = ui.interact(r, Id::new("resize_grip"), Sense::drag()).on_hover_cursor(egui::CursorIcon::ResizeSouthEast);
    let p = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Foreground, Id::new("grip_paint")));
    let st = egui::Stroke::new(1.0, if resp.hovered() { theme::ACCENT } else { theme::WEAK });
    for k in [4.0, 8.0, 12.0] {
        p.line_segment([egui::pos2(r.right() - k - 2.0, r.bottom() - 2.0), egui::pos2(r.right() - 2.0, r.bottom() - k - 2.0)], st);
    }
    if resp.drag_started() {
        ui.ctx().send_viewport_cmd(ViewportCommand::BeginResize(egui::ResizeDirection::SouthEast));
    }
}
