//! App core: one OS window with two modes (capture popup / main window), hotkeys, tray.

use crate::config::Config;
use crate::db::Db;
use crate::doc::{self, Doc};
use crate::main_view::MainState;
use crate::{log, theme, tray, win};
use egui::ViewportCommand;
use global_hotkey::hotkey::HotKey;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering::SeqCst};
use tray_icon::menu::MenuEvent;
use tray_icon::{MouseButton, MouseButtonState, TrayIconEvent};

pub const TITLE: &str = "Omniaware";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Hidden,
    Capture,
    Main,
}

/// Cross-thread wake-ups from hotkey / tray handlers.
#[derive(Default)]
struct Signals {
    capture: AtomicBool,
    /// Ctrl+C+C: save clipboard silently.
    stash: AtomicBool,
    unflash: AtomicBool,
    main: AtomicBool,
    quit: AtomicBool,
}

fn poke(sig: &Signals, ctx: &egui::Context, flag: fn(&Signals) -> &AtomicBool) {
    flag(sig).store(true, SeqCst);
    ctx.request_repaint();
}

pub struct App {
    pub(crate) db: Db,
    pub(crate) dir: PathBuf,
    pub(crate) cfg: Config,
    sig: Arc<Signals>,
    hwnd: isize,
    _hotkeys: Option<GlobalHotKeyManager>,
    tray: Option<tray::Tray>,

    pub(crate) mode: Mode,
    quitting: bool,
    /// Capture was opened on top of the main window; go back there when done.
    return_to_main: bool,
    /// Window that had focus before Omni appeared (paste-back target).
    prev_fg: isize,
    main_rect: Option<win::Rect>,
    v_was_down: bool,
    last_stash: Option<String>,

