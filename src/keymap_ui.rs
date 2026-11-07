//! Settings → Keybindings: a list of actions you can rebind, plus an
//! on-screen keyboard showing which keys are used and which are free.
//! Shortcuts only work inside cheapedit; nothing here touches Windows settings.

use crate::keybinds::{Action, Binding, Keybinds, key_display};
use crate::theme;
use eframe::egui::{self, Color32, Key, Modifiers, RichText, Sense, Stroke};

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Layer {
    #[default]
    None,
    Ctrl,
    Shift,
    Alt,
}

impl Layer {
    fn label(self) -> &'static str {
        match self {
            Layer::None => "No modifier",
            Layer::Ctrl => "Ctrl +",
            Layer::Shift => "Shift +",
            Layer::Alt => "Alt +",
        }
    }
    fn modifiers(self) -> Modifiers {
        match self {
            Layer::None => Modifiers::NONE,
            Layer::Ctrl => Modifiers::CTRL,
            Layer::Shift => Modifiers::SHIFT,
            Layer::Alt => Modifiers::ALT,
        }
    }
}

#[derive(Default)]
pub struct KeymapUi {
    capturing: Option<Action>,
    conflict: Option<(Action, Binding, Action)>,
    layer: Layer,
}

/// Keyboard layout: (key name, width in key units). Empty name = gap.
const ROWS: &[&[(&str, f32)]] = &[
    &[("Escape", 1.0), ("", 0.5), ("F1", 1.0), ("F2", 1.0), ("F3", 1.0), ("F4", 1.0), ("", 0.25), ("F5", 1.0), ("F6", 1.0), ("F7", 1.0), ("F8", 1.0), ("", 0.25), ("F9", 1.0), ("F10", 1.0), ("F11", 1.0), ("F12", 1.0)],
    &[("Backtick", 1.0), ("1", 1.0), ("2", 1.0), ("3", 1.0), ("4", 1.0), ("5", 1.0), ("6", 1.0), ("7", 1.0), ("8", 1.0), ("9", 1.0), ("0", 1.0), ("Minus", 1.0), ("Equals", 1.0), ("Backspace", 2.0)],
    &[("Tab", 1.5), ("Q", 1.0), ("W", 1.0), ("E", 1.0), ("R", 1.0), ("T", 1.0), ("Y", 1.0), ("U", 1.0), ("I", 1.0), ("O", 1.0), ("P", 1.0), ("OpenBracket", 1.0), ("CloseBracket", 1.0), ("Backslash", 1.5)],
    &[("", 1.75), ("A", 1.0), ("S", 1.0), ("D", 1.0), ("F", 1.0), ("G", 1.0), ("H", 1.0), ("J", 1.0), ("K", 1.0), ("L", 1.0), ("Semicolon", 1.0), ("Quote", 1.0), ("Enter", 2.25)],
    &[("", 2.25), ("Z", 1.0), ("X", 1.0), ("C", 1.0), ("V", 1.0), ("B", 1.0), ("N", 1.0), ("M", 1.0), ("Comma", 1.0), ("Period", 1.0), ("Slash", 1.0)],
    &[("", 3.75), ("Space", 6.25)],
];

const NAV: &[&[(&str, f32)]] = &[
    &[("Insert", 1.0), ("Home", 1.0), ("PageUp", 1.0)],
    &[("Delete", 1.0), ("End", 1.0), ("PageDown", 1.0)],
    &[("", 1.0)],
    &[("", 1.0), ("ArrowUp", 1.0)],
    &[("ArrowLeft", 1.0), ("ArrowDown", 1.0), ("ArrowRight", 1.0)],
];

fn cap_label(name: &str) -> String {
    match name {
        "Backtick" => "`".into(),
        "Minus" => "-".into(),
        "Equals" => "=".into(),
        "OpenBracket" => "[".into(),
        "CloseBracket" => "]".into(),
        "Backslash" => "\\".into(),
        "Semicolon" => ";".into(),
        "Quote" => "'".into(),
        "Comma" => ",".into(),
        "Period" => ".".into(),
        "Slash" => "/".into(),
        "Insert" => "Ins".into(),
        "Delete" => "Del".into(),
        other => key_display(other),
    }
}

impl KeymapUi {
    pub fn capturing(&self) -> bool {
        self.capturing.is_some() || self.conflict.is_some()
    }

