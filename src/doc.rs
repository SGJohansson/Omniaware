//! One open entry being edited (capture popup or main-window editor): autosave, naming, images.

use crate::db::{self, Db, Entry, Img};
use crate::{blob, log, theme};
use egui::text::{CCursor, CCursorRange};
use egui::{Id, TextEdit, Ui};
use std::collections::HashMap;
use std::path::Path;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

const DEBOUNCE: Duration = Duration::from_millis(300);
/// "Sparad" toast: fade in, hold, fade out.
const TOAST_IN: f32 = 0.15;
const TOAST_HOLD: f32 = 2.5;
const TOAST_OUT: f32 = 0.6;
pub const THUMB: egui::Vec2 = egui::vec2(160.0, 96.0);
pub const THUMB_LARGE: egui::Vec2 = egui::vec2(200.0, 120.0);
/// Autosave pulse on the status dot.
const PULSE: f32 = 0.7;

pub enum Status {
    Idle,
    /// Opened, nothing changed yet.
    Clean,
    Saved(String),
    Error(String),
}

pub struct Doc {
    pub id: Option<i64>,
    pub body: String,
    /// Attached images, in order (the same blob may appear more than once).
    pub images: Vec<Img>,
    /// Selected attachment ids (click / Ctrl+click); Delete removes them.
    pub selected: Vec<i64>,
    /// Name field contents ("" = unnamed).
    pub name: String,
    pub saved_name: Option<String>,
    pub created: i64,
    #[allow(dead_code)] // used by Ctrl+D events (step 3)
    pub starts: Option<i64>,
    /// Created in this session (an emptied fresh doc is dropped, not trashed).
    fresh: bool,
    dirty: bool,
    last_edit: Instant,
    pub status: Status,
    editor_id: Id,
    pub focus: bool,
    pub cursor_to_end: bool,
    pub preview: bool,
    pub name_takeover: Option<String>,
    pub name_msg: Option<String>,
    /// (first save of the current toast, latest save) — the "Sparad" toast, only after Ctrl+S.
    toast: Option<(Instant, Instant)>,
    toast_text: String,
    /// Last autosave — drives a short pulse on the status dot (no text).
    pulse: Option<Instant>,
}

/// Result of adding an image.
pub enum Attach {
    Added,
    /// Same picture already attached as #n (1-based); needs confirmation to add a copy.
    Duplicate(usize),
    Failed,
}

impl Doc {
    pub fn new(editor: &str) -> Self {
        Self {
            id: None,
            body: String::new(),
            images: Vec::new(),
            selected: Vec::new(),
            name: String::new(),
            saved_name: None,
            created: db::now_ms(),
            starts: None,
            fresh: true,
            dirty: false,
            last_edit: Instant::now(),
            status: Status::Idle,
            editor_id: Id::new(editor),
            focus: true,
            cursor_to_end: true,
            preview: false,
            name_takeover: None,
            name_msg: None,
            toast: None,
            toast_text: String::new(),
            pulse: None,
        }
    }

    pub fn open(e: Entry, editor: &str) -> Self {
        Self {
            id: Some(e.id),
            body: e.body,
            images: e.images,
            name: e.name.clone().unwrap_or_default(),
            saved_name: e.name,
            created: e.created,
            starts: e.starts,
            fresh: false,
            status: Status::Clean,
            ..Self::new(editor)
        }
    }

    pub fn has_focus(&self, ctx: &egui::Context) -> bool {
        ctx.memory(|m| m.has_focus(self.editor_id))
    }

    /// Text or images — an entry worth keeping.
    pub fn has_content(&self) -> bool {
        !self.body.trim().is_empty() || !self.images.is_empty()
    }

    pub fn mark_dirty(&mut self) {
        self.dirty = true;
        self.last_edit = Instant::now();
    }

    /// Debounced autosave; call every frame/logic pass.
    pub fn tick(&mut self, db: &Db, ctx: &egui::Context) {
        if self.dirty {
            let el = self.last_edit.elapsed();
            if el >= DEBOUNCE {
                self.flush(db);
            } else {
                ctx.request_repaint_after(DEBOUNCE - el);
            }
        }
    }

