//! Keyboard shortcuts such as `F9` or `Ctrl+Shift+K`, stored as text in the config file.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Mods {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub win: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModKind {
    Ctrl,
    Shift,
    Alt,
    Win,
}

/// Returns which modifier an `rdev` key name stands for, if any.
pub fn modifier_kind(name: &str) -> Option<ModKind> {
    match name {
        "ControlLeft" | "ControlRight" => Some(ModKind::Ctrl),
        "ShiftLeft" | "ShiftRight" => Some(ModKind::Shift),
        "Alt" | "AltGr" => Some(ModKind::Alt),
        "MetaLeft" | "MetaRight" => Some(ModKind::Win),
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
                "Win" => mods.win = true,
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
        if self.mods.win {
            text.push_str("Win+");
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

/// Keys that can be used in a shortcut even though they cannot be recorded in a macro:
/// keypad keys, Num Lock, Print Screen and media keys (reported by rdev as `Unknown(vk)`).
pub fn hotkey_key_allowed(name: &str) -> bool {
    name.starts_with("Kp") || name == "NumLock" || name == "PrintScreen" || media_key_label(name).is_some()
}

fn media_key_label(name: &str) -> Option<&'static str> {
    Some(match name {
        "Unknown(173)" => "Mudo",
        "Unknown(174)" => "Volume -",
        "Unknown(175)" => "Volume +",
        "Unknown(176)" => "Próxima faixa",
        "Unknown(177)" => "Faixa anterior",
        "Unknown(178)" => "Parar mídia",
        "Unknown(179)" => "Play/Pausa mídia",
        _ => return None,
    })
}

pub fn key_label(key: &str) -> String {
    if let Some(label) = media_key_label(key) {
        return label.to_owned();
    }
    if let Some(rest) = key.strip_prefix("Kp") {
        return format!("Num {rest}");
    }
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
        assert!(!combo.matches("KeyK", Mods { ctrl: true, shift: true, ..Mods::default() }));
    }

    #[test]
    fn supports_windows_key_keypad_and_media_keys() {
        let combo = Hotkey::parse("Ctrl+Win+Kp5");
        assert!(combo.mods.ctrl && combo.mods.win);
        assert_eq!(combo.label(), "Ctrl+Win+Num 5");
        assert!(hotkey_key_allowed("Kp5"));
        assert!(hotkey_key_allowed("Unknown(175)"));
        assert!(!hotkey_key_allowed("Unknown(999)"));
    }
}