    pub fn show(&mut self, ctx: &egui::Context, kb: &mut Keybinds, open: &mut bool) {
        // Key capture for rebinding.
        if let Some(action) = self.capturing {
            let mut got: Option<(Key, Modifiers)> = None;
            ctx.input(|i| {
                for ev in &i.events {
                    if let egui::Event::Key { key, pressed: true, modifiers, .. } = ev {
                        got = Some((*key, *modifiers));
                    }
                }
            });
            if let Some((key, m)) = got {
                self.capturing = None;
                if key != Key::Escape {
                    let b = Binding::new(key, m);
                    match kb.conflict(&b, action) {
                        Some(other) => self.conflict = Some((action, b, other)),
                        None => {
                            kb.map.insert(action, b);
                        }
                    }
                }
            }
        }

        egui::Window::new("Keybindings").open(open).default_width(720.0).resizable(false).collapsible(false).show(ctx, |ui| {
            theme::hint(ui, "These shortcuts only work while cheapedit is the active window. They never change anything in Windows.");
            ui.add_space(6.0);

            if let Some((action, b, other)) = self.conflict.clone() {
                theme::card().show(ui, |ui| {
                    ui.label(RichText::new(format!("{} is already used by \"{}\". Swap them?", b.display(), other.label())).color(theme::WARN));
                    ui.horizontal(|ui| {
                        if ui.button("Swap").clicked() {
                            kb.assign_swapping(action, b.clone());
                            self.conflict = None;
                        }
                        if ui.button("Cancel").clicked() {
                            self.conflict = None;
                        }
                    });
                });
                ui.add_space(6.0);
            }

            egui::Grid::new("binds").num_columns(3).spacing([16.0, 6.0]).striped(true).show(ui, |ui| {
                for a in Action::ALL {
                    ui.label(a.label());
                    if a.is_click() {
                        self.click_row(ui, kb, a);
                        ui.end_row();
                        continue;
                    }
                    let text = if self.capturing == Some(a) {
                        RichText::new("Press a key…  (Esc cancels)").color(theme::CORAL)
                    } else {
                        RichText::new(kb.get(a).map(|b| b.display()).unwrap_or_else(|| "Not set".into())).monospace()
                    };
                    if ui.add(egui::Button::new(text).min_size(egui::vec2(200.0, 0.0))).on_hover_text("Click, then press the new key").clicked() {
                        self.capturing = Some(a);
                    }
                    let default = Keybinds::default_for(a);
                    if ui.add_enabled(kb.get(a) != default.as_ref(), egui::Button::new("Reset").small()).clicked() {
                        if let Some(d) = default {
                            kb.assign_swapping(a, d);
                        }
                    }
                    ui.end_row();
                }
            });
            ui.add_space(4.0);
            if ui.button("Reset all to defaults").clicked() {
                *kb = Keybinds::default();
            }

            ui.add_space(10.0);
            ui.separator();
            ui.horizontal(|ui| {
                ui.label(RichText::new("Keyboard map").strong());
                ui.add_space(8.0);
                for l in [Layer::None, Layer::Ctrl, Layer::Shift, Layer::Alt] {
                    ui.selectable_value(&mut self.layer, l, l.label());
                }
            });
            theme::hint(ui, "Highlighted keys do something. Dim keys are free — click one to give it an action. Timeline clicks are set above.");
            ui.add_space(6.0);
            self.keyboard(ui, kb);
        });
    }

    /// A timeline-click action: pick which modifier to hold while clicking.
    /// A plain click is not offered — that always scrubs the timeline.
    fn click_row(&mut self, ui: &mut egui::Ui, kb: &mut Keybinds, a: Action) {
        const CHOICES: [Modifiers; 6] = [
            Modifiers::CTRL,
            Modifiers::SHIFT,
            Modifiers::ALT,
            Modifiers { ctrl: true, shift: true, ..Modifiers::NONE },
            Modifiers { ctrl: true, alt: true, ..Modifiers::NONE },
            Modifiers { shift: true, alt: true, ..Modifiers::NONE },
        ];
        let current = kb.get(a).map(|b| b.display()).unwrap_or_else(|| "Not set".into());
        let resp = ui.add(egui::Button::new(RichText::new(current).monospace()).min_size(egui::vec2(200.0, 0.0))).on_hover_text("Choose the key to hold while clicking the timeline");
        egui::Popup::menu(&resp).show(|ui| {
            ui.label(RichText::new("Hold this while clicking the timeline:").strong());
            for m in CHOICES {
                let b = Binding::click(m);
                if ui.button(b.display()).clicked() {
                    match kb.conflict(&b, a) {
                        Some(other) => self.conflict = Some((a, b, other)),
                        None => {
                            kb.map.insert(a, b);
                        }
                    }
                    ui.close();
                }
            }
        });
        let default = Keybinds::default_for(a);
        if ui.add_enabled(kb.get(a) != default.as_ref(), egui::Button::new("Reset").small()).clicked() {
            if let Some(d) = default {
                kb.assign_swapping(a, d);
            }
        }
    }

