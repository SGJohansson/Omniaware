//! App core: one OS window with two modes (capture popup / main window), hotkeys, tray.

use crate::config::Config;
use crate::db::Db;
use crate::doc::{self, Doc};
use crate::main_view::MainState;
use crate::{export, log, text as t, theme, tray, win};
use egui::ViewportCommand;
use global_hotkey::hotkey::HotKey;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering::SeqCst};
use std::time::{Duration, Instant};
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
    /// Notice clicked: open this entry (0 = none).
    open_entry: std::sync::atomic::AtomicI64,
}

/// Uncloak after this long even if no frame has been painted yet.
const REVEAL_TIMEOUT: Duration = Duration::from_millis(250);
/// Opens slower than this are written to the log.
const SLOW_OPEN: Duration = Duration::from_millis(200);
/// Presses that would close a window this soon after it appeared are dropped: they were
/// almost always made while it was still opening (see `App::settling`).
const SETTLE: Duration = Duration::from_millis(400);

/// A window placed by `win::place` that is still cloaked, waiting for a fresh frame.
struct Reveal {
    mode: Mode,
    since: Instant,
    /// Frames painted since it was placed (uncloak after two).
    frames: u8,
    /// Time spent in `ui` for those frames.
    ui: Duration,
    last: Instant,
    longest_gap: Duration,
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
    /// Keep asking for the foreground until this moment (Windows may refuse the first try).
    focus_until: Option<Instant>,
    /// Placed and painting, but not on screen yet.
    reveal: Option<Reveal>,
    /// When the current mode appeared on screen.
    shown_at: Option<Instant>,
    /// Last Ctrl+C+C: clipboard key and the entry it became (a repeat is not stored twice).
    last_stash: Option<(String, i64)>,

