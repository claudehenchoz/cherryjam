//! Presets: instrument + its state + controller mapping + effects, stored as JSON.

use std::path::{Path, PathBuf};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use serde::{Deserialize, Serialize};

use crate::controller::Mapping;
use crate::fx::FxSettings;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preset {
    pub name: String,
    pub plugin_name: String,
    /// Path of the `.vst3` bundle (or file).
    pub plugin_path: PathBuf,
    /// Class id as 32 hex chars.
    pub class_id: String,
    pub component_state: String,
    pub controller_state: String,
    pub mapping: Mapping,
    pub fx: FxSettings,
}

impl Preset {
    pub fn set_state(&mut self, comp: &[u8], ctl: &[u8]) {
        self.component_state = B64.encode(comp);
        self.controller_state = B64.encode(ctl);
    }

    pub fn state(&self) -> (Vec<u8>, Vec<u8>) {
        (
            B64.decode(&self.component_state).unwrap_or_default(),
            B64.decode(&self.controller_state).unwrap_or_default(),
        )
    }
}

pub fn dir() -> PathBuf {
    crate::config::config_dir().join("presets")
}

fn file_name(name: &str) -> String {
    let clean: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || " -_()".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("{}.json", clean.trim())
}

pub fn path_for(name: &str) -> PathBuf {
    dir().join(file_name(name))
}

pub fn list() -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir())
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| load_file(&e.path()).ok().map(|p| p.name))
        .collect();
    v.sort_by_key(|n| n.to_lowercase());
    v
}

pub fn load_file(path: &Path) -> Result<Preset, String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    serde_json::from_str(&text).map_err(|e| e.to_string())
}

pub fn load(name: &str) -> Result<Preset, String> {
    load_file(&path_for(name))
}

pub fn save(p: &Preset) -> Result<(), String> {
    std::fs::create_dir_all(dir()).map_err(|e| e.to_string())?;
    let text = serde_json::to_string_pretty(p).map_err(|e| e.to_string())?;
    std::fs::write(path_for(&p.name), text).map_err(|e| e.to_string())
}

pub fn delete(name: &str) -> Result<(), String> {
    std::fs::remove_file(path_for(name)).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_roundtrip() {
        let mut p = Preset {
            name: "Fat Bass".into(),
            plugin_name: "Mini V4".into(),
            plugin_path: PathBuf::from("C:/x/Mini V4.vst3"),
            class_id: "00112233445566778899AABBCCDDEEFF".into(),
            ..Default::default()
        };
        p.set_state(&[1, 2, 3, 0, 255], &[]);
        p.mapping.x.param_id = Some(42);
        p.fx.delay.enabled = true;
        let s = serde_json::to_string(&p).unwrap();
        let q: Preset = serde_json::from_str(&s).unwrap();
        assert_eq!(p, q);
        assert_eq!(q.state().0, vec![1, 2, 3, 0, 255]);
    }

    #[test]
    fn old_files_get_defaults() {
        let q: Preset = serde_json::from_str(r#"{"name":"x"}"#).unwrap();
        assert_eq!(q.fx.gain, 1.0);
        assert_eq!(q.mapping.sensitivity, Mapping::default().sensitivity);
    }

    #[test]
    fn file_names_are_safe() {
        assert_eq!(file_name("a/b:c"), "a_b_c.json");
    }
}
