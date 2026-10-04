//! Discovery of VST3 instruments on disk.
//!
//! Scanning only walks directories and reads `moduleinfo.json` where available, so it is fast and
//! never loads plugin code. Bundles without module info are listed as well (their kind is unknown
//! until loaded).

use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct PluginEntry {
    /// Display name (bundle file name without extension, or the class name from moduleinfo).
    pub name: String,
    pub vendor: String,
    /// Path to the `.vst3` bundle or file, as the user sees it.
    pub bundle: PathBuf,
    /// Path to the loadable binary inside the bundle.
    pub binary: PathBuf,
    /// `Some(true)` if moduleinfo says it's an instrument, `None` if unknown.
    pub is_instrument: Option<bool>,
}

pub fn default_folders() -> Vec<PathBuf> {
    let mut v = Vec::new();
    #[cfg(windows)]
    {
        if let Some(p) = std::env::var_os("CommonProgramFiles") {
            v.push(PathBuf::from(p).join("VST3"));
        }
        if let Some(p) = dirs::data_local_dir() {
            v.push(p.join("Programs").join("Common").join("VST3"));
        }
    }
    #[cfg(target_os = "linux")]
    {
        if let Some(h) = dirs::home_dir() {
            v.push(h.join(".vst3"));
        }
        v.push(PathBuf::from("/usr/lib/vst3"));
        v.push(PathBuf::from("/usr/local/lib/vst3"));
    }
    v
}

pub fn scan(extra: &[PathBuf]) -> Vec<PluginEntry> {
    let mut out = Vec::new();
    let mut folders = default_folders();
    folders.extend(extra.iter().cloned());
    for f in &folders {
        walk(f, 0, &mut out);
    }
    // The same plugin can be reachable through several folders.
    out.sort_by_key(|a| a.name.to_lowercase());
    out.dedup_by(|a, b| a.bundle == b.bundle);
    out
}

fn walk(dir: &Path, depth: usize, out: &mut Vec<PluginEntry>) {
    if depth > 6 {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let path = e.path();
        let is_vst3 = path
            .extension()
            .is_some_and(|x| x.eq_ignore_ascii_case("vst3"));
        if is_vst3 {
            if let Some(entry) = entry_for(&path) {
                out.push(entry);
            }
        } else if path.is_dir() {
            walk(&path, depth + 1, out);
        }
    }
}

fn entry_for(bundle: &Path) -> Option<PluginEntry> {
    let stem = bundle.file_stem()?.to_string_lossy().into_owned();
    let binary = binary_path(bundle)?;
    let mut entry = PluginEntry {
        name: stem,
        vendor: String::new(),
        bundle: bundle.to_path_buf(),
        binary,
        is_instrument: None,
    };
    if bundle.is_dir() {
        let info = bundle
            .join("Contents")
            .join("Resources")
            .join("moduleinfo.json");
        if let Ok(text) = std::fs::read_to_string(info) {
            apply_module_info(&mut entry, &text);
        }
    }
    Some(entry)
}

/// Resolves the shared library inside a bundle (or the file itself for single-file plugins).
pub fn binary_path(bundle: &Path) -> Option<PathBuf> {
    if bundle.is_file() {
        return Some(bundle.to_path_buf());
    }
    let stem = bundle.file_stem()?;
    #[cfg(windows)]
    let p = bundle
        .join("Contents")
        .join("x86_64-win")
        .join(Path::new(stem).with_extension("vst3"));
    #[cfg(target_os = "linux")]
    let p = bundle
        .join("Contents")
        .join("x86_64-linux")
        .join(Path::new(stem).with_extension("so"));
    p.is_file().then_some(p)
}

fn apply_module_info(entry: &mut PluginEntry, text: &str) {
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&strip_trailing_commas(text)) else {
        return;
    };
    let Some(classes) = json.get("Classes").and_then(|c| c.as_array()) else {
        return;
    };
    let mut any_audio = false;
    for c in classes {
        if c.get("Category").and_then(|v| v.as_str()) != Some("Audio Module Class") {
            continue;
        }
        any_audio = true;
        let subs = c
            .get("Sub Categories")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join("|")
            })
            .unwrap_or_default();
        if subs.contains("Instrument") {
            if let Some(n) = c.get("Name").and_then(|v| v.as_str()) {
                entry.name = n.to_string();
            }
            if let Some(v) = c.get("Vendor").and_then(|v| v.as_str()) {
                entry.vendor = v.to_string();
            }
            entry.is_instrument = Some(true);
            return;
        }
    }
    if any_audio {
        entry.is_instrument = Some(false);
    }
}

/// moduleinfo.json is written by Steinberg tooling that tolerates trailing commas.
fn strip_trailing_commas(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let chars: Vec<char> = s.chars().collect();
    let mut in_str = false;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if in_str {
            out.push(c);
            if c == '\\' && i + 1 < chars.len() {
                out.push(chars[i + 1]);
                i += 1;
            } else if c == '"' {
                in_str = false;
            }
        } else if c == '"' {
            in_str = true;
            out.push(c);
        } else if c == ',' {
            let next = chars[i + 1..].iter().find(|c| !c.is_whitespace());
            if !matches!(next, Some('}') | Some(']')) {
                out.push(c);
            }
        } else {
            out.push(c);
        }
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trailing_commas() {
        let s = r#"{"a": [1, 2,], "b": "x,}",}"#;
        let v: serde_json::Value = serde_json::from_str(&strip_trailing_commas(s)).unwrap();
        assert_eq!(v["b"], "x,}");
        assert_eq!(v["a"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn module_info_instrument() {
        let mut e = PluginEntry {
            name: "x".into(),
            vendor: String::new(),
            bundle: PathBuf::new(),
            binary: PathBuf::new(),
            is_instrument: None,
        };
        let s = r#"{"Classes": [
            {"Category": "Component Controller Class", "Name": "Ctl"},
            {"Category": "Audio Module Class", "Name": "Synth", "Vendor": "V",
             "Sub Categories": ["Instrument", "Synth"]},
        ]}"#;
        apply_module_info(&mut e, s);
        assert_eq!(e.is_instrument, Some(true));
        assert_eq!(e.name, "Synth");
    }
}
