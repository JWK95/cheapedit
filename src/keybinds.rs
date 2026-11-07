//! In-app keyboard shortcuts. These only work while cheapedit's window is
//! focused — they are never global hotkeys and change nothing in Windows.

use eframe::egui::{self, Key, Modifiers};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Action {
    PlayPause,
    FrameBack,
    FrameForward,
    SeekBack,
    SeekForward,
    SetStart,
    SetEnd,
    ToggleView,
    Export,
    OpenFile,
    ShowShortcuts,
}

impl Action {
    pub const ALL: [Action; 11] = [
        Action::PlayPause,
        Action::FrameBack,
        Action::FrameForward,
        Action::SeekBack,
        Action::SeekForward,
        Action::SetStart,
        Action::SetEnd,
        Action::ToggleView,
        Action::Export,
        Action::OpenFile,
        Action::ShowShortcuts,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Action::PlayPause => "Play / pause",
            Action::FrameBack => "Back one frame",
            Action::FrameForward => "Forward one frame",
            Action::SeekBack => "Back 1 second",
            Action::SeekForward => "Forward 1 second",
            Action::SetStart => "Set start",
            Action::SetEnd => "Set end",
            Action::ToggleView => "Switch Original / Result",
            Action::Export => "Export clip",
            Action::OpenFile => "Open video",
            Action::ShowShortcuts => "Show shortcuts",
        }
    }
    /// Short label for the on-screen keyboard.
    pub fn short(self) -> &'static str {
        match self {
            Action::PlayPause => "Play",
            Action::FrameBack => "-1 frame",
            Action::FrameForward => "+1 frame",
            Action::SeekBack => "-1 s",
            Action::SeekForward => "+1 s",
            Action::SetStart => "Start",
            Action::SetEnd => "End",
            Action::ToggleView => "Orig/Res",
            Action::Export => "Export",
            Action::OpenFile => "Open",
            Action::ShowShortcuts => "Keys",
        }
    }
    /// Shown in the strip under the player.
    pub fn in_strip(self) -> bool {
        matches!(self, Action::PlayPause | Action::FrameBack | Action::FrameForward | Action::SetStart | Action::SetEnd | Action::Export)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Binding {
    /// egui key name, e.g. "Space", "I", "ArrowLeft".
    pub key: String,
    #[serde(default)]
    pub ctrl: bool,
    #[serde(default)]
    pub shift: bool,
    #[serde(default)]
    pub alt: bool,
}

impl Binding {
    pub fn new(key: Key, m: Modifiers) -> Binding {
        Binding { key: key.name().to_string(), ctrl: m.ctrl || m.command, shift: m.shift, alt: m.alt }
    }
    fn plain(key: Key) -> Binding {
        Binding::new(key, Modifiers::NONE)
    }
    pub fn key(&self) -> Option<Key> {
        Key::from_name(&self.key)
    }
    pub fn matches(&self, key: Key, m: Modifiers) -> bool {
        self.key() == Some(key) && self.ctrl == (m.ctrl || m.command) && self.shift == m.shift && self.alt == m.alt
    }
    pub fn has_modifier(&self) -> bool {
        self.ctrl || self.alt
    }
    pub fn display(&self) -> String {
        let mut s = String::new();
        if self.ctrl {
            s.push_str("Ctrl+");
        }
        if self.alt {
            s.push_str("Alt+");
        }
        if self.shift {
            s.push_str("Shift+");
        }
        s.push_str(&key_display(&self.key));
        s
    }
}

pub fn key_display(name: &str) -> String {
    match name {
        // Words, not arrow symbols: the bundled font has no arrow glyphs.
        "ArrowLeft" | "Left" | "⏴" => "Left".into(),
        "ArrowRight" | "Right" | "⏵" => "Right".into(),
        "ArrowUp" | "Up" | "⏶" => "Up".into(),
        "ArrowDown" | "Down" | "⏷" => "Down".into(),
        "PageUp" => "PgUp".into(),
        "PageDown" => "PgDn".into(),
        "Escape" => "Esc".into(),
        "Backspace" => "Bksp".into(),
        other => other.into(),
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Keybinds {
    pub map: BTreeMap<Action, Binding>,
}

impl Default for Keybinds {
    fn default() -> Self {
        let mut map = BTreeMap::new();
        let shift = Modifiers::SHIFT;
        let ctrl = Modifiers::CTRL;
        map.insert(Action::PlayPause, Binding::plain(Key::Space));
        map.insert(Action::FrameBack, Binding::plain(Key::ArrowLeft));
        map.insert(Action::FrameForward, Binding::plain(Key::ArrowRight));
        map.insert(Action::SeekBack, Binding::new(Key::ArrowLeft, shift));
        map.insert(Action::SeekForward, Binding::new(Key::ArrowRight, shift));
        map.insert(Action::SetStart, Binding::plain(Key::I));
        map.insert(Action::SetEnd, Binding::plain(Key::O));
        map.insert(Action::ToggleView, Binding::plain(Key::V));
        map.insert(Action::Export, Binding::new(Key::E, ctrl));
        map.insert(Action::OpenFile, Binding::new(Key::O, ctrl));
        map.insert(Action::ShowShortcuts, Binding::plain(Key::F1));
        Keybinds { map }
    }
}

impl Keybinds {
    pub fn default_for(a: Action) -> Option<Binding> {
        Keybinds::default().map.get(&a).cloned()
    }
    /// Give any action missing from saved settings its default binding,
    /// unless that binding is already taken by something else.
    pub fn fill_missing(&mut self) {
        for (a, b) in Keybinds::default().map {
            if !self.map.contains_key(&a) && self.conflict(&b, a).is_none() {
                self.map.insert(a, b);
            }
        }
    }
    pub fn get(&self, a: Action) -> Option<&Binding> {
        self.map.get(&a)
    }
    /// Which action (other than `except`) already uses this binding?
    pub fn conflict(&self, b: &Binding, except: Action) -> Option<Action> {
        self.map.iter().find(|(a, x)| **a != except && *x == b).map(|(a, _)| *a)
    }
    /// Set `a` to `b`. If another action had `b`, it gets `a`'s old key (swap).
    pub fn assign_swapping(&mut self, a: Action, b: Binding) {
        let old = self.map.get(&a).cloned();
        if let Some(other) = self.conflict(&b, a) {
            match old {
                Some(o) => {
                    self.map.insert(other, o);
                }
                None => {
                    self.map.remove(&other);
                }
            }
        }
        self.map.insert(a, b);
    }
    /// Read this frame's key presses and return the actions they trigger.
    /// While a text box has focus, only Ctrl/Alt shortcuts fire.
    pub fn triggered(&self, ctx: &egui::Context) -> Vec<Action> {
        let typing = ctx.egui_wants_keyboard_input();
        let mut out = Vec::new();
        ctx.input(|i| {
            for ev in &i.events {
                if let egui::Event::Key { key, pressed: true, modifiers, .. } = ev {
                    for (a, b) in &self.map {
                        if b.matches(*key, *modifiers) && (!typing || b.has_modifier()) {
                            out.push(*a);
                        }
                    }
                }
            }
        });
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swap_on_conflict() {
        let mut k = Keybinds::default();
        let i = k.get(Action::SetStart).cloned().unwrap();
        let o = k.get(Action::SetEnd).cloned().unwrap();
        k.assign_swapping(Action::SetStart, o.clone());
        assert_eq!(k.get(Action::SetStart), Some(&o));
        assert_eq!(k.get(Action::SetEnd), Some(&i));
    }

    #[test]
    fn exact_modifiers() {
        let b = Binding::new(Key::ArrowLeft, Modifiers::NONE);
        assert!(b.matches(Key::ArrowLeft, Modifiers::NONE));
        assert!(!b.matches(Key::ArrowLeft, Modifiers::SHIFT));
    }

    #[test]
    fn roundtrips_through_toml() {
        let k = Keybinds::default();
        let s = toml::to_string(&k).unwrap();
        let back: Keybinds = toml::from_str(&s).unwrap();
        assert_eq!(k, back);
    }
}
