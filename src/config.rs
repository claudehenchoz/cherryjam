//! App settings persisted between runs.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Additional folders to scan for VST3 plugins.
    pub extra_folders: Vec<PathBuf>,
    pub last_preset: Option<String>,
    /// Scale the plugin editor to fill the available area.
    pub fit_editor: bool,
    pub velocity: f32,
    pub show_all_plugins: bool,
    /// Bundles found to be effects when loaded; hidden from the instrument list.
    pub known_effects: Vec<PathBuf>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            extra_folders: Vec::new(),
            last_preset: None,
            fit_editor: true,
            velocity: 0.8,
            show_all_plugins: false,
            known_effects: Vec::new(),
        }
    }
}

pub fn config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("cherryjam")
}

fn path() -> PathBuf {
    config_dir().join("config.json")
}

impl Config {
    /// Scans plugin folders and applies what we learned about effects from earlier loads.
    pub fn scan(&self) -> Vec<crate::scan::PluginEntry> {
        let mut entries = crate::scan::scan(&self.extra_folders);
        for e in &mut entries {
            if e.is_instrument.is_none() && self.known_effects.contains(&e.bundle) {
                e.is_instrument = Some(false);
            }
        }
        entries
    }

    pub fn load() -> Config {
        std::fs::read_to_string(path())
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let _ = std::fs::create_dir_all(config_dir());
        if let Ok(t) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(path(), t);
        }
    }
}
