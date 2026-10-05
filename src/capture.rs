//! Capture popup UI (Ctrl+Alt+O / tray click).

use crate::app::App;
use crate::text as t;
use crate::theme;
use egui::{Align, Id, Key, Layout, Margin, Modifiers, RichText, Sense, TextEdit, ViewportCommand};

enum Action {
    None,
    Save,
    Discard,
}

/// How the name field was left.
#[derive(Clone, Copy, PartialEq)]
enum NameDone {
    /// Enter: apply, back to the text (or stay put on a name clash).
    Enter,
    /// Tab or a click elsewhere: apply, focus goes wherever the user went.
    Leave,
    /// Ctrl+Enter: apply and close the popup.
    Close,
}

/// Clicks in the footer key row.
#[derive(Clone, Copy)]
enum Foot {
    Save,
    Name,
    Version,
    Expand,
    Help,
}

fn bar(fill: egui::Color32, x: i8, y: i8) -> egui::Frame {
    egui::Frame::new().fill(fill).inner_margin(Margin::symmetric(x, y))
}

impl App {
    pub(crate) fn capture_ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        if self.capture.is_none() {
            self.hide(&ctx);
            return;
        }

        // ---- keys (the lightbox owns Esc while it is open) ----
        let modal = self.lightbox.is_some() || self.dup_prompt.is_some();
        let mut action = Action::None;
        let esc_used = self.selection_keys(&ctx);
        let (esc, shift) = ctx.input(|i| (i.key_pressed(Key::Escape), i.modifiers.shift));
        if modal || esc_used {
        } else if esc {
            if self.capture_naming {
                self.capture_naming = false;
                if let Some(d) = self.capture.as_mut() {
                    d.focus = true;
                    d.name_msg = None;
                }
            } else {
                action = if shift { Action::Discard } else { Action::Save };
            }
        } else if !self.capture_naming && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::F2)) {
            self.capture_naming = true;
            self.capture_name_focus = true;
        } else if ctx.input_mut(|i| i.consume_key(Modifiers::CTRL, Key::S)) {
            if let Some(d) = self.capture.as_mut() {
                d.save_now(&self.db);
            }
        }
        match action {
            Action::Save => return self.finish_capture(&ctx, false),
            Action::Discard => return self.finish_capture(&ctx, true),
            Action::None => {}
        }

        let naming = self.capture_naming;
        let show_help = self.capture_help || ctx.input(|i| i.key_down(Key::F1));
        let mut foot: Option<Foot> = None;
        let dir = self.dir.clone();
        let today = chrono::Local::now().date_naive();
        let Some(doc) = self.capture.as_mut() else { return };

        let mut img_act = None;

        // ---- header (drag handle) ----
        egui::Panel::top("cap_hdr").frame(bar(theme::BG, 12, 8)).show(ui, |ui| {
            // Drag area first, so widgets added afterwards stay clickable on top of it.
            if ui.interact(ui.max_rect(), Id::new("cap_drag"), Sense::drag()).drag_started() {
                ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
            }
            ui.horizontal(|ui| {
                    ui.label(RichText::new(t::QUICK_NOTE).family(theme::medium()));
                    let dest = match (&doc.saved_name, naming) {
                        (Some(n), _) => t::dest_named(Some(n)),
                        (None, true) => t::dest_named(None),
                        (None, false) => t::dest_journal(&theme::day_short(today)),
                    };
                    ui.label(RichText::new(dest).color(theme::WEAK).size(12.0));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        doc.indicator(ui);
                    });
                })