    pub(crate) capture: Option<Doc>,
    pub(crate) capture_naming: bool,
    pub(crate) main: MainState,
    pub(crate) md_cache: egui_commonmark::CommonMarkCache,
    /// Image ref shown enlarged in a modal.
    pub(crate) lightbox: Option<crate::db::Img>,
    /// Pasted picture already in the entry: (blob, #n) waiting for "Lägg till kopia?".
    pub(crate) dup_prompt: Option<(String, usize)>,
    /// Shortcut overlay in the capture popup ("?" button / hold F1).
    pub(crate) capture_help: bool,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, cfg: Config, db: Db, dir: PathBuf) -> Self {
        let ctx = cc.egui_ctx.clone();
        theme::install(&ctx, cfg.window.font_size);
        egui_extras::install_image_loaders(&ctx);

        let sig = Arc::new(Signals::default());
        {
            let d = dir.clone();
            std::thread::spawn(move || crate::blob::repair_transparent(&d));
        }
        // Test hook: open a mode on startup (OMNIAWARE_OPEN=capture|main).
        match std::env::var("OMNIAWARE_OPEN").or_else(|_| std::env::var("OMNIWARE_OPEN")).as_deref() {
            Ok("capture") => sig.capture.store(true, SeqCst),
            Ok("main") => sig.main.store(true, SeqCst),
            _ => {}
        }

        // Hotkeys (manager must live on the event-loop thread and stay alive).
        let mut ids: Vec<(u32, fn(&Signals) -> &AtomicBool)> = Vec::new();
        let hotkeys = match GlobalHotKeyManager::new() {
            Ok(mgr) => {
                // `main` is optional; skip it when empty or identical to `capture`.
                let main = if cfg.hotkeys.main == cfg.hotkeys.capture { "" } else { cfg.hotkeys.main.as_str() };
                let specs: [(&str, fn(&Signals) -> &AtomicBool); 2] =
                    [(&cfg.hotkeys.capture, |s| &s.capture), (main, |s| &s.main)];
                for (spec, flag) in specs {
                    if spec.trim().is_empty() {
                        continue;
                    }
                    match spec.parse::<HotKey>().map_err(|e| e.to_string()).and_then(|hk| {
                        mgr.register(hk).map_err(|e| e.to_string())?;
                        Ok(hk.id())
                    }) {
                        Ok(id) => ids.push((id, flag)),
                        Err(e) => log::error(format!("kortkommando '{spec}': {e}")),
                    }
                }
                Some(mgr)
            }
            Err(e) => {
                log::error(format!("kortkommandon: {e}"));
                None
            }
        };
        {
            let (s, c) = (sig.clone(), ctx.clone());
            GlobalHotKeyEvent::set_event_handler(Some(move |e: GlobalHotKeyEvent| {
                if e.state == HotKeyState::Pressed
                    && let Some((_, flag)) = ids.iter().find(|(id, _)| *id == e.id)
                {
                    poke(&s, &c, *flag);
                }
            }));
        }

        {
            let (s, c) = (sig.clone(), ctx.clone());
            win::on_double_copy(move || poke(&s, &c, |s| &s.stash));
        }

        let tray = match if cfg!(windows) { tray::build() } else { Err("tray: bara Windows".into()) } {
            Ok(t) => {
                let (s, c) = (sig.clone(), ctx.clone());
                let (cap, main, quit) = (t.capture_id.clone(), t.main_id.clone(), t.quit_id.clone());
                MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
                    if e.id == cap {
                        poke(&s, &c, |s| &s.capture);
                    } else if e.id == main {
                        poke(&s, &c, |s| &s.main);
                    } else if e.id == quit {
                        poke(&s, &c, |s| &s.quit);
                    }
                }));
                let (s, c) = (sig.clone(), ctx.clone());
                TrayIconEvent::set_event_handler(Some(move |e: TrayIconEvent| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = e
                    {
                        poke(&s, &c, |s| &s.capture);
                    }
                }));
                Some(t)
            }
            Err(e) => {
                log::error(format!("tray: {e}"));
                None
            }
        };

        Self {
            db,
            dir,
            cfg,
            sig,
            hwnd: win::hwnd_of(cc),
            _hotkeys: hotkeys,
            tray: tray,
            mode: Mode::Hidden,
            quitting: false,
            return_to_main: false,
            prev_fg: 0,
            main_rect: None,
            v_was_down: false,
            last_stash: None,
            capture: None,
            capture_naming: false,
            main: MainState::new(),
            md_cache: Default::default(),
            lightbox: None,
            dup_prompt: None,
            capture_help: false,
        }
    }

    // ---------- mode switching ----------

    fn show(&self, ctx: &egui::Context, capture: bool) {
        let w = &self.cfg.window;
        if capture {
            win::place(self.hwnd, None, (w.width, w.height), true, false);
        } else {
            win::place(self.hwnd, self.main_rect, (w.main_width, w.main_height), false, true);
        }
        ctx.send_viewport_cmd(ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(ViewportCommand::Focus);
    }

    pub(crate) fn open_capture(&mut self, ctx: &egui::Context) {
        match self.mode {
            Mode::Capture => {
                ctx.send_viewport_cmd(ViewportCommand::Focus);
                return;
            }
            Mode::Main => {
                self.main_rect = win::window_rect(self.hwnd);
                if let Some(ed) = self.main.editor.as_mut() {
                    ed.checkpoint(&self.db);
                }
                self.return_to_main = true;
            }
            Mode::Hidden => {
                self.prev_fg = win::foreground();
                self.return_to_main = false;
            }
        }
        self.capture = Some(self.new_capture_doc());
        self.capture_naming = false;
        self.mode = Mode::Capture;
        self.show(ctx, true);
    }

    /// New capture doc, pre-filled from the clipboard and persisted immediately.
    fn new_capture_doc(&self) -> Doc {
        let mut d = Doc::new(&format!("capture-{}", crate::db::now_ms()));
        let clip = self.read_clipboard();
        // Pressing the shortcut again with an unchanged clipboard should not create a duplicate.
        let latest = self.db.latest().ok().flatten();
        match clip {
            Some(Clip::Text(t)) if latest.as_ref().is_none_or(|(b, _)| *b != t) => {
                d.body = t;
                d.mark_dirty();
                d.flush(&self.db);
            }
            Some(Clip::Image(r)) if latest.as_ref().is_none_or(|(b, imgs)| !(b.is_empty() && imgs == &[r.clone()])) => {
                d.attach(&self.db, r, false);
            }
            _ => {}
        }
        d
    }

    /// Clipboard as text, or as an image stored in the blob store.
    fn read_clipboard(&self) -> Option<Clip> {
        let mut cb = arboard::Clipboard::new().map_err(|e| log::error(format!("urklipp: {e}"))).ok()?;
        if let Ok(t) = cb.get_text()
            && !t.trim().is_empty()
        {
            return Some(Clip::Text(t.replace("\r\n", "\n")));
        }
        let img = cb.get_image().ok()?;
        doc::store_image(&self.db, &self.dir, &img).map_err(log::error).ok().map(Clip::Image)
    }

    /// Esc (save) or Shift+Esc (discard) in the capture popup.
    pub(crate) fn finish_capture(&mut self, ctx: &egui::Context, discard: bool) {
        if let Some(mut d) = self.capture.take() {
            if self.capture_naming {
                d.commit_name(&self.db);
            }
            if discard {
                d.discard(&self.db);
            } else {
                d.close(&self.db);
                if d.has_content() {
                    self.flash_tray(ctx);
                }
            }
        }
        self.capture_naming = false;
        self.main.stale = true;
        if std::mem::take(&mut self.return_to_main) {
            self.open_main(ctx);
        } else {
            self.hide(ctx);
        }
    }

    pub(crate) fn open_main(&mut self, ctx: &egui::Context) {
        match self.mode {
            Mode::Main => {
                ctx.send_viewport_cmd(ViewportCommand::Focus);
                return;
            }
            Mode::Capture => {
                // Expand: keep editing the same entry in the main window.
                if let Some(mut d) = self.capture.take() {
                    if self.capture_naming {
                        d.commit_name(&self.db);
                    }
                    d.flush(&self.db);
                    if d.body.trim().is_empty() {
                        d.close(&self.db);
                    } else {
                        d.focus = true;
                        if let Some(mut old) = self.main.editor.replace(d) {
                            old.close(&self.db);
                        }
                    }
                }
                self.capture_naming = false;
                self.return_to_main = false;
            }
            Mode::Hidden => self.prev_fg = win::foreground(),
        }
        self.main.stale = true;
        self.mode = Mode::Main;
        self.show(ctx, false);
    }

    pub(crate) fn hide(&mut self, ctx: &egui::Context) {
        if self.mode == Mode::Main {
            self.main_rect = win::window_rect(self.hwnd);
            if let Some(ed) = self.main.editor.as_mut() {
                ed.checkpoint(&self.db);
            }
        }
        self.mode = Mode::Hidden;
        ctx.send_viewport_cmd(ViewportCommand::Visible(false));
    }

    /// Hide Omni and paste `text` into the window that was active before it.
    pub(crate) fn paste_back(&mut self, ctx: &egui::Context, text: String) {
        self.hide(ctx);
        win::paste_to(self.prev_fg, text);
    }

    /// Enlarged image with a compact toolbar; right-click on the image gives the same menu.
    fn lightbox_ui(&mut self, ctx: &egui::Context) {
        let Some(img) = self.lightbox.clone() else { return };
        let r = img.blob.clone();
        let uri = doc::blob_uri(&self.dir, &r);
        let info = doc::meta_line(&self.dir, &r);
        let max = ctx.content_rect().size() * egui::vec2(0.85, 0.75);
        let mut close = false;
        let mut act = None;
        let resp = egui::Modal::new(egui::Id::new("lightbox")).show(ctx, |ui| {
            let pic = ui.add(
                egui::Image::new(uri)
                    .fit_to_exact_size(max)
                    .maintain_aspect_ratio(true)
                    .corner_radius(4)
                    .sense(egui::Sense::click()),
            );
            pic.context_menu(|ui| {
                if let Some(a) = doc::image_menu(ui, &r, vec![img.id]) {
                    act = Some(a);
                }
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().button_padding = egui::vec2(8.0, 2.0);
                let b = |ui: &mut egui::Ui, t: &str, c: egui::Color32| {
                    ui.add(egui::Button::new(egui::RichText::new(t).size(12.5).color(c))).clicked()
                };
                if b(ui, "Kopiera", theme::TEXT) {
                    act = Some(doc::ImgAction::Copy(r.clone()));
                }
                if b(ui, "Spara som…", theme::TEXT) {
                    act = Some(doc::ImgAction::SaveAs(r.clone()));
                }
                if b(ui, "Öppna med…", theme::TEXT) {
                    act = Some(doc::ImgAction::OpenWith(r.clone()));
                }
                if b(ui, "Visa i mapp", theme::TEXT) {
                    act = Some(doc::ImgAction::Reveal(r.clone()));
                }
                if b(ui, "Ta bort", theme::ERR) {
                    act = Some(doc::ImgAction::Remove(vec![img.id]));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    close = b(ui, "Stäng", theme::TEXT);
                    ui.label(egui::RichText::new("Esc").color(theme::WEAK).size(12.0));
                    ui.add_space(10.0);
                    ui.label(egui::RichText::new(&info).color(theme::WEAK).size(12.0));
                });
            });
        });
        let removed = matches!(act, Some(doc::ImgAction::Remove(_)));
        if let Some(a) = act {
            self.image_action(a);
        }
        if close || removed || resp.should_close() {
            self.lightbox = None;
        }
    }

    /// Applies an image action to the doc that is on screen.
    pub(crate) fn image_action(&mut self, a: doc::ImgAction) {
        use doc::ImgAction::*;
        let path = |r: &str| doc::blob_path(&self.dir, r);
        match a {
            Open(r) => self.lightbox = Some(r),
            Copy(r) => {
                if let Err(e) = copy_image(&path(&r)) {
                    log::error(format!("kopiera bild: {e}"));
                }
            }
            SaveAs(r) => {
                let src = path(&r);
                let name = format!("omniaware-{}.png", chrono::Local::now().format("%Y%m%d-%H%M%S"));
                if let Some(dst) = rfd::FileDialog::new().set_file_name(name).add_filter("PNG", &["png"]).save_file()
                    && let Err(e) = std::fs::copy(&src, &dst)
                {
                    log::error(format!("spara som {}: {e}", dst.display()));
                }
            }
            Reveal(r) => reveal(&path(&r)),
            OpenWith(r) => open_with(&path(&r)),
            Select(r, add) => {
                if let Some(d) = self.active_doc() {
                    d.select(r, add);
                }
            }
            Remove(list) => {
                let db = &self.db;
                let doc = match self.mode {
                    Mode::Capture => self.capture.as_mut(),
                    _ => self.main.editor.as_mut(),
                };
                if let Some(d) = doc {
                    for r in &list {
                        d.detach(db, *r);
                    }
                }
                self.main.stale = true;
            }
        }
    }

    fn active_doc(&mut self) -> Option<&mut Doc> {
        match self.mode {
            Mode::Capture => self.capture.as_mut(),
            Mode::Main => self.main.editor.as_mut(),
            Mode::Hidden => None,
        }
    }

    /// Delete removes selected images, Esc clears the selection (when the text field isn't focused).
    /// Returns true if it consumed Esc.
    pub(crate) fn selection_keys(&mut self, ctx: &egui::Context) -> bool {
        if self.lightbox.is_some() || self.dup_prompt.is_some() {
            return false;
        }
        let Some(d) = self.active_doc() else { return false };
        if d.selected.is_empty() || d.has_focus(ctx) {
            return false;
        }
        let (del, esc) = ctx.input(|i| (i.key_pressed(egui::Key::Delete), i.key_pressed(egui::Key::Escape)));
        if del {
            let list = d.selected.clone();
            self.image_action(doc::ImgAction::Remove(list));
        } else if esc {
            d.selected.clear();
            return true;
        }
        false
    }

    /// Ctrl+C+C: store the clipboard as a journal entry without showing anything; tray flashes green.
    fn stash(&mut self, ctx: &egui::Context) {
        if self.mode != Mode::Hidden && win::foreground() == self.hwnd {
            return; // copying inside Omniaware itself
        }
        let Some(clip) = self.read_clipboard() else { return };
        let key = match &clip {
            Clip::Text(t) => format!("t:{t}"),
            Clip::Image(r) => format!("i:{r}"),
        };
        if self.last_stash.as_deref() != Some(key.as_str()) {
            let res = match &clip {
                Clip::Text(t) => self.db.insert(t).and_then(|id| self.db.add_revision(id, t)),
                Clip::Image(r) => self.db.insert("").and_then(|id| self.db.add_attachment(id, r)).map(|_| ()),
            };
            match res {
                Ok(()) => {
                    self.main.stale = true;
                    self.last_stash = Some(key);
                }
                Err(e) => return log::error(format!("tyst fångst: {e}")),
            }
        }
        self.flash_tray(ctx);
    }

    /// Short green flash of the tray icon: "it's on disk".
    fn flash_tray(&self, ctx: &egui::Context) {
        if let Some(t) = &self.tray {
            t.flash(true);
            let (s, c) = (self.sig.clone(), ctx.clone());
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(900));
                poke(&s, &c, |s| &s.unflash);
            });
        }
    }

    /// Ctrl+V with an image-only clipboard into whichever editor has focus.
    fn poll_image_paste(&mut self, ctx: &egui::Context) {
        let v = win::key_down(win::VK_V);
        let fire = v && !self.v_was_down && ctx.input(|i| i.modifiers.ctrl);
        self.v_was_down = v;
        if !fire {
            return;
        }
        let doc = match self.mode {
            Mode::Capture => self.capture.as_mut(),
            Mode::Main => self.main.editor.as_mut(),
            Mode::Hidden => None,
        };
        // Works wherever focus is inside the window; modals own the keyboard while open.
        if self.lightbox.is_some() || self.dup_prompt.is_some() || !ctx.input(|i| i.focused) {
            return;
        }
        if let Some(d) = doc
            && let Some((blob, res)) = d.paste_image(&self.db, &self.dir)
        {
            if let doc::Attach::Duplicate(n) = res {
                self.dup_prompt = Some((blob, n));
            }
            self.main.stale = true;
        }
    }

    /// "Bilden är identisk med #n – lägg till som kopia?"
    fn dup_prompt_ui(&mut self, ctx: &egui::Context) {
        let Some((blob, n)) = self.dup_prompt.clone() else { return };
        let mut answer = None;
        let resp = egui::Modal::new(egui::Id::new("dup_prompt")).show(ctx, |ui| {
            ui.set_max_width(360.0);
            ui.horizontal(|ui| {
                ui.add(
                    egui::Image::new(doc::blob_uri(&self.dir, &blob))
                        .fit_to_exact_size(egui::vec2(96.0, 64.0))
                        .maintain_aspect_ratio(true)
                        .corner_radius(4),
                );
                ui.vertical(|ui| {
                    ui.label(egui::RichText::new(format!("Bilden är identisk med #{n}.")).family(theme::medium()));
                    ui.label(
                        egui::RichText::new("En kopia återanvänder samma fil – inget extra utrymme.")
                            .color(theme::WEAK)
                            .size(12.0),
                    );
                    ui.label(egui::RichText::new(doc::meta_line(&self.dir, &blob)).color(theme::WEAK).size(12.0));
                });
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button(egui::RichText::new("Lägg till kopia").color(theme::ACCENT)).clicked() {
                    answer = Some(true);
                }
                if ui.button("Avbryt").clicked() {
                    answer = Some(false);
                }
                ui.label(egui::RichText::new("Enter / Esc").color(theme::WEAK).size(12.0));
            });
        });
        if ctx.input(|i| i.key_pressed(egui::Key::Enter)) {
            answer = Some(true);
        }
        if resp.should_close() {
            answer = answer.or(Some(false));
        }
        match answer {
            Some(true) => {
                let doc = match self.mode {
                    Mode::Capture => self.capture.as_mut(),
                    _ => self.main.editor.as_mut(),
                };
                if let Some(d) = doc {
                    d.attach(&self.db, blob, true);
                }
                self.main.stale = true;
                self.dup_prompt = None;
            }
            Some(false) => self.dup_prompt = None,
            None => {}
        }
    }
}

