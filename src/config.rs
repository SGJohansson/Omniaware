use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Config {
    /// "wgpu" (default) or "glow" (OpenGL fallback).
    pub renderer: String,
    pub hotkeys: Hotkeys,
    pub window: WindowCfg,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Hotkeys {
    /// global-hotkey syntax: modifiers alt/ctrl/shift/super + key code, e.g. "super+alt+KeyV".
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
        Self { renderer: "wgpu".into(), hotkeys: Hotkeys::default(), window: WindowCfg::default() }
    }
}
impl Default for Hotkeys {
    fn default() -> Self {
        Self { capture: "super+alt+KeyV".into(), main: "super+alt+Space".into() }
    }
}
impl Default for WindowCfg {
    fn default() -> Self {
        Self { width: 640.0, height: 420.0, font_size: 14.0, main_width: 1040.0, main_height: 700.0 }
    }
}

/// %OMNIWARE_DATA% if set, else %APPDATA%\Omniware.
pub fn data_dir() -> PathBuf {
    if let Some(p) = std::env::var_os("OMNIWARE_DATA") {
        return PathBuf::from(p);
    }
    let base = std::env::var_os("APPDATA")
        .or_else(|| std::env::var_os("XDG_DATA_HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("Omniware")
}

/// Loads config.toml; writes defaults if missing. A broken file is never overwritten.
pub fn load(dir: &Path) -> Config {
    let path = dir.join("config.toml");
    match std::fs::read_to_string(&path) {
        Ok(s) => toml::from_str(&s).unwrap_or_else(|e| {
            crate::log::error(format!("config.toml ogiltig, använder standard: {e}"));
            Config::default()
        }),
        Err(_) => {
            let cfg = Config::default();
            if let Ok(s) = toml::to_string_pretty(&cfg) {
                let _ = std::fs::write(&path, s);
            }
            cfg
        }
    }
}