    pub fn flush(&mut self, db: &Db) {
        if !self.dirty {
            return;
        }
        let res = match self.id {
            None if !self.has_content() => Ok(()),
            None => db.insert(&self.body).map(|id| self.id = Some(id)),
            Some(id) => db.update_body(id, &self.body),
        };
        match res {
            Ok(()) => {
                self.dirty = false;
                if self.id.is_some() {
                    self.mark_saved();
                }
            }
            Err(e) => self.fail(format!("sparning: {e}")),
        }
    }

    pub fn fail(&mut self, msg: String) {
        log::error(&msg);
        self.status = Status::Error(msg);
    }

    /// Flush + revision snapshot, keep editing.
    pub fn checkpoint(&mut self, db: &Db) {
        self.flush(db);
        if let Some(id) = self.id
            && self.has_content()
            && let Err(e) = db.add_revision(id, &self.body)
        {
            self.fail(format!("version: {e}"));
        }
    }

    /// Leaving the doc: snapshot, or drop/trash it if emptied.
    pub fn close(&mut self, db: &Db) {
        self.flush(db);
        let Some(id) = self.id else { return };
        let r = if self.has_content() {
            db.add_revision(id, &self.body)
        } else if self.fresh {
            db.delete_hard(id)
        } else {
            db.delete_soft(id)
        };
        if let Err(e) = r {
            self.fail(format!("stängning: {e}"));
        }
    }

    pub fn discard(&mut self, db: &Db) {
        self.flush(db);
        if let Some(id) = self.id
            && let Err(e) = db.delete_soft(id)
        {
            self.fail(format!("kasta: {e}"));
        }
    }

    /// Applies the name field. Returns true when done; on conflict sets a message and
    /// requires a second call with the same name to move the name here.
    pub fn commit_name(&mut self, db: &Db) -> bool {
        let name = self.name.trim().to_string();
        if Some(&name) == self.saved_name.as_ref() || (name.is_empty() && self.saved_name.is_none()) {
            return true;
        }
        if !self.has_content() {
            self.name_msg = Some("Tomt inlägg, inget att namnge.".into());
            return false;
        }
        self.dirty = true;
        self.flush(db);
        let Some(id) = self.id else { return false };
        if name.is_empty() {
            return match db.clear_name(id) {
                Ok(()) => {
                    self.saved_name = None;
                    true
                }
                Err(e) => {
                    self.fail(format!("namn: {e}"));
                    false
                }
            };
        }
        match db.name_owner(&name) {
            Ok(Some(owner)) if owner != id && self.name_takeover.as_deref() != Some(name.as_str()) => {
                self.name_msg = Some(format!(
                    "”{name}” finns redan. Enter igen flyttar namnet hit, det gamla inlägget behålls utan namn."
                ));
                self.name_takeover = Some(name);
                return false;
            }
            Err(e) => {
                self.fail(format!("namn: {e}"));
                return false;
            }
            _ => {}
        }
        match db.set_name(id, &name) {
            Ok(()) => {
                self.saved_name = Some(name);
                self.name_msg = None;
                self.name_takeover = None;
                true
            }
            Err(e) => {
                self.fail(format!("namn: {e}"));
                false
            }
        }
    }

    /// Ctrl+V with an image-only clipboard (egui only forwards text pastes).
    /// Returns the stored blob and what happened (a duplicate waits for confirmation).
    pub fn paste_image(&mut self, db: &Db, dir: &Path) -> Option<(String, Attach)> {
        let mut cb = arboard::Clipboard::new().ok()?;
        if cb.get_text().is_ok_and(|t| !t.is_empty()) {
            return None;
        }
        let img = cb.get_image().ok()?;
        match store_image(db, dir, &img) {
            Ok(r) => {
                let res = self.attach(db, r.clone(), false);
                Some((r, res))
            }
            Err(e) => {
                self.fail(e);
                None
            }
        }
    }