impl eframe::App for App {
    /// Runs before every frame and also while hidden (on request_repaint).
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.sig.quit.swap(false, SeqCst) {
            if let Some(mut d) = self.capture.take() {
                d.close(&self.db);
            }
            if let Some(mut d) = self.main.editor.take() {
                d.close(&self.db);
            }
            self.quitting = true;
            ctx.send_viewport_cmd(ViewportCommand::Close);
            return;
        }
        if self.sig.stash.swap(false, SeqCst) {
            self.stash(ctx);
        }
        if self.sig.unflash.swap(false, SeqCst)
            && let Some(t) = &self.tray
        {
            t.flash(false);
        }
        if self.sig.capture.swap(false, SeqCst) {
            // One key, three steps: popup → main window → closed.
            match self.mode {
                Mode::Hidden => self.open_capture(ctx),
                Mode::Capture => self.open_main(ctx),
                Mode::Main => self.hide(ctx),
            }
        }
        if self.sig.main.swap(false, SeqCst) {
            if self.mode == Mode::Main { self.hide(ctx) } else { self.open_main(ctx) }
        }
        if let Some(d) = self.capture.as_mut() {
            d.tick(&self.db, ctx);
        }
        if let Some(d) = self.main.editor.as_mut() {
            d.tick(&self.db, ctx);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        if ctx.input(|i| i.viewport().close_requested()) && !self.quitting {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            match self.mode {
                Mode::Capture => self.finish_capture(&ctx, false),
                _ => self.hide(&ctx),
            }
        }
        match self.mode {
            Mode::Hidden => {
                // eframe force-shows the root window after its first painted frame
                // (epi_integration::post_rendering). Keep it hidden until summoned.
                ctx.send_viewport_cmd(ViewportCommand::Visible(false));
            }
            Mode::Capture => {
                self.poll_image_paste(&ctx);
                self.capture_ui(ui);
                self.lightbox_ui(&ctx);
                self.dup_prompt_ui(&ctx);
            }
            Mode::Main => {
                self.poll_image_paste(&ctx);
                self.main_ui(ui);
                self.lightbox_ui(&ctx);
                self.dup_prompt_ui(&ctx);
            }
        }
    }
}

