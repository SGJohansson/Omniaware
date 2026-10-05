//! Capture popup UI (Win+Alt+V / tray click).

use crate::app::App;
use crate::theme;
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
        let mut toggle_help = false;
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
                    ui.label(RichText::new("Snabbanteckning").family(theme::medium()));
                    let dest = match (&doc.saved_name, naming) {
                        (Some(n), _) => format!("→ Namngivna · {n}"),
                        (None, true) => "→ Namngivna".to_string(),
                        (None, false) => format!("→ Journal · {}", theme::day_short(today)),
                    };
                    ui.label(RichText::new(dest).color(theme::WEAK).size(12.0));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        doc.indicator(ui);
                        let q = ui
                            .add(egui::Button::new(RichText::new("?").size(12.0).color(theme::WEAK)).frame(false))
                            .on_hover_text("Genvägar (håll F1)");
                        toggle_help = q.clicked();
                    });
                })
;
        });

        // ---- footer: images only (shortcuts live behind "?" / F1) ----
        if !doc.images.is_empty() {
            egui::Panel::bottom("cap_ftr").frame(bar(theme::BG, 12, 8)).show(ui, |ui| {
                if let Some(a) = doc.thumbs(ui, &dir, crate::doc::THUMB) {
                    img_act = Some(a);
                }
            });
        }

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
                    ui.label(RichText::new("Enter sparar · Esc avbryter").color(theme::WEAK).size(12.0));
                });
                if let Some(m) = &doc.name_msg {
                    ui.label(RichText::new(m).color(theme::WARN).size(12.0));
                }
                ui.add_space(4.0);
            }
            doc.editor(ui, "Skriv eller klistra in…");
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
                                theme::shortcut_groups(&mut c[0], &theme::CAPTURE_KEYS[..1]);
                                theme::shortcut_groups(&mut c[1], &theme::CAPTURE_KEYS[1..]);
                            });
                        });
                });
        }
        if toggle_help {
            self.capture_help = !self.capture_help;
        }
        if let Some(a) = img_act {
            self.image_action(a);
        }
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