    /// Adds an image; the entry is created first if needed (an image alone is content).
    /// Unless `copy` is set, a picture that is already attached is reported instead of added.
    pub fn attach(&mut self, db: &Db, blob: String, copy: bool) -> Attach {
        if !copy && let Some(i) = self.images.iter().position(|x| x.blob == blob) {
            return Attach::Duplicate(i + 1);
        }
        if self.id.is_none() {
            match db.insert(&self.body) {
                Ok(id) => {
                    self.id = Some(id);
                    self.dirty = false;
                }
                Err(e) => {
                    self.fail(format!("sparning: {e}"));
                    return Attach::Failed;
                }
            }
        }
        let Some(id) = self.id else { return Attach::Failed };
        match db.add_attachment(id, &blob) {
            Ok(aid) => {
                self.images.push(Img { id: aid, blob });
                self.mark_saved();
                Attach::Added
            }
            Err(e) => {
                self.fail(format!("bild: {e}"));
                Attach::Failed
            }
        }
    }

    /// Removes an attachment (the file stays in the blob store).
    pub fn detach(&mut self, db: &Db, aid: i64) {
        self.images.retain(|x| x.id != aid);
        self.selected.retain(|x| *x != aid);
        match db.remove_attachment(aid) {
            Ok(()) => self.mark_saved(),
            Err(e) => self.fail(format!("bild: {e}")),
        }
    }

    /// Autosave bookkeeping: status + dot pulse, no text.
    fn mark_saved(&mut self) {
        self.status = Status::Saved(chrono::Local::now().format("%H:%M:%S").to_string());
        self.pulse = Some(Instant::now());
    }

    /// Ctrl+S: write now, store a version and say so.
    pub fn save_now(&mut self, db: &Db) {
        self.dirty = true;
        self.flush(db);
        let Some(id) = self.id else {
            self.show_toast("Inget att spara än".into());
            return;
        };
        match db.add_revision(id, &self.body).and_then(|()| db.revision_count(id)) {
            Ok(n) => self.show_toast(format!("✓ Sparad · version {n}")),
            Err(e) => self.fail(format!("version: {e}")),
        }
    }

    fn show_toast(&mut self, text: String) {
        let now = Instant::now();
        self.toast_text = text;
        self.toast = Some((now, now));
    }

    fn toast_alpha(&self, now: Instant) -> f32 {
        let Some((start, last)) = self.toast else { return 0.0 };
        let fade_in = (now.duration_since(start).as_secs_f32() / TOAST_IN).min(1.0);
        let since = now.duration_since(last).as_secs_f32();
        let fade_out = if since <= TOAST_HOLD { 1.0 } else { 1.0 - (since - TOAST_HOLD) / TOAST_OUT };
        (fade_in * fade_out).clamp(0.0, 1.0)
    }