/// Windows "Öppna med"-dialogen (val av program); xdg-open elsewhere.
fn open_with(path: &std::path::Path) {
    let r = if cfg!(windows) {
        std::process::Command::new("rundll32.exe")
            .arg("shell32.dll,OpenAs_RunDLL")
            .arg(path)
            .spawn()
    } else {
        std::process::Command::new("xdg-open").arg(path).spawn()
    };
    if let Err(e) = r {
        log::error(format!("öppna med {}: {e}", path.display()));
    }
}

/// Opens the folder holding the original file, with the file selected.
fn reveal(path: &std::path::Path) {
    let r = if cfg!(windows) {
        std::process::Command::new("explorer.exe").arg(format!("/select,{}", path.display())).spawn()
    } else {
        std::process::Command::new("xdg-open").arg(path.parent().unwrap_or(path)).spawn()
    };
    if let Err(e) = r {
        log::error(format!("visa i mapp {}: {e}", path.display()));
    }
}

fn copy_image(path: &std::path::Path) -> Result<(), String> {
    let img = image::open(path).map_err(|e| e.to_string())?.to_rgba8();
    let (w, h) = img.dimensions();
    let data = arboard::ImageData { width: w as usize, height: h as usize, bytes: img.into_raw().into() };
    arboard::Clipboard::new().and_then(|mut c| c.set_image(data)).map_err(|e| e.to_string())
}

enum Clip {
    Text(String),
    /// Blob ref of an image already written to the blob store.
    Image(String),
}
