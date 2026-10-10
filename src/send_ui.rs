//! "Send to": Ctrl+Enter sends the note to the suggested shell (asking first, until told not
//! to), Ctrl+Shift+Enter opens the chooser. Shells get the text on the prompt line, unrun.

use crate::app::{App, Mode};
use crate::detect::{self, Kind};
use crate::send::{self, Target};
use crate::text as t;
use crate::{log, notice, theme, win};
use egui::{Key, Modifiers, RichText};

pub struct SendDlg {
    /// Highlighted row in `Target::ALL`.
    pub pick: usize,
    pub admin: bool,
    /// "Don't ask again" (only offered when confirming the suggestion).
    pub remember: bool,
    /// Opened by Ctrl+Enter to confirm the suggested target.
    pub confirm: bool,
}

/// Cached detection: (text hash, result).
pub type Hint = (u64, Option<Kind>);

fn hash(s: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

fn one_line(text: &str) -> bool {
    !text.trim_end_matches(['\r', '\n']).contains('\n')
}

impl App {
    /// Text of the entry on screen.
    fn send_text(&self) -> Option<String> {
        let d = match self.mode {
            Mode::Capture => self.capture.as_ref(),
            Mode::Main => self.main.editor.as_ref(),
            Mode::Hidden => None,
        }?;
        (!d.body.trim().is_empty()).then(|| d.body.clone())
    }

    /// What the text on screen looks like (re-detected only when it changes).
    pub(crate) fn send_kind(&mut self) -> Option<Kind> {
        let text = self.send_text()?;
        let h = hash(&text);
        match self.send_hint {
            Some((k, kind)) if k == h => kind,
            _ => {
                let kind = detect::detect(&text);
                self.send_hint = Some((h, kind));
                kind
            }
        }
    }

    /// Suggested target for the text on screen.
    pub(crate) fn send_suggestion(&mut self) -> Option<Target> {
        Target::for_kind(self.send_kind())
    }

    /// Footer label: "→ PowerShell" or "send to…".
    pub(crate) fn send_label(&mut self) -> String {
        match self.send_suggestion() {
            Some(tg) => format!("→ {}", tg.label()),
            None => t::SEND_TO.to_string(),
        }
    }

    /// Ctrl+Enter / Ctrl+Shift+Enter. Call before the editor is drawn (it would take the keys).
    pub(crate) fn send_keys(&mut self, ctx: &egui::Context) {
        if self.send_dlg.is_some() || self.lightbox.is_some() || self.dup_prompt.is_some() {
            return;
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::CTRL | Modifiers::SHIFT, Key::Enter)) {
            self.open_send(ctx, false);
        } else if ctx.input_mut(|i| i.consume_key(Modifiers::CTRL, Key::Enter)) {
            self.open_send(ctx, true);
        }
    }

    /// `quick`: send to the suggestion straight away when it no longer asks.
    pub(crate) fn open_send(&mut self, ctx: &egui::Context, quick: bool) {
        if self.send_text().is_none() {
            return;
        }
        let suggestion = self.send_suggestion();
        if quick
            && let Some(tg) = suggestion
            && self.cfg.send.skip_confirm.iter().any(|s| s == tg.name())
            && self.target_ready(tg)
        {
            return self.do_send(ctx, tg, false);
        }
        let pick = suggestion.and_then(|s| Target::ALL.iter().position(|&x| x == s)).unwrap_or(0);
        self.send_dlg = Some(SendDlg { pick, admin: false, remember: false, confirm: quick && suggestion.is_some() });
    }

    /// Whether `tg` can take the text on screen (Explorer needs a path, the window a target).
    fn target_ready(&mut self, tg: Target) -> bool {
        match tg {
            Target::Explorer => matches!(self.send_kind(), Some(Kind::WinPath | Kind::UnixPath | Kind::Url)),
            Target::Window => self.prev_fg != 0,
            _ => true,
        }
    }

    fn target_note(&mut self, tg: Target, text: &str) -> (String, bool) {
        let one = one_line(text);
        match tg {
            Target::Pwsh => (t::SEND_PWSH.into(), false),
            Target::Wsl => (if one { t::SEND_WSL } else { t::SEND_WSL_MANY }.into(), false),
            Target::Cmd => (if one { t::SEND_CMD } else { t::SEND_CMD_MANY }.into(), false),
            Target::Explorer => match self.send_kind() {
                Some(Kind::Url) => (t::SEND_BROWSER.into(), false),
                Some(Kind::WinPath | Kind::UnixPath) => (t::send_opens(text.trim().trim_matches('"')), false),
                _ => (t::SEND_NO_PATH.into(), false),
            },
            Target::Window if self.prev_fg == 0 => (t::SEND_NO_WINDOW.into(), false),
            Target::Window if !one => (t::SEND_WINDOW_MANY.into(), true),
            Target::Window => (t::SEND_WINDOW.into(), false),
        }
    }

    pub(crate) fn send_dialog_ui(&mut self, ctx: &egui::Context) {
        let Some(mut dlg) = self.send_dlg.take() else { return };
        let Some(text) = self.send_text() else { return };
        let ready: Vec<bool> = Target::ALL.iter().map(|&tg| self.target_ready(tg)).collect();
        let notes: Vec<(String, bool)> = Target::ALL.iter().map(|&tg| self.target_note(tg, &text)).collect();
        let pal = theme::p();

        // Keys: digits pick and send, arrows move, A toggles administrator, Enter sends.
        let mut go = false;
        ctx.input_mut(|i| {
            for (n, k) in [Key::Num1, Key::Num2, Key::Num3, Key::Num4, Key::Num5].into_iter().enumerate() {
                if !dlg.confirm && i.consume_key(Modifiers::NONE, k) && ready[n] {
                    dlg.pick = n;
                    go = true;
                }
            }
            if !dlg.confirm && i.consume_key(Modifiers::NONE, Key::ArrowDown) {
                dlg.pick = (dlg.pick + 1) % Target::ALL.len();
            }
            if !dlg.confirm && i.consume_key(Modifiers::NONE, Key::ArrowUp) {
                dlg.pick = (dlg.pick + Target::ALL.len() - 1) % Target::ALL.len();
            }
            if i.consume_key(Modifiers::NONE, Key::A) && Target::ALL[dlg.pick].can_elevate() {
                dlg.admin = !dlg.admin;
            }
            if i.consume_key(Modifiers::NONE, Key::D) && dlg.confirm {
                dlg.remember = !dlg.remember;
            }
            if i.consume_key(Modifiers::NONE, Key::Enter) && ready[dlg.pick] {
                go = true;
            }
        });

        let mut cancel = false;
        let resp = egui::Modal::new(egui::Id::new("send_dlg")).show(ctx, |ui| {
            ui.set_width(460.0);
            let title = if dlg.confirm { t::send_confirm(Target::ALL[dlg.pick].label()) } else { t::SEND_TITLE.to_string() };
            ui.label(RichText::new(title).family(theme::medium()));
            ui.add_space(6.0);
            // Preview: the first lines, as they will arrive.
            egui::Frame::new().fill(pal.bg_side).stroke(egui::Stroke::new(1.0, pal.line)).corner_radius(4).inner_margin(egui::Margin::same(8)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                let lines: Vec<&str> = text.lines().collect();
                for l in lines.iter().take(5) {
                    let mut s: String = l.chars().take(64).collect();
                    if l.chars().count() > 64 {
                        s.push('…');
                    }
                    ui.label(RichText::new(s).monospace().size(11.5).color(pal.soft));
                }
                if lines.len() > 5 {
                    ui.label(RichText::new(t::send_more(lines.len() - 5)).size(11.0).color(pal.faint));
                }
            });
            ui.add_space(8.0);
            for (n, &tg) in Target::ALL.iter().enumerate() {
                if dlg.confirm && n != dlg.pick {
                    continue;
                }
                let on = n == dlg.pick;
                let (note, warn) = &notes[n];
                let fill = if on { pal.accent_fill } else { egui::Color32::TRANSPARENT };
                let r = egui::Frame::new().fill(fill).corner_radius(4).inner_margin(egui::Margin::symmetric(4, 1)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        if !dlg.confirm {
                            theme::key_hint(ui, &[&(n + 1).to_string()]);
                        }
                        let name_col = if !ready[n] { pal.faint } else if on { pal.accent } else { pal.text };
                        ui.label(RichText::new(format!("{:<16}", tg.label())).monospace().color(name_col));
                        let note_col = if *warn { pal.warn } else if ready[n] { pal.weak } else { pal.faint };
                        ui.label(RichText::new(note).size(11.5).color(note_col));
                    });
                });
                let row = r.response.rect;
                let click = ui.interact(row, egui::Id::new(("send_row", n)), egui::Sense::click());
                if click.clicked() && ready[n] {
                    dlg.pick = n;
                }
                if click.double_clicked() && ready[n] {
                    go = true;
                }
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let tg = Target::ALL[dlg.pick];
                ui.add_enabled_ui(tg.can_elevate(), |ui| {
                    ui.checkbox(&mut dlg.admin, t::SEND_ADMIN);
                    theme::key_hint(ui, &["A"]);
                });
                if dlg.confirm {
                    ui.add_space(10.0);
                    ui.checkbox(&mut dlg.remember, t::send_remember(tg.label()));
                    theme::key_hint(ui, &["D"]);
                }
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.add_enabled_ui(ready[dlg.pick], |ui| {
                    if theme::primary_button(ui, t::BTN_SEND).clicked() {
                        go = true;
                    }
                });
                if theme::quiet_button(ui, t::BTN_CANCEL).clicked() {
                    cancel = true;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    theme::key_hint(ui, &["esc", "enter"]);
                });
            });
        });
        if resp.should_close() {
            cancel = true;
        }
        if go && ready[dlg.pick] {
            let tg = Target::ALL[dlg.pick];
            if dlg.confirm && dlg.remember && !self.cfg.send.skip_confirm.iter().any(|s| s == tg.name()) {
                self.cfg.send.skip_confirm.push(tg.name().into());
                crate::config::save(&self.dir, &self.cfg);
            }
            self.do_send(ctx, tg, dlg.admin && tg.can_elevate());
        } else if !cancel {
            self.send_dlg = Some(dlg);
        }
    }

    /// Hands the text over. From the capture popup the note is saved and closed afterwards, so
    /// the new window has the screen to itself.
    fn do_send(&mut self, ctx: &egui::Context, tg: Target, admin: bool) {
        let Some(text) = self.send_text() else { return };
        // Make sure what is sent is also what is stored.
        match self.mode {
            Mode::Capture => {
                if let Some(d) = self.capture.as_mut() {
                    d.flush(&self.db);
                }
            }
            Mode::Main => {
                if let Some(d) = self.main.editor.as_mut() {
                    d.flush(&self.db);
                }
            }
            Mode::Hidden => {}
        }
        match tg {
            Target::Window => {
                let target = self.prev_fg;
                if self.mode == Mode::Capture {
                    self.finish_capture(ctx, false);
                    win::paste_to(target, text);
                } else {
                    self.paste_back(ctx, text);
                }
                return;
            }
            Target::Explorer => {
                if let Err(e) = send::explore(&text) {
                    report(tg, e);
                }
                return;
            }
            Target::Pwsh | Target::Cmd | Target::Wsl => {}
        }
        let file = match send::stage(&self.dir, tg, &text) {
            Ok(f) => f,
            Err(e) => return report(tg, e),
        };
        let opts = send::Opts::from(&self.cfg);
        let owner = self.hwnd;
        std::thread::spawn(move || {
            let r = if admin { send::launch_elevated(tg, &file, owner) } else { send::launch(tg, &file, &text, &opts) };
            if let Err(e) = r {
                report(tg, e);
            }
        });
        if self.mode == Mode::Capture {
            self.finish_capture(ctx, false);
        }
    }
}

fn report(tg: Target, e: String) {
    let msg = format!("{}: {e}", tg.label());
    log::error(format!("send to {msg}"));
    notice::show(notice::Notice::error(msg));
}
