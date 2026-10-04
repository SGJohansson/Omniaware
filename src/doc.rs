//! One open entry being edited (capture popup or main-window editor): autosave, naming, images.

use crate::db::{self, Db, Entry};
use crate::{blob, log, theme};
use egui::text::{CCursor, CCursorRange};
use egui::{Id, TextEdit, Ui};
use std::path::Path;
use std::time::{Duration, Instant};

const DEBOUNCE: Duration = Duration::from_millis(300);

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
}

impl Doc {
    pub fn new(editor: &str) -> Self {
        Self {
            id: None,
            body: String::new(),
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
        }
    }

    pub fn open(e: Entry, editor: &str) -> Self {
        Self {
            id: Some(e.id),
            body: e.body,
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
            None if self.body.trim().is_empty() => Ok(()),
            None => db.insert(&self.body).map(|id| self.id = Some(id)),
            Some(id) => db.update_body(id, &self.body),
        };
        match res {
            Ok(()) => {
                self.dirty = false;
                if self.id.is_some() {
                    self.status = Status::Saved(chrono::Local::now().format("%H:%M:%S").to_string());
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
            && !self.body.trim().is_empty()
            && let Err(e) = db.add_revision(id, &self.body)
        {
            self.fail(format!("version: {e}"));
        }
    }

    /// Leaving the doc: snapshot, or drop/trash it if emptied.
    pub fn close(&mut self, db: &Db) {
        self.flush(db);
        let Some(id) = self.id else { return };
        let r = if !self.body.trim().is_empty() {
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
        if self.body.trim().is_empty() {
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
    pub fn paste_image(&mut self, ctx: &egui::Context, db: &Db, dir: &Path) {
        let Ok(mut cb) = arboard::Clipboard::new() else { return };
        if cb.get_text().is_ok_and(|t| !t.is_empty()) {
            return;
        }
        let Ok(img) = cb.get_image() else { return };
        let md = match store_image(db, dir, &img) {
            Ok(md) => md,
            Err(e) => return self.fail(e),
        };
        let chars = self.body.chars().count();
        let at = TextEdit::load_state(ctx, self.editor_id)
            .and_then(|s| s.cursor.char_range())
            .map(|r| r.primary.index.0.min(chars))
            .unwrap_or(chars);
        let byte = self.body.char_indices().nth(at).map(|(b, _)| b).unwrap_or(self.body.len());
        self.body.insert_str(byte, &md);
        set_cursor(ctx, self.editor_id, at + md.chars().count());
        self.mark_dirty();
    }

    /// Framed, full-size text editor.
    pub fn editor(&mut self, ui: &mut Ui, hint: &str) {
        let ctx = ui.ctx().clone();
        egui::Frame::new()
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
        if self.cursor_to_end {
            set_cursor(&ctx, self.editor_id, self.body.chars().count());
            self.cursor_to_end = false;
        }
    }

    /// Rendered markdown, `blob:` refs resolved to files.
    pub fn preview_ui(&self, ui: &mut Ui, cache: &mut egui_commonmark::CommonMarkCache, dir: &Path) {
        let mut text = self.body.clone();
        for r in db::blob_refs(&self.body) {
            text = text.replace(&format!("(blob:{r})"), &format!("({})", blob_uri(dir, &r)));
        }
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            egui_commonmark::CommonMarkViewer::new().max_image_width(Some(720)).show(ui, cache, &text);
        });
    }

    /// Thumbnail strip of referenced images.
    pub fn thumbs(&self, ui: &mut Ui, dir: &Path) {
        let refs = db::blob_refs(&self.body);
        if refs.is_empty() {
            return;
        }
        egui::ScrollArea::horizontal().id_salt("thumbs").show(ui, |ui| {
            ui.horizontal(|ui| {
                for r in refs {
                    ui.add(egui::Image::new(blob_uri(dir, &r)).max_height(64.0).corner_radius(4));
                }
            });
        });
    }
}

pub fn blob_uri(dir: &Path, r: &str) -> String {
    let (hash, ext) = r.split_once('.').unwrap_or((r, "png"));
    if hash.len() < 2 {
        return String::new();
    }
    format!("file://{}", blob::path_for(dir, hash, ext).display())
}

pub fn store_image(db: &Db, dir: &Path, img: &arboard::ImageData<'_>) -> Result<String, String> {
    let hash = blob::store_rgba(dir, img.width as u32, img.height as u32, &img.bytes).map_err(|e| format!("bild: {e}"))?;
    db.add_blob(&hash, "image/png").map_err(|e| format!("bild-db: {e}"))?;
    Ok(format!("![](blob:{hash}.png)\n"))
}

fn set_cursor(ctx: &egui::Context, id: Id, char_idx: usize) {
    if let Some(mut s) = TextEdit::load_state(ctx, id) {
        s.cursor.set_char_range(Some(CCursorRange::one(CCursor::new(char_idx))));
        s.store(ctx, id);
    }
}
