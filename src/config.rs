use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Config {
    /// Bumped when defaults change; old files are migrated on load.
    #[serde(default)]
    pub version: u32,
    /// "wgpu" (default) or "glow" (OpenGL fallback).
    pub renderer: String,
    pub hotkeys: Hotkeys,
    pub window: WindowCfg,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Hotkeys {
    /// global-hotkey syntax: modifiers alt/ctrl/shift/super + key code, e.g. "super+KeyO".
    /// Opens the capture popup.
    pub capture: String,
    /// Opens/closes the main window (timeline, calendar, search).
    pub main: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct WindowCfg {
    pub width: f32,
    pub height: f32,
    pub font_size: f32,
    pub main_width: f32,
    pub main_height: f32,
}

impl Default for Config {
    fn default() -> Self {
        Self { version: CONFIG_VERSION, renderer: "wgpu".into(), hotkeys: Hotkeys::default(), window: WindowCfg::default() }
    }
}
impl Default for Hotkeys {
    fn default() -> Self {
        Self { capture: "super+KeyO".into(), main: "ctrl+alt+KeyO".into() }
    }
}
impl Default for WindowCfg {
    fn default() -> Self {
        Self { width: 640.0, height: 420.0, font_size: 14.0, main_width: 1040.0, main_height: 700.0 }
    }
}

pub const CONFIG_VERSION: u32 = 2;

/// %OMNIAWARE_DATA% if set, else %APPDATA%\Omniaware (moved from the pre-rename %APPDATA%\Omniware once).
pub fn data_dir() -> PathBuf {
    if let Some(p) = std::env::var_os("OMNIAWARE_DATA").or_else(|| std::env::var_os("OMNIWARE_DATA")) {
        return PathBuf::from(p);
    }
    let base = std::env::var_os("APPDATA")
        .or_else(|| std::env::var_os("XDG_DATA_HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let (new, old) = (base.join("Omniaware"), base.join("Omniware"));
    if !new.exists() && old.exists() && std::fs::rename(&old, &new).is_err() {
        return old; // still locked by an old instance; try again next start
    }
    new
}

/// Loads config.toml; writes defaults if missing. A broken file is never overwritten.
pub fn load(dir: &Path) -> Config {
    let path = dir.join("config.toml");
    match std::fs::read_to_string(&path) {
        Ok(s) => match toml::from_str::<Config>(&s) {
            Ok(mut cfg) => {
                if cfg.version < CONFIG_VERSION {
                    migrate(&mut cfg);
                    if let Ok(s) = toml::to_string_pretty(&cfg) {
                        let _ = std::fs::write(&path, s);
                    }
                }
                cfg
            }
            Err(e) => {
                crate::log::error(format!("config.toml ogiltig, använder standard: {e}"));
                Config::default()
            }
        },
        Err(_) => {
            let cfg = Config::default();
            if let Ok(s) = toml::to_string_pretty(&cfg) {
                let _ = std::fs::write(&path, s);
            }
            cfg
        }
    }
}

/// v0/v1 → v2: replace the old default hotkeys (Win+Alt+V / Win+Alt+Space) with Win+O / Ctrl+Alt+O.
/// Hotkeys the user changed themselves are kept.
fn migrate(cfg: &mut Config) {
    let d = Hotkeys::default();
    if cfg.hotkeys.capture == "super+alt+KeyV" {
        cfg.hotkeys.capture = d.capture;
    }
    if cfg.hotkeys.main == "super+alt+Space" {
        cfg.hotkeys.main = d.main;
    }
    cfg.version = CONFIG_VERSION;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrates_old_defaults_only() {
        let mut c: Config = toml::from_str("[hotkeys]\ncapture = \"super+alt+KeyV\"\n").unwrap();
        assert_eq!(c.version, 0);
        migrate(&mut c);
        assert_eq!(c.hotkeys.capture, "super+KeyO");
        assert_eq!(c.hotkeys.main, "ctrl+alt+KeyO");
        let mut c: Config = toml::from_str("[hotkeys]\ncapture = \"ctrl+F1\"\n").unwrap();
        migrate(&mut c);
        assert_eq!(c.hotkeys.capture, "ctrl+F1");
        assert_eq!(c.version, CONFIG_VERSION);
    }
}