    /// Permanent state: a small dot (grey = empty, amber = unsaved, green = on disk, red = error).
    /// Each autosave sends a short ring out from the dot.
    pub fn indicator(&self, ui: &mut Ui) {
        let (col, tip) = match &self.status {
            Status::Error(e) => (theme::ERR, e.clone()),
            _ if self.dirty => (theme::WARN, "Osparade ändringar – sparas automatiskt strax".to_string()),
            Status::Saved(t) => (theme::OK, format!("Allt ligger på disk (senast {t}). Ctrl+S sparar en version.")),
            Status::Clean => (theme::OK, "Allt ligger på disk. Ctrl+S sparar en version.".to_string()),
            Status::Idle => (theme::WEAK, "Inget att spara än".to_string()),
        };
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::hover());
        let c = rect.center();
        if let Some(t0) = self.pulse {
            let t = t0.elapsed().as_secs_f32() / PULSE;
            if t < 1.0 {
                ui.painter().circle_stroke(c, 4.0 + 5.0 * t, egui::Stroke::new(1.5, col.gamma_multiply(1.0 - t)));
                ui.ctx().request_repaint();
            }
        }
        ui.painter().circle_filled(c, 4.0, col);
        resp.on_hover_text(tip);
        if let Status::Error(_) = self.status {
            ui.label(egui::RichText::new("fel vid sparning").color(theme::ERR).size(12.0));
        }
    }

    /// Transient pill (Ctrl+S feedback) over the bottom-right corner of `area` (no layout impact).
    fn paint_toast(&self, ctx: &egui::Context, area: egui::Rect) {
        let a = self.toast_alpha(Instant::now());
        if a <= 0.0 {
            return;
        }
        ctx.request_repaint();
        let p = ctx.layer_painter(egui::LayerId::new(egui::Order::Middle, self.editor_id.with("toast")));
        let g = p.layout_no_wrap(self.toast_text.clone(), egui::FontId::monospace(12.0), theme::OK.gamma_multiply(a));
        let size = g.size() + egui::vec2(16.0, 8.0);
        let rect = egui::Rect::from_min_size(area.right_bottom() - size - egui::vec2(10.0, 10.0), size);
        p.rect(
            rect,
            5.0,
            theme::BG.gamma_multiply(a),
            egui::Stroke::new(1.0, theme::OK.gamma_multiply(0.5 * a)),
            egui::StrokeKind::Inside,
        );
        p.galley(rect.min + egui::vec2(8.0, 4.0), g, theme::OK);
    }

    /// Framed, full-size text editor.
    pub fn editor(&mut self, ui: &mut Ui, hint: &str) {
        let ctx = ui.ctx().clone();
        let framed = egui::Frame::new()
            .fill(theme::BG_FIELD)
            .stroke(egui::Stroke::new(1.0, theme::LINE))
            .corner_radius(6)
            .inner_margin(egui::Margin::same(10))
            .show(ui, |ui| {
                let avail = ui.available_size();
                let resp = egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.add_sized(
                            avail,
                            TextEdit::multiline(&mut self.body)
                                .id(self.editor_id)
                                .font(egui::TextStyle::Monospace)
                                .frame(egui::Frame::NONE)
                                .desired_width(f32::INFINITY)
                                .lock_focus(true)
                                .hint_text(hint),
                        )
                    })
                    .inner;
                if resp.changed() {
                    self.mark_dirty();
                }
                if self.focus {
                    resp.request_focus();
                    self.focus = false;
                }
            });
        self.paint_toast(&ctx, framed.response.rect);
        if self.cursor_to_end {
            set_cursor(&ctx, self.editor_id, self.body.chars().count());
            self.cursor_to_end = false;
        }
    }

    /// Rendered markdown, `blob:` refs resolved to files.
    pub fn preview_ui(&self, ui: &mut Ui, cache: &mut egui_commonmark::CommonMarkCache, dir: &Path) {
        // Images first, then the text. <…> lets CommonMark accept paths with spaces.
        let mut text: String = self.images.iter().map(|i| format!("![](<{}>)\n\n", blob_uri(dir, &i.blob))).collect();
        text.push_str(&self.body);
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            egui_commonmark::CommonMarkViewer::new().max_image_width(Some(720)).show(ui, cache, &text);
        });
    }

    /// Click selects, Ctrl+click adds/removes.
    pub fn select(&mut self, aid: i64, additive: bool) {
        if additive {
            if let Some(i) = self.selected.iter().position(|x| *x == aid) {
                self.selected.remove(i);
            } else {
                self.selected.push(aid);
            }
        } else {
            self.selected = vec![aid];
        }
    }

    /// "#n" of the first earlier attachment with the same blob, if this one is a copy.
    fn copy_of(&self, idx: usize) -> Option<usize> {
        let blob = &self.images[idx].blob;
        self.images[..idx].iter().position(|x| &x.blob == blob).map(|p| p + 1)
    }

    /// Thumbnail strip.
    pub fn thumbs(&self, ui: &mut Ui, dir: &Path, size: egui::Vec2) -> Option<ImgAction> {
        if self.images.is_empty() {
            return None;
        }
        let mut act = None;
        egui::ScrollArea::horizontal().id_salt(self.editor_id.with("thumbs")).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.set_min_height(size.y + 8.0);
                for (i, img) in self.images.iter().enumerate() {
                    if let Some(a) = image_tile(ui, dir, img, i + 1, self.copy_of(i), size, &self.selected) {
                        act = Some(a);
                    }
                }
            });
        });
        act
    }

    /// First image large (for image-only entries), the rest as thumbnails.
    pub fn hero(&self, ui: &mut Ui, dir: &Path, max: egui::Vec2) -> Option<ImgAction> {
        let mut act = None;
        ui.horizontal(|ui| {
            for (i, img) in self.images.iter().enumerate() {
                let size = if i == 0 { max } else { THUMB };
                if let Some(a) = image_tile(ui, dir, img, i + 1, self.copy_of(i), size, &self.selected) {
                    act = Some(a);
                }
            }
        });
        act
    }
}