;
        });

        // ---- footer: images (if any) + one slim row of clickable shortcuts ----
        egui::Panel::bottom("cap_ftr").frame(bar(theme::BG, 12, 7)).show(ui, |ui| {
            if !doc.images.is_empty() {
                if let Some(a) = doc.thumbs(ui, &dir, crate::doc::THUMB) {
                    img_act = Some(a);
                }
                ui.add_space(6.0);
            }
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 14.0;
                let keys: [(&[&str], &str, Foot); 4] = [
                    (&["esc"], t::FOOT_SAVE, Foot::Save),
                    (&["F2"], t::FOOT_NAME, Foot::Name),
                    (&["ctrl", "s"], t::FOOT_VERSION, Foot::Version),
                    (&["ctrl", "alt", "o"], t::FOOT_EXPAND, Foot::Expand),
                ];
                for (k, label, f) in keys {
                    if theme::key_button(ui, k, label).clicked() {
                        foot = Some(f);
                    }
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if theme::key_button(ui, &["F1"], t::FOOT_ALL).on_hover_text(t::TIP_F1).clicked() {
                        foot = Some(Foot::Help);
                    }
                });
            });
        });

        // ---- body ----
        let mut name_done: Option<NameDone> = None;
        let focus_name = std::mem::take(&mut self.capture_name_focus);
        egui::CentralPanel::default().frame(bar(theme::BG, 12, 6)).show(ui, |ui| {
            if naming {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(t::NAME_LABEL).color(theme::WEAK));
                    let r = ui.add(
                        TextEdit::singleline(&mut doc.name)
                            .id(Id::new("cap_name"))
                            .desired_width(240.0)
                            .hint_text(t::NAME_HINT),
                    );
                    if focus_name {
                        r.request_focus();
                    }
                    if r.changed() {
                        doc.name_takeover = None;
                        doc.name_msg = None;
                    }
                    // Enter / Tab / a click elsewhere apply the name and keep the note open;
                    // Ctrl+Enter applies it and closes. (Esc is handled above and cancels.)
                    if r.has_focus() && ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Enter)) {
                        name_done = Some(NameDone::Close);
                    } else if r.lost_focus() {
                        let enter = ui.input(|i| i.key_pressed(Key::Enter));
                        name_done = Some(if enter { NameDone::Enter } else { NameDone::Leave });
                    }
                    theme::key_hint(ui, &["enter"]);
                    ui.label(RichText::new(t::NAME_BACK).color(theme::WEAK).size(11.0));
                    theme::key_hint(ui, &["ctrl", "enter"]);
                    ui.label(RichText::new(t::NAME_CLOSE).color(theme::WEAK).size(11.0));
                });
                if let Some(m) = &doc.name_msg {
                    ui.label(RichText::new(m).color(theme::WARN).size(12.0));
                }
                ui.add_space(4.0);
            }
            doc.editor(ui, t::EDITOR_HINT);
        });

        if show_help {
            egui::Area::new(Id::new("cap_help"))
                .order(egui::Order::Foreground)
                .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                .show(&ctx, |ui| {
                    egui::Frame::new()
                        .fill(theme::BG_SIDE)
                        .stroke(egui::Stroke::new(1.0, theme::LINE))
                        .corner_radius(8)
                        .inner_margin(Margin::same(14))
                        .show(ui, |ui| {
                            ui.set_min_width(540.0);
                            ui.columns(2, |c| {
                                theme::shortcut_groups(&mut c[0], &t::CAPTURE_KEYS[..1]);
                                theme::status_legend(&mut c[0]);
                                theme::shortcut_groups(&mut c[1], &t::CAPTURE_KEYS[1..]);
                            });
                        });
                });
        }
        match foot {
            Some(Foot::Help) => self.capture_help = !self.capture_help,
            Some(Foot::Save) => return self.finish_capture(&ctx, false),
            Some(Foot::Expand) => return self.open_main(&ctx),
            Some(Foot::Name) => {
                self.capture_naming = true;
                self.capture_name_focus = true;
            }
            Some(Foot::Version) => {
                if let Some(d) = self.capture.as_mut() {
                    d.save_now(&self.db);
                }
            }
            None => {}
        }
        if let Some(a) = img_act {
            self.image_action(a);
        }
        if let Some(how) = name_done {
            let ok = self.capture.as_mut().is_some_and(|d| d.commit_name(&self.db));
            if ok {
                self.capture_naming = false;
                if how == NameDone::Close {
                    return self.finish_capture(&ctx, false);
                }
                if let Some(d) = self.capture.as_mut()
                    && how == NameDone::Enter
                {
                    d.focus = true;
                }
            } else if how != NameDone::Leave {
                // Name clash: keep the field so a second Enter can move the name here.
                self.capture_name_focus = true;
            }
        }
    }
}
