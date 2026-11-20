//! Keyboard shortcuts such as `F9` or `Ctrl+Shift+K`, stored as text in the config file.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Mods {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModKind {
    Ctrl,
    Shift,
    Alt,
}

/// Returns which modifier an `rdev` key name stands for, if any.
pub fn modifier_kind(name: &str) -> Option<ModKind> {
    match name {
        "ControlLeft" | "ControlRight" => Some(ModKind::Ctrl),
        "ShiftLeft" | "ShiftRight" => Some(ModKind::Shift),
        "Alt" | "AltGr" => Some(ModKind::Alt),
        _ => None,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hotkey {
    pub mods: Mods,
    /// `rdev` key name of the non-modifier key.
    pub key: String,
}

impl Hotkey {
    pub fn new(mods: Mods, key: &str) -> Self {
        Self { mods, key: key.to_owned() }
    }

    /// Parses `"Ctrl+Shift+KeyK"`; a bare name such as `"F9"` has no modifiers.
    pub fn parse(text: &str) -> Self {
        let mut parts: Vec<&str> = text.split('+').collect();
        let key = parts.pop().unwrap_or("").to_owned();
        let mut mods = Mods::default();
        for part in parts {
            match part {
                "Ctrl" => mods.ctrl = true,
                "Shift" => mods.shift = true,
                "Alt" => mods.alt = true,
                _ => {}
            }
        }
        Self { mods, key }
    }

    pub fn encode(&self) -> String {
        let mut text = String::new();
        if self.mods.ctrl {
            text.push_str("Ctrl+");
        }
        if self.mods.shift {
            text.push_str("Shift+");
        }
        if self.mods.alt {
            text.push_str("Alt+");
        }
        text.push_str(&self.key);
        text
    }

    /// Human-friendly text, e.g. `Ctrl+K`.
    pub fn label(&self) -> String {
        let mut text = self.encode();
        text.truncate(text.len() - self.key.len());
        text.push_str(&key_label(&self.key));
        text
    }

    pub fn matches(&self, key: &str, mods: Mods) -> bool {
        self.key == key && self.mods == mods
    }
}

pub fn key_label(key: &str) -> String {
    if let Some(letter) = key.strip_prefix("Key").filter(|rest| rest.len() == 1) {
        return letter.to_owned();
    }
    if let Some(digit) = key.strip_prefix("Num").filter(|rest| rest.len() == 1) {
        return digit.to_owned();
    }
    key.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_with_and_without_modifiers() {
        let plain = Hotkey::parse("F9");
        assert_eq!(plain, Hotkey::new(Mods::default(), "F9"));
        assert_eq!(plain.encode(), "F9");

        let combo = Hotkey::parse("Ctrl+Shift+KeyK");
        assert!(combo.mods.ctrl && combo.mods.shift && !combo.mods.alt);
        assert_eq!(combo.key, "KeyK");
        assert_eq!(combo.encode(), "Ctrl+Shift+KeyK");
        assert_eq!(combo.label(), "Ctrl+Shift+K");
    }

    #[test]
    fn modifiers_must_match_exactly() {
        let combo = Hotkey::parse("Ctrl+KeyK");
        assert!(combo.matches("KeyK", Mods { ctrl: true, ..Mods::default() }));
        assert!(!combo.matches("KeyK", Mods::default()));
        assert!(!combo.matches("KeyK", Mods { ctrl: true, shift: true, alt: false }));
    }
}