/// What the user did with an image tile (handled by the app, which owns the active doc).
pub enum ImgAction {
    Open(Img),
    Select(i64, bool),
    Copy(String),
    SaveAs(String),
    Reveal(String),
    OpenWith(String),
    /// Attachment ids.
    Remove(Vec<i64>),
}

// ---------- image facts (size, dimensions, format) ----------

#[derive(Clone)]
pub struct Meta {
    pub w: u32,
    pub h: u32,
    pub bytes: u64,
    pub ext: String,
}

static META: LazyLock<Mutex<HashMap<String, Option<Meta>>>> = LazyLock::new(Default::default);

/// File size + pixel dimensions (header only) of a blob, cached for the session.
pub fn meta(dir: &Path, blob: &str) -> Option<Meta> {
    let mut cache = META.lock().unwrap_or_else(|e| e.into_inner());
    cache
        .entry(blob.to_string())
        .or_insert_with(|| {
            let path = blob_path(dir, blob);
            let bytes = std::fs::metadata(&path).ok()?.len();
            let (w, h) = image::image_dimensions(&path).ok()?;
            let ext = blob.rsplit_once('.').map(|(_, e)| e).unwrap_or("png").to_uppercase();
            Some(Meta { w, h, bytes, ext })
        })
        .clone()
}

/// 23.8 kB (decimal units, like Windows Explorer's "kB" is not — but it is what people read).
pub fn fmt_size(bytes: u64) -> (String, &'static str) {
    match bytes {
        b if b < 1_000 => (b.to_string(), "B"),
        b if b < 1_000_000 => (format!("{:.1}", b as f64 / 1e3), "kB"),
        b => (format!("{:.1}", b as f64 / 1e6), "MB"),
    }
}

/// "1920×1080 px · 23.8 kB · PNG" (for the lightbox toolbar).
pub fn meta_line(dir: &Path, blob: &str) -> String {
    meta(dir, blob)
        .map(|m| {
            let (v, u) = fmt_size(m.bytes);
            format!("{}×{} px · {v} {u} · {}", m.w, m.h, m.ext)
        })
        .unwrap_or_default()
}

/// Context menu shared by tiles and the lightbox. `targets` = attachment ids "Ta bort" applies to.
pub fn image_menu(ui: &mut Ui, blob: &str, targets: Vec<i64>) -> Option<ImgAction> {
    let mut act = None;
    let item = |ui: &mut Ui, label: &str| ui.button(egui::RichText::new(label).size(13.0)).clicked();
    if item(ui, "Kopiera") {
        act = Some(ImgAction::Copy(blob.to_string()));
    }
    if item(ui, "Spara som…") {
        act = Some(ImgAction::SaveAs(blob.to_string()));
    }
    if item(ui, "Öppna med…") {
        act = Some(ImgAction::OpenWith(blob.to_string()));
    }
    if item(ui, "Visa i mapp") {
        act = Some(ImgAction::Reveal(blob.to_string()));
    }
    ui.separator();
    let n = targets.len();
    let label = if n > 1 { format!("Ta bort {n} bilder") } else { "Ta bort".to_string() };
    if ui.button(egui::RichText::new(label).size(13.0).color(theme::ERR)).clicked() {
        act = Some(ImgAction::Remove(targets));
    }
    if act.is_some() {
        ui.close();
    }
    act
}

const SHADE: egui::Color32 = egui::Color32::from_rgba_premultiplied(12, 13, 15, 214);
const COPY_FG: egui::Color32 = egui::Color32::from_rgb(159, 225, 203);

/// Multi-colour text: (text, colour) runs in one galley.
fn runs(ui: &Ui, parts: &[(&str, egui::Color32)], size: f32) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::default();
    for (t, c) in parts {
        job.append(t, 0.0, egui::TextFormat::simple(egui::FontId::monospace(size), *c));
    }
    ui.fonts_mut(|f| f.layout_job(job))
}

