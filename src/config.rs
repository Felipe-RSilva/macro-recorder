//! Small user-settings file stored in `%APPDATA%\MacroRecorder\config.json`.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// `rdev` key name that starts/stops recording.
    pub record_hotkey: String,
    /// `rdev` key name that pauses/resumes playback.
    pub pause_hotkey: String,
    /// 0 = 1x, 1 = 0.5x, 2 = 2x, 100 = 100x, `u32::MAX` = custom.
    pub speed: u32,
    pub custom_speed: u32,
    pub repeats: u32,
    pub continuous: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            record_hotkey: "F9".to_owned(),
            pause_hotkey: "F10".to_owned(),
            speed: 0,
            custom_speed: 8,
            repeats: 1,
            continuous: false,
        }
    }
}

fn config_path() -> Option<PathBuf> {
    let base = std::env::var_os("APPDATA").or_else(|| std::env::var_os("HOME"))?;
    Some(PathBuf::from(base).join("MacroRecorder").join("config.json"))
}

impl Config {
    pub fn load() -> Self {
        config_path()
            .and_then(|path| std::fs::read(path).ok())
            .and_then(|bytes| serde_json::from_slice::<Config>(&bytes).ok())
            .map(|mut config| {
                config.repeats = config.repeats.max(1);
                config.custom_speed = config.custom_speed.clamp(1, 100);
                config
            })
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let Some(path) = config_path() else { return };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(bytes) = serde_json::to_vec_pretty(self) {
            let _ = std::fs::write(path, bytes);
        }
    }
}
