//! Capture popup UI (Win+Alt+V / tray click).

use crate::app::App;
use crate::theme::{self, hint};
use egui::{Align, Id, Key, Layout, Margin, Modifiers, RichText, Sense, TextEdit, ViewportCommand};

enum Action {
    None,
    Save,
    Discard,
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

        // ---- keys ----
        let mut action = Action::None;
        let (esc, shift) = ctx.input(|i| (i.key_pressed(Key::Escape), i.modifiers.shift));
        if esc {
            if self.capture_naming {
                self.capture_naming = false;
                if let Some(d) = self.capture.as_mut() {
                    d.focus = true;
                    d.name_msg = None;
                }
            } else {
                action = if shift { Action::Discard } else { Action::Save };
            }
        } else if !self.capture_naming && ctx.input_mut(|i| i.consume_key(Modifiers::CTRL, Key::S)) {
            self.capture_naming = true;
        }
        match action {
            Action::Save => return self.finish_capture(&ctx, false),
            Action::Discard => return self.finish_capture(&ctx, true),
            Action::None => {}
        }

        let naming = self.capture_naming;
        let dir = self.dir.clone();
        let today = chrono::Local::now().date_naive();
        let Some(doc) = self.capture.as_mut() else { return };

        // ---- header (drag handle) ----
        egui::Panel::top("cap_hdr").frame(bar(theme::BG, 12, 8)).show(ui, |ui| {
            let r = ui
                .horizontal(|ui| {
                    ui.label(RichText::new("Snabbanteckning").family(theme::medium()));
                    let dest = match (&doc.saved_name, naming) {
                        (Some(n), _) => format!("→ Namngivna · {n}"),
                        (None, true) => "→ Namngivna".to_string(),
                        (None, false) => format!("→ Journal · {}", theme::day_short(today)),
                    };
                    ui.label(RichText::new(dest).color(theme::WEAK).size(12.0));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| theme::status_label(ui, &doc.status));
                })
                .response;
            if ui.interact(r.rect, Id::new("cap_drag"), Sense::drag()).drag_started() {
                ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
            }
        });

        // ---- footer ----
        egui::Panel::bottom("cap_ftr").frame(bar(theme::BG, 12, 8)).show(ui, |ui| {
            doc.thumbs(ui, &dir);
            ui.horizontal(|ui| {
                if naming {
                    hint(ui, &["Enter"], "spara namn");
                    hint(ui, &["Esc"], "avbryt");
                } else {
                    hint(ui, &["Esc"], "spara");
                    hint(ui, &["Ctrl", "Alt", "O"], "vidga");
                    hint(ui, &["Ctrl", "S"], "namnge");
                    hint(ui, &["Ctrl", "V"], "bild");
                    hint(ui, &["Shift", "Esc"], "kasta");
                }
            });
        });

        // ---- body ----
        let mut confirm = false;
        egui::CentralPanel::default().frame(bar(theme::BG, 12, 6)).show(ui, |ui| {
            if naming {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Namn").color(theme::WEAK));
                    let r = ui.add(
                        TextEdit::singleline(&mut doc.name)
                            .id(Id::new("cap_name"))
                            .desired_width(280.0)
                            .hint_text("adress-jobb"),
                    );
                    if !r.has_focus() && doc.name_msg.is_none() && !r.lost_focus() {
                        r.request_focus();
                    }
                    if r.changed() {
                        doc.name_takeover = None;
                        doc.name_msg = None;
                    }
                    if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                        confirm = true;
                    }
                });
                if let Some(m) = &doc.name_msg {
                    ui.label(RichText::new(m).color(theme::WARN).size(12.0));
                }
                ui.add_space(4.0);
            }
            doc.editor(ui, "Skriv eller klistra in…");
        });

        if confirm {
            let ok = self.capture.as_mut().is_some_and(|d| d.commit_name(&self.db));
            if ok {
                self.capture_naming = false;
                self.finish_capture(&ctx, false);
            } else {
                ctx.memory_mut(|m| m.request_focus(Id::new("cap_name")));
            }
        }
    }
}