/// Image tile: click = select, Ctrl+click = multi-select, double-click = enlarge, right-click = menu.
/// Always shows "#n" (and "kopia av #m") top-left and a "W×H · size" badge bottom-right;
/// on hover or when selected, a side panel with size, dimensions and format replaces the badge.
pub fn image_tile(
    ui: &mut Ui,
    dir: &Path,
    img: &Img,
    n: usize,
    copy_of: Option<usize>,
    size: egui::Vec2,
    selected: &[i64],
) -> Option<ImgAction> {
    let is_sel = selected.contains(&img.id);
    let picture = egui::Image::new(blob_uri(dir, &img.blob))
        .fit_to_exact_size(size)
        .maintain_aspect_ratio(true)
        .corner_radius(4)
        .sense(egui::Sense::click());
    let resp = egui::Frame::new()
        .fill(theme::BG_SIDE)
        .stroke(egui::Stroke::new(1.0, theme::LINE))
        .corner_radius(6)
        .inner_margin(egui::Margin::same(3))
        .show(ui, |ui| ui.add(picture))
        .inner
        .on_hover_text("Dubbelklick: förstora · Ctrl+klick: markera flera · Högerklick: meny");
    let r = resp.rect;
    let p = ui.painter().with_clip_rect(r);
    let (dim, txt, acc) = (theme::WEAK, theme::TEXT, theme::ACCENT);

    // top-left: number / copy label (+ check when selected)
    let check = if is_sel { "✓ " } else { "" };
    let (label, fg, bg) = match copy_of {
        Some(m) => (format!("{check}#{n} · kopia av #{m}"), COPY_FG, theme::ACCENT_DIM),
        None => (format!("{check}#{n}"), txt, SHADE),
    };
    let g = p.layout_no_wrap(label, egui::FontId::monospace(10.5), fg);
    let pill = egui::Rect::from_min_size(r.min + egui::vec2(5.0, 5.0), g.size() + egui::vec2(10.0, 4.0));
    p.rect_filled(pill, 3.0, bg);
    p.galley(pill.min + egui::vec2(5.0, 2.0), g, fg);

    if let Some(m) = meta(dir, &img.blob) {
        let (sv, su) = fmt_size(m.bytes);
        let (w, h) = (m.w.to_string(), m.h.to_string());
        if is_sel || resp.hovered() {
            // side panel (variant A): label small and dim, value bright, unit in accent
            let pw = (r.width() * 0.5).clamp(70.0, 96.0);
            let panel = egui::Rect::from_min_max(egui::pos2(r.right() - pw, r.top()), r.max);
            p.rect_filled(panel, 0.0, SHADE);
            let rows: [(&str, Vec<(&str, egui::Color32)>); 3] = [
                ("storlek", vec![(&sv, txt), (" ", txt), (su, acc)]),
                ("mått", vec![(&w, txt), ("×", acc), (&h, txt), (" px", acc)]),
                ("format", vec![(&m.ext, txt)]),
            ];
            let mut y = panel.top() + 7.0;
            for (lbl, val) in rows {
                let lg = runs(ui, &[(lbl, dim)], 9.5);
                p.galley(egui::pos2(panel.right() - 7.0 - lg.size().x, y), lg.clone(), dim);
                y += lg.size().y;
                let vg = runs(ui, &val, 10.5);
                p.galley(egui::pos2(panel.right() - 7.0 - vg.size().x, y), vg.clone(), txt);
                y += vg.size().y + 4.0;
            }
        } else {
            // corner badge (variant B): W×H · size
            let g = runs(ui, &[(&w, txt), ("×", dim), (&h, txt), (" · ", dim), (&sv, txt), (" ", txt), (su, acc)], 10.0);
            let badge = egui::Rect::from_min_size(r.right_bottom() - g.size() - egui::vec2(15.0, 9.0), g.size() + egui::vec2(10.0, 4.0));
            p.rect_filled(badge, 3.0, SHADE);
            p.galley(badge.min + egui::vec2(5.0, 2.0), g, txt);
        }
    }

    let frame = r.expand(3.0);
    if is_sel {
        ui.painter().rect_stroke(frame, 6.0, egui::Stroke::new(2.0, acc), egui::StrokeKind::Inside);
    } else if resp.hovered() {
        ui.painter().rect_stroke(frame, 6.0, egui::Stroke::new(1.5, acc), egui::StrokeKind::Inside);
    }

    let mut act = None;
    if resp.double_clicked() {
        act = Some(ImgAction::Open(img.clone()));
    } else if resp.clicked() {
        act = Some(ImgAction::Select(img.id, ui.input(|i| i.modifiers.ctrl)));
    }
    if resp.secondary_clicked() && !is_sel {
        act = Some(ImgAction::Select(img.id, false));
    }
    let targets = if is_sel { selected.to_vec() } else { vec![img.id] };
    resp.context_menu(|ui| {
        if let Some(a) = image_menu(ui, &img.blob, targets) {
            act = Some(a);
        }
    });
    act
}