    pub(crate) capture: Option<Doc>,
    pub(crate) capture_naming: bool,
    /// Give the name field focus on the next frame (only once, so Tab / clicks can leave it).
    pub(crate) capture_name_focus: bool,
    pub(crate) main: MainState,
    pub(crate) md_cache: egui_commonmark::CommonMarkCache,
    /// Image ref shown enlarged in a modal.
    pub(crate) lightbox: Option<crate::db::Img>,
    /// Pasted picture already in the entry: (blob, #n) waiting for "add copy?".
    pub(crate) dup_prompt: Option<(String, usize)>,
    /// Shortcut overlay in the capture popup ("?" button / hold F1).
    pub(crate) capture_help: bool,
    pub(crate) theme: theme::ThemeChoice,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, cfg: Config, db: Db, dir: PathBuf) -> Self {
        let ctx = cc.egui_ctx.clone();
        let theme_choice = theme::ThemeChoice::parse(&cfg.theme);
        theme::apply(&ctx, theme_choice.resolve(ctx.system_theme() == Some(egui::Theme::Light)));
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
                        Err(e) => log::error(format!("hotkey '{spec}': {e}")),
                    }
                }
                Some(mgr)
            }
            Err(e) => {
                log::error(format!("hotkeys: {e}"));
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
        {
            let (s, c) = (sig.clone(), ctx.clone());
            crate::notice::init(move |id| {
                s.open_entry.store(id, SeqCst);
                c.request_repaint();
            });
        }

        let tray = match if cfg!(windows) { tray::build() } else { Err("tray: Windows only".into()) } {
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
            focus_until: None,
            reveal: None,
            shown_at: None,
            last_stash: None,
            capture: None,
            capture_naming: false,
            capture_name_focus: false,
            main: MainState::new(),
            md_cache: Default::default(),
            lightbox: None,
            dup_prompt: None,
            capture_help: false,
            theme: theme_choice,
        }
    }

    // ---------- theme ----------

    /// Keeps the palette in line with the chosen theme (and, for "system", with Windows).
    fn sync_theme(&mut self, ctx: &egui::Context) {
        let light = ctx.system_theme() == Some(egui::Theme::Light);
        if theme::apply(ctx, self.theme.resolve(light)) {
            ctx.request_repaint();
        }
    }

    /// Ctrl+Shift+T / the theme button: dark → light → system → voidflow, remembered in config.
    pub(crate) fn cycle_theme(&mut self, ctx: &egui::Context) {
        self.theme = self.theme.next();
        self.cfg.theme = self.theme.name().into();
        crate::config::save(&self.dir, &self.cfg);
        self.sync_theme(ctx);
    }

    /// The theme's own touches on top of the panels: VoidFlow's crimson edge and scanlines.
    fn theme_overlay(&self, ctx: &egui::Context) {
        let pal = theme::p();
        let rect = ctx.content_rect();
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("theme_overlay")));
        if let Some(edge) = pal.edge {
            painter.rect_filled(egui::Rect::from_min_size(rect.min, egui::vec2(2.0, rect.height())), 0.0, edge);
        }
        if self.cfg.scanlines && std::ptr::eq(pal, &theme::VOIDFLOW) {
            let shade = egui::Color32::from_black_alpha(46);
            let mut y = rect.top() + 1.5;
            while y < rect.bottom() {
                painter.hline(rect.x_range(), y, egui::Stroke::new(1.0, shade));
                y += 3.0;
            }
        }
    }

    // ---------- mode switching ----------

    fn show(&mut self, ctx: &egui::Context, capture: bool) {
        let w = &self.cfg.window;
        if capture {
            win::place(self.hwnd, None, (w.width, w.height), true, false);
        } else {
            win::place(self.hwnd, self.main_rect, (w.main_width, w.main_height), false, true);
        }
        // No ViewportCommand::Focus: winit's fallback for it fakes an Alt tap (see win::activate).
        ctx.send_viewport_cmd(ViewportCommand::Visible(true));
        let now = Instant::now();
        self.reveal = Some(Reveal { mode: self.mode, since: now, frames: 0, ui: Duration::ZERO, last: now, longest_gap: Duration::ZERO });
        self.shown_at = None;
        ctx.request_repaint();
    }

    /// Uncloaks the window once a frame at its new size has been painted (or after a timeout),
    /// then starts taking focus. Slow opens are logged with where the time went.
    fn tick_reveal(&mut self, ctx: &egui::Context) {
        let Some(r) = self.reveal.as_mut() else { return };
        let now = Instant::now();
        r.longest_gap = r.longest_gap.max(now - r.last);
        r.last = now;
        let waited = now - r.since;
        if r.frames < 2 && waited < REVEAL_TIMEOUT {
            ctx.request_repaint();
            return;
        }
        if waited >= SLOW_OPEN {
            log::note(format!(
                "slow open: {:?} took {} ms ({} frame(s), ui {} ms, longest frame {} ms)",
                r.mode,
                waited.as_millis(),
                r.frames,
                r.ui.as_millis(),
                r.longest_gap.as_millis()
            ));
        }
        self.reveal = None;
        win::reveal(self.hwnd);
        self.shown_at = Some(now);
        self.focus_until = Some(now + Duration::from_millis(1500));
        ctx.request_repaint();
    }

    /// Still opening, or only just opened. A shortcut press that arrives now was usually made
    /// while the window was slow to appear and would otherwise close it straight away.
    fn settling(&self) -> bool {
        self.reveal.is_some() || self.shown_at.is_some_and(|t| t.elapsed() < SETTLE)
    }

    /// Retries activation for a short while after showing; until then keys may go to the app behind.
    fn ensure_focus(&mut self, ctx: &egui::Context) {
        let Some(until) = self.focus_until else { return };
        // Done once Windows says we are in front with focus and winit/egui has seen it too.
        let ok = win::activate(self.hwnd) && ctx.input(|i| i.focused);
        if ok || Instant::now() > until {
            self.focus_until = None;
        } else {
            ctx.request_repaint_after(std::time::Duration::from_millis(40));
        }
    }

    pub(crate) fn open_capture(&mut self, ctx: &egui::Context) {
        match self.mode {
            Mode::Capture => {
                win::activate(self.hwnd);
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
        let mut cb = arboard::Clipboard::new().map_err(|e| log::error(format!("clipboard: {e}"))).ok()?;
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
                if let doc::Status::Error(e) = &d.status {
                    crate::notice::show(crate::notice::Notice::error(e.clone()));
                } else if d.has_content() {
                    self.flash_tray(ctx);
                    crate::notice::show(self.saved_notice(&d));
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
                win::activate(self.hwnd);
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
        self.reveal = None;
        self.shown_at = None;
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
            // Facts under the picture like a caption; actions on their own row.
            ui.add_space(4.0);
            doc::info_chip(ui, &self.dir, &r, 11.5);
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().button_padding = egui::vec2(8.0, 2.0);
                let b = |ui: &mut egui::Ui, t: &str, c: egui::Color32| {
                    ui.add(egui::Button::new(egui::RichText::new(t).size(12.5).color(c))).clicked()
                };
                if b(ui, t::BTN_COPY, theme::p().text) {
                    act = Some(doc::ImgAction::Copy(r.clone()));
                }
                if b(ui, t::BTN_SAVE_AS, theme::p().text) {
                    act = Some(doc::ImgAction::SaveAs(r.clone()));
                }
                if b(ui, t::BTN_OPEN_WITH, theme::p().text) {
                    act = Some(doc::ImgAction::OpenWith(r.clone()));
                }
                if b(ui, t::BTN_REVEAL, theme::p().text) {
                    act = Some(doc::ImgAction::Reveal(r.clone()));
                }
                if b(ui, t::BTN_REMOVE, theme::p().err) {
                    act = Some(doc::ImgAction::Remove(vec![img.id]));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    close = theme::quiet_button(ui, t::BTN_CLOSE).clicked();
                    theme::key_hint(ui, &["esc"]);
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
                    log::error(format!("copy image: {e}"));
                }
            }
            SaveAs(r) => {
                let src = path(&r);
                let name = format!("omniaware-{}.png", chrono::Local::now().format("%Y%m%d-%H%M%S"));
                #[cfg_attr(not(windows), allow(unused_mut))]
                let mut dlg = rfd::FileDialog::new().set_file_name(name).add_filter("PNG", &["png"]);
                #[cfg(windows)]
                {
                    dlg = dlg.set_parent(&export::owner::Owner(self.hwnd));
                }
                if let Some(dst) = dlg.save_file()
                    && let Err(e) = std::fs::copy(&src, &dst)
                {
                    log::error(format!("save as {}: {e}", dst.display()));
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

    /// Ctrl+Shift+S / "save as": the entry on screen (capture popup or main editor) to a file.
    /// The entry stays in the journal; the file is a copy. The path is remembered for next time.
    pub(crate) fn save_doc_as(&mut self) {
        let owner = self.hwnd;
        let db = &self.db;
        let d = match self.mode {
            Mode::Capture => self.capture.as_mut(),
            Mode::Main => self.main.editor.as_mut(),
            Mode::Hidden => None,
        };
        let Some(d) = d else { return };
        d.flush(db);
        let suggested = d
            .export_path
            .clone()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(export::suggest_name(&d.name, &d.body, d.created)));
        let Some(path) = export::ask_path(owner, &suggested) else { return };
        match export::write(&path, &d.body) {
            Ok(_) => d.exported(db, &path),
            Err(e) => export_failed(&path, &e),
        }
    }

    /// Selected list entries to one plain-text file (one entry: its text as is).
    pub(crate) fn export_entries(&mut self, ids: &[i64]) {
        let entries: Vec<crate::db::Entry> =
            ids.iter().filter_map(|&id| self.db.get(id).map_err(|e| log::error(format!("export #{id}: {e}"))).ok()).collect();
        let n = entries.len();
        let single = match entries.as_slice() {
            [e] => Some((e.id, e.export_path.clone(), export::suggest_name(e.name.as_deref().unwrap_or(""), &e.body, e.created))),
            _ => None,
        };
        let suggested = match &single {
            Some((_, Some(p), _)) => PathBuf::from(p),
            Some((_, None, name)) => PathBuf::from(name),
            None if n > 0 => PathBuf::from(export::dump_name(n)),
            None => return,
        };
        let text = export::dump(entries);
        let Some(path) = export::ask_path(self.hwnd, &suggested) else { return };
        match export::write(&path, &text) {
            Ok(bytes) => {
                if let Some((id, ..)) = single
                    && let Err(e) = self.db.set_export_path(id, &path.to_string_lossy())
                {
                    log::error(format!("remember export path: {e}"));
                }
                let (v, u) = doc::fmt_size(bytes);
                let file = path.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
                crate::notice::show(crate::notice::Notice::ok(
                    t::NOTICE_EXPORTED,
                    file,
                    t::exported_detail(n, &format!("{v} {u}")),
                    None,
                ));
            }
            Err(e) => export_failed(&path, &e),
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
        let (title, id) = match &self.last_stash {
            Some((k, id)) if *k == key => (t::NOTICE_ALREADY, *id),
            _ => {
                let res = match &clip {
                    Clip::Text(s) => self.db.insert(s).and_then(|id| self.db.add_revision(id, s).map(|()| id)),
                    Clip::Image(r) => self.db.insert("").and_then(|id| self.db.add_attachment(id, r).map(|_| id)),
                };
                match res {
                    Ok(id) => {
                        self.main.stale = true;
                        self.last_stash = Some((key, id));
                        (t::NOTICE_CAPTURED, id)
                    }
                    Err(e) => {
                        log::error(format!("silent capture: {e}"));
                        return crate::notice::show(crate::notice::Notice::error(e.to_string()));
                    }
                }
            }
        };
        let detail = match &clip {
            Clip::Text(s) => {
                let line = s.split_whitespace().collect::<Vec<_>>().join(" ");
                let mut d: String = line.chars().take(70).collect();
                if line.chars().count() > 70 {
                    d.push('…');
                }
                d
            }
            Clip::Image(r) => {
                let m = doc::meta(&self.dir, r);
                let size = m.as_ref().map(|m| crate::notice::detail(0, 1, m.bytes)).unwrap_or_default();
                match m {
                    Some(m) => format!("{size} · {}×{} px", m.w, m.h),
                    None => size,
                }
            }
        };
        let today = theme::day_short(chrono::Local::now().date_naive());
        crate::notice::show(crate::notice::Notice::ok(title, format!("{} · {today}", t::JOURNAL), detail, Some(id)));
        self.flash_tray(ctx);
    }

    /// "✓ Saved · Journal · Mon 5 Oct" / "42 words · 1 image · 270.7 kB".
    fn saved_notice(&self, d: &Doc) -> crate::notice::Notice {
        let context = match &d.saved_name {
            Some(n) => format!("{} · {n}", t::NAMED),
            None => format!("{} · {}", t::JOURNAL, theme::day_short(chrono::Local::now().date_naive())),
        };
        let words = d.body.split_whitespace().count();
        let mut seen = std::collections::HashSet::new();
        let bytes = d
            .images
            .iter()
            .filter(|i| seen.insert(i.blob.as_str()))
            .filter_map(|i| doc::meta(&self.dir, &i.blob))
            .map(|m| m.bytes)
            .sum();
        crate::notice::Notice::ok(t::NOTICE_SAVED, context, crate::notice::detail(words, d.images.len(), bytes), d.id)
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

    /// "This image is identical to #n" – add it as a copy?
    fn dup_prompt_ui(&mut self, ctx: &egui::Context) {
        let Some((blob, n)) = self.dup_prompt.clone() else { return };
        let mut answer = None;
        let dir = self.dir.clone();
        let resp = egui::Modal::new(egui::Id::new("dup_prompt")).show(ctx, |ui| {
            ui.set_width(400.0);
            // 1) what happened  2) why it is harmless  3) the data — each in its own weight/colour
            ui.horizontal_top(|ui| {
                egui::Frame::new()
                    .stroke(egui::Stroke::new(1.0, theme::p().accent_dim))
                    .corner_radius(4)
                    .inner_margin(egui::Margin::same(2))
                    .show(ui, |ui| {
                        ui.add(
                            egui::Image::new(doc::blob_uri(&dir, &blob))
                                .fit_to_exact_size(egui::vec2(104.0, 68.0))
                                .maintain_aspect_ratio(true)
                                .corner_radius(3),
                        );
                    });
                ui.add_space(4.0);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.label(egui::RichText::new(t::dup_title(n)).family(theme::medium()));
                    ui.label(egui::RichText::new(t::DUP_NOTE).color(theme::p().weak).size(11.5));
                    ui.add_space(10.0);
                    doc::info_chip(ui, &dir, &blob, 11.5);
                });
            });
            ui.add_space(10.0);
            let line = ui.available_rect_before_wrap();
            ui.painter().hline(line.x_range(), line.top(), egui::Stroke::new(1.0, theme::p().line));
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if theme::primary_button(ui, t::BTN_ADD_COPY).clicked() {
                    answer = Some(true);
                }
                if theme::quiet_button(ui, t::BTN_CANCEL).clicked() {
                    answer = Some(false);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    theme::key_hint(ui, &["esc", "enter"]);
                });
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
        let open = self.sig.open_entry.swap(0, SeqCst);
        if open != 0 {
            if self.mode == Mode::Capture {
                self.finish_capture(ctx, false);
            }
            self.open_main(ctx);
            self.open_entry(open);
        }
        if self.sig.unflash.swap(false, SeqCst)
            && let Some(t) = &self.tray
        {
            t.flash(false);
        }
        self.tick_reveal(ctx);
        self.sync_theme(ctx);
        if self.sig.capture.swap(false, SeqCst) {
            // One key, three steps: popup → main window → closed.
            match self.mode {
                Mode::Hidden => self.open_capture(ctx),
                Mode::Capture => self.open_main(ctx),
                Mode::Main if self.settling() => log::note("shortcut ignored: main window was still opening"),
                Mode::Main => self.hide(ctx),
            }
        }
        if self.sig.main.swap(false, SeqCst) {
            match self.mode {
                Mode::Main if self.settling() => log::note("shortcut ignored: main window was still opening"),
                Mode::Main => self.hide(ctx),
                _ => self.open_main(ctx),
            }
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
        let started = Instant::now();

        if ctx.input(|i| i.viewport().close_requested()) && !self.quitting {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            match self.mode {
                Mode::Capture => self.finish_capture(&ctx, false),
                _ => self.hide(&ctx),
            }
        }
        if self.mode != Mode::Hidden
            && self.lightbox.is_none()
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL | egui::Modifiers::SHIFT, egui::Key::T))
        {
            self.cycle_theme(&ctx);
        }
        match self.mode {
            Mode::Hidden => {
                // eframe force-shows the root window after its first painted frame
                // (epi_integration::post_rendering). Keep it hidden until summoned.
                ctx.send_viewport_cmd(ViewportCommand::Visible(false));
            }
            Mode::Capture => {
                self.ensure_focus(&ctx);
                self.poll_image_paste(&ctx);
                self.capture_ui(ui);
                self.lightbox_ui(&ctx);
                self.dup_prompt_ui(&ctx);
            }
            Mode::Main => {
                self.ensure_focus(&ctx);
                self.poll_image_paste(&ctx);
                self.main_ui(ui);
                self.lightbox_ui(&ctx);
                self.dup_prompt_ui(&ctx);
            }
        }
        // eframe is built without its browser feature; links (e.g. in the preview) open here.
        let urls: Vec<String> = ctx.output_mut(|o| {
            let mut v = Vec::new();
            o.commands.retain(|c| match c {
                egui::OutputCommand::OpenUrl(u) => {
                    v.push(u.url.clone());
                    false
                }
                _ => true,
            });
            v
        });
        for u in urls {
            theme::open_url(&u);
        }
        if self.mode != Mode::Hidden {
            self.theme_overlay(&ctx);
        }
        if let Some(r) = self.reveal.as_mut() {
            r.frames = r.frames.saturating_add(1);
            r.ui += started.elapsed();
            ctx.request_repaint();
        }
    }
}

fn export_failed(path: &std::path::Path, e: &std::io::Error) {
    let msg = format!("save as {}: {e}", path.display());
    log::error(msg.clone());
    crate::notice::show(crate::notice::Notice::error(msg));
}

/// The Windows "Open with" dialog (choose a program); xdg-open elsewhere.
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
        log::error(format!("open with {}: {e}", path.display()));
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
        log::error(format!("show in folder {}: {e}", path.display()));
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
