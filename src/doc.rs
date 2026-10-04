//! One open entry being edited (capture popup or main-window editor): autosave, naming, images.

use crate::db::{self, Db, Entry};
use crate::{blob, log, theme};
use egui::text::{CCursor, CCursorRange};
use egui::{Id, TextEdit, Ui};
use std::path::Path;
use std::time::{Duration, Instant};

const DEBOUNCE: Duration = Duration::from_millis(300);
/// "Sparad" toast: fade in, hold, fade out.
const TOAST_IN: f32 = 0.15;
const TOAST_HOLD: f32 = 2.5;
const TOAST_OUT: f32 = 0.6;
pub const THUMB: egui::Vec2 = egui::vec2(160.0, 96.0);
pub const THUMB_LARGE: egui::Vec2 = egui::vec2(200.0, 120.0);

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
    /// Attached images as blob refs ("<hash>.<ext>"), in order.
    pub images: Vec<String>,
    /// Selected image tiles (click / Ctrl+click); Delete removes them.
    pub selected: Vec<String>,
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
    /// (first save of the current toast, latest save) — drives the transient "Sparad" toast.
    toast: Option<(Instant, Instant)>,
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
    pub fn paste_image(&mut self, db: &Db, dir: &Path) {
        let Ok(mut cb) = arboard::Clipboard::new() else { return };
        if cb.get_text().is_ok_and(|t| !t.is_empty()) {
            return;
        }
        let Ok(img) = cb.get_image() else { return };
        match store_image(db, dir, &img) {
            Ok(r) => self.attach(db, r),
            Err(e) => self.fail(e),
        }
    }

    /// Adds an image; the entry is created first if needed (an image alone is content).
    pub fn attach(&mut self, db: &Db, r: String) {
        if self.images.contains(&r) {
            return;
        }
        self.images.push(r.clone());
        if self.id.is_none() {
            self.dirty = true;
            self.flush(db);
        }
        if let Some(id) = self.id {
            match db.add_attachment(id, &r) {
                Ok(()) => self.mark_saved(),
                Err(e) => self.fail(format!("bild: {e}")),
            }
        }
    }

    /// Removes an image from this entry (the file stays in the blob store).
    pub fn detach(&mut self, db: &Db, r: &str) {
        self.images.retain(|x| x != r);
        self.selected.retain(|x| x != r);
        if let Some(id) = self.id {
            match db.remove_attachment(id, r) {
                Ok(()) => self.mark_saved(),
                Err(e) => self.fail(format!("bild: {e}")),
            }
        }
    }

    fn mark_saved(&mut self) {
        self.status = Status::Saved(chrono::Local::now().format("%H:%M:%S").to_string());
        let now = Instant::now();
        let start = match self.toast {
            Some((s, _)) if self.toast_alpha(now) > 0.0 => s,
            _ => now,
        };
        self.toast = Some((start, now));
    }

    fn toast_alpha(&self, now: Instant) -> f32 {
        let Some((start, last)) = self.toast else { return 0.0 };
        let fade_in = (now.duration_since(start).as_secs_f32() / TOAST_IN).min(1.0);
        let since = now.duration_since(last).as_secs_f32();
        let fade_out = if since <= TOAST_HOLD { 1.0 } else { 1.0 - (since - TOAST_HOLD) / TOAST_OUT };
        (fade_in * fade_out).clamp(0.0, 1.0)
    }

    /// Permanent state: a small dot (green = everything on disk, amber = unsaved, red = error).
    pub fn indicator(&self, ui: &mut Ui) {
        let (col, tip) = match &self.status {
            Status::Error(e) => (theme::ERR, e.clone()),
            _ if self.dirty => (theme::WARN, "Osparade ändringar – sparas strax".to_string()),
            Status::Saved(t) => (theme::OK, format!("Allt sparat (senast {t})")),
            Status::Clean => (theme::OK, "Allt sparat".to_string()),
            Status::Idle => (theme::WEAK, "Inget att spara än".to_string()),
        };
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::hover());
        ui.painter().circle_filled(rect.center(), 4.0, col);
        resp.on_hover_text(tip);
        if let Status::Error(_) = self.status {
            ui.label(egui::RichText::new("fel vid sparning").color(theme::ERR).size(12.0));
        }
    }

    /// Transient "✓ Sparad" pill painted over the bottom-right corner of `area` (no layout impact).
    fn paint_toast(&self, ctx: &egui::Context, area: egui::Rect) {
        let a = self.toast_alpha(Instant::now());
        if a <= 0.0 {
            return;
        }
        ctx.request_repaint();
        let p = ctx.layer_painter(egui::LayerId::new(egui::Order::Middle, self.editor_id.with("toast")));
        let g = p.layout_no_wrap("✓ Sparad".into(), egui::FontId::monospace(12.0), theme::OK.gamma_multiply(a));
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
        let mut text: String = self.images.iter().map(|r| format!("![](<{}>)\n\n", blob_uri(dir, r))).collect();
        text.push_str(&self.body);
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            egui_commonmark::CommonMarkViewer::new().max_image_width(Some(720)).show(ui, cache, &text);
        });
    }

    /// Click selects, Ctrl+click adds/removes, empty selection clears.
    pub fn select(&mut self, r: &str, additive: bool) {
        if additive {
            if let Some(i) = self.selected.iter().position(|x| x == r) {
                self.selected.remove(i);
            } else {
                self.selected.push(r.to_string());
            }
        } else {
            self.selected = vec![r.to_string()];
        }
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
                for r in &self.images {
                    if let Some(a) = image_tile(ui, dir, r, size, &self.selected) {
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
            for (i, r) in self.images.iter().enumerate() {
                let size = if i == 0 { max } else { THUMB };
                if let Some(a) = image_tile(ui, dir, r, size, &self.selected) {
                    act = Some(a);
                }
            }
        });
        act
    }
}