pub fn blob_path(dir: &Path, r: &str) -> std::path::PathBuf {
    let (hash, ext) = r.split_once('.').unwrap_or((r, "png"));
    blob::path_for(dir, hash, ext)
}

pub fn blob_uri(dir: &Path, r: &str) -> String {
    let (hash, ext) = r.split_once('.').unwrap_or((r, "png"));
    if hash.len() < 2 {
        return String::new();
    }
    // egui_extras expects file:///C:/… on Windows (file://C:\… is read as a network host).
    file_uri(&blob::path_for(dir, hash, ext).to_string_lossy())
}

fn file_uri(path: &str) -> String {
    let path = path.replace('\\', "/");
    if path.starts_with('/') { format!("file://{path}") } else { format!("file:///{path}") }
}

#[cfg(test)]
mod tests {
    #[test]
    fn duplicate_needs_confirmation_then_becomes_copy() {
        use super::{Attach, Doc};
        let p = std::env::temp_dir().join(format!("omni-dup-{}.db", crate::db::now_ms()));
        let db = crate::db::Db::open(&p).unwrap();
        let mut d = Doc::new("t");
        assert!(matches!(d.attach(&db, "aa.png".into(), false), Attach::Added));
        assert!(d.id.is_some(), "an image alone creates the entry");
        assert!(matches!(d.attach(&db, "bb.png".into(), false), Attach::Added));
        assert!(matches!(d.attach(&db, "aa.png".into(), false), Attach::Duplicate(1)));
        assert_eq!(d.images.len(), 2);
        assert!(matches!(d.attach(&db, "aa.png".into(), true), Attach::Added));
        assert_eq!(d.images.len(), 3);
        assert_eq!(d.copy_of(2), Some(1));
        assert_eq!(d.copy_of(1), None);
        // removing the copy leaves the original
        let copy = d.images[2].id;
        d.detach(&db, copy);
        assert_eq!(db.attachments(d.id.unwrap()).unwrap().len(), 2);
    }

    #[test]
    fn sizes() {
        assert_eq!(super::fmt_size(292), ("292".into(), "B"));
        assert_eq!(super::fmt_size(23_800), ("23.8".into(), "kB"));
        assert_eq!(super::fmt_size(2_500_000), ("2.5".into(), "MB"));
    }

    #[test]
    fn file_uris() {
        assert_eq!(super::file_uri(r"C:\Users\x\blobs\4d\4d9e.png"), "file:///C:/Users/x/blobs/4d/4d9e.png");
        assert_eq!(super::file_uri("/home/x/blobs/ab/ab.png"), "file:///home/x/blobs/ab/ab.png");
    }
}

pub fn store_image(db: &Db, dir: &Path, img: &arboard::ImageData<'_>) -> Result<String, String> {
    let hash = blob::store_rgba(dir, img.width as u32, img.height as u32, &img.bytes).map_err(|e| format!("bild: {e}"))?;
    db.add_blob(&hash, "image/png").map_err(|e| format!("bild-db: {e}"))?;
    Ok(format!("{hash}.png"))
}

fn set_cursor(ctx: &egui::Context, id: Id, char_idx: usize) {
    if let Some(mut s) = TextEdit::load_state(ctx, id) {
        s.cursor.set_char_range(Some(CCursorRange::one(CCursor::new(char_idx))));
        s.store(ctx, id);
    }
}