    fn keyboard(&mut self, ui: &mut egui::Ui, kb: &mut Keybinds) {
        let unit = 38.0;
        let h = 40.0;
        let gap = 3.0;
        let mods = self.layer.modifiers();
        let main_w = 15.0 * unit;
        let total = egui::vec2(main_w + 3.5 * unit, ROWS.len() as f32 * (h + gap));
        let (area, _) = ui.allocate_exact_size(total, Sense::hover());

        let mut draw_rows = |ui: &mut egui::Ui, rows: &[&[(&str, f32)]], origin: egui::Pos2| {
            for (r, row) in rows.iter().enumerate() {
                let mut x = origin.x;
                let y = origin.y + r as f32 * (h + gap);
                for (name, w) in row.iter() {
                    let width = w * unit - gap;
                    if !name.is_empty() {
                        if let Some(key) = Key::from_name(name) {
                            let rect = egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(width, h));
                            self.key(ui, kb, key, name, mods, rect);
                        }
                    }
                    x += w * unit;
                }
            }
        };
        draw_rows(ui, ROWS, area.min);
        draw_rows(ui, NAV, area.min + egui::vec2(main_w + 0.4 * unit, 0.0));
    }

    fn key(&mut self, ui: &mut egui::Ui, kb: &mut Keybinds, key: Key, name: &str, mods: Modifiers, rect: egui::Rect) {
        let bound = kb.map.iter().find(|(_, b)| b.matches(key, mods)).map(|(a, _)| *a);
        let id = ui.scope_id().with(("key", name, mods.ctrl, mods.shift, mods.alt));
        let resp = ui.interact(rect, id, Sense::click());
        let (fill, stroke, text_col) = match bound {
            Some(_) => (theme::ACCENT_DIM, Stroke::new(1.0, theme::ACCENT), Color32::WHITE),
            None if resp.hovered() => (theme::CARD, Stroke::new(1.0, theme::BORDER), theme::TEXT),
            None => (theme::CODE_BG, Stroke::new(1.0, theme::BORDER), theme::MUTED),
        };
        let p = ui.painter();
        p.rect(rect, 4.0, fill, stroke, egui::StrokeKind::Inside);
        p.text(rect.center_top() + egui::vec2(0.0, 12.0), egui::Align2::CENTER_CENTER, cap_label(name), egui::FontId::proportional(12.5), text_col);
        if let Some(a) = bound {
            p.text(rect.center_bottom() - egui::vec2(0.0, 10.0), egui::Align2::CENTER_CENTER, a.short(), egui::FontId::proportional(9.5), theme::CORAL);
        }

        let combo = Binding::new(key, mods).display();
        let resp = match bound {
            Some(a) => resp.on_hover_text(format!("{combo}: {}. Click to give this action a different key.", a.label())),
            None => resp.on_hover_text(format!("{combo}: free. Click to assign an action.")),
        };
        match bound {
            Some(a) => {
                if resp.clicked() {
                    self.capturing = Some(a);
                }
            }
            None => {
                egui::Popup::menu(&resp).show(|ui| {
                    ui.label(RichText::new(format!("Assign {combo} to…")).strong());
                    for a in Action::ALL.into_iter().filter(|a| !a.is_click()) {
                        let current = kb.get(a).map(|b| b.display()).unwrap_or_else(|| "not set".into());
                        if ui.button(format!("{}  (now {current})", a.label())).clicked() {
                            kb.map.insert(a, Binding::new(key, mods));
                            ui.close();
                        }
                    }
                });
            }
        }
    }
}