/// What the user did with an image tile (handled by the app, which owns the active doc).
pub enum ImgAction {
    Open(String),
    Select(String, bool),
    Copy(String),
    SaveAs(String),
    Reveal(String),
    OpenWith(String),
    Remove(Vec<String>),
}

/// Context menu shared by tiles and the lightbox. `targets` = what "Ta bort" applies to.
pub fn image_menu(ui: &mut Ui, r: &str, targets: Vec<String>) -> Option<ImgAction> {
    let mut act = None;
    let item = |ui: &mut Ui, label: &str| ui.button(egui::RichText::new(label).size(13.0)).clicked();
    if item(ui, "Kopiera") {
        act = Some(ImgAction::Copy(r.to_string()));
    }
    if item(ui, "Spara som…") {
        act = Some(ImgAction::SaveAs(r.to_string()));
    }
    if item(ui, "Öppna med…") {
        act = Some(ImgAction::OpenWith(r.to_string()));
    }
    if item(ui, "Visa i mapp") {
        act = Some(ImgAction::Reveal(r.to_string()));
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

/// Framed image tile: click = select, Ctrl+click = multi-select, double-click = enlarge,
/// right-click = menu. Selected tiles get an accent border and a check mark.
pub fn image_tile(ui: &mut Ui, dir: &Path, r: &str, size: egui::Vec2, selected: &[String]) -> Option<ImgAction> {
    let is_sel = selected.iter().any(|x| x == r);
    let img = egui::Image::new(blob_uri(dir, r))
        .fit_to_exact_size(size)
        .maintain_aspect_ratio(true)
        .corner_radius(4)
        .sense(egui::Sense::click());
    let resp = egui::Frame::new()
        .fill(theme::BG_SIDE)
        .stroke(egui::Stroke::new(1.0, theme::LINE))
        .corner_radius(6)
        .inner_margin(egui::Margin::same(3))
        .show(ui, |ui| ui.add(img))
        .inner
        .on_hover_text("Dubbelklick: förstora · Ctrl+klick: markera flera · Högerklick: meny");
    let frame = resp.rect.expand(3.0);
    if is_sel {
        ui.painter().rect_stroke(frame, 6.0, egui::Stroke::new(2.0, theme::ACCENT), egui::StrokeKind::Inside);
        let c = frame.right_top() + egui::vec2(-12.0, 12.0);
        ui.painter().circle(c, 8.0, theme::BG, egui::Stroke::new(1.5, theme::ACCENT));
        ui.painter().text(c, egui::Align2::CENTER_CENTER, "✓", egui::FontId::monospace(11.0), theme::ACCENT);
    } else if resp.hovered() {
        ui.painter().rect_stroke(frame, 6.0, egui::Stroke::new(1.5, theme::ACCENT), egui::StrokeKind::Inside);
    }
    let mut act = None;
    if resp.double_clicked() {
        act = Some(ImgAction::Open(r.to_string()));
    } else if resp.clicked() {
        act = Some(ImgAction::Select(r.to_string(), ui.input(|i| i.modifiers.ctrl)));
    }
    if resp.secondary_clicked() && !is_sel {
        act = Some(ImgAction::Select(r.to_string(), false));
    }
    let targets = if is_sel { selected.to_vec() } else { vec![r.to_string()] };
    resp.context_menu(|ui| {
        if let Some(a) = image_menu(ui, r, targets) {
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
