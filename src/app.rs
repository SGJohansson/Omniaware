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
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, cfg: Config, db: Db, dir: PathBuf) -> Self {
        let ctx = cc.egui_ctx.clone();
        theme::install(&ctx, cfg.window.font_size);
        egui_extras::install_image_loaders(&ctx);

        let sig = Arc::new(Signals::default());
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
        match arboard::Clipboard::new() {
            Ok(mut cb) => {
                if let Ok(t) = cb.get_text()
                    && !t.trim().is_empty()
                {
                    d.body = t.replace("\r\n", "\n");
                } else if let Ok(img) = cb.get_image() {
                    match doc::store_image(&self.db, &self.dir, &img) {
                        Ok(md) => d.body = md,
                        Err(e) => d.fail(e),
                    }
                }
            }
            Err(e) => log::error(format!("urklipp: {e}")),
        }
        if !d.body.is_empty() {
            d.mark_dirty();
            d.flush(&self.db);
        }
        d
    }

    /// Esc (save) or Shift+Esc (discard) in the capture popup.
    pub(crate) fn finish_capture(&mut self, ctx: &egui::Context, discard: bool) {
        if let Some(mut d) = self.capture.take() {
            if self.capture_naming {
                d.commit_name(&self.db);
            }
            if discard { d.discard(&self.db) } else { d.close(&self.db) }
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

    /// Ctrl+C+C: store the clipboard as a journal entry without showing anything; tray flashes green.
    fn stash(&mut self, ctx: &egui::Context) {
        if self.mode != Mode::Hidden && win::foreground() == self.hwnd {
            return; // copying inside Omniaware itself
        }
        let body = match arboard::Clipboard::new() {
            Ok(mut cb) => match cb.get_text() {
                Ok(t) if !t.trim().is_empty() => t.replace("\r\n", "\n"),
                _ => match cb.get_image() {
                    Ok(img) => match doc::store_image(&self.db, &self.dir, &img) {
                        Ok(md) => md,
                        Err(e) => return log::error(e),
                    },
                    Err(_) => return,
                },
            },
            Err(e) => return log::error(format!("urklipp: {e}")),
        };
        if self.last_stash.as_deref() != Some(body.as_str()) {
            match self.db.insert(&body).and_then(|id| self.db.add_revision(id, &body)) {
                Ok(()) => {
                    self.main.stale = true;
                    self.last_stash = Some(body);
                }
                Err(e) => return log::error(format!("tyst fångst: {e}")),
            }
        }
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
        if let Some(d) = doc
            && d.has_focus(ctx)
        {
            d.paste_image(ctx, &self.db, &self.dir);
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
            }
            Mode::Main => {
                self.poll_image_paste(&ctx);
                self.main_ui(ui);
            }
        }
    }
}
