//! XY mouse controller: while Caps Lock is on, mouse movement drives two mapped parameters.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AxisMap {
    pub param_id: Option<u32>,
    /// Parameter title, kept for display and as a fallback when ids change.
    pub param_name: String,
    pub min: f32,
    pub max: f32,
    pub invert: bool,
}

impl Default for AxisMap {
    fn default() -> Self {
        AxisMap {
            param_id: None,
            param_name: String::new(),
            min: 0.0,
            max: 1.0,
            invert: false,
        }
    }
}

impl AxisMap {
    /// Maps a controller position in 0..1 to the parameter's normalized value.
    pub fn value(&self, pos: f32) -> f64 {
        let p = if self.invert { 1.0 - pos } else { pos };
        (self.min + (self.max - self.min) * p.clamp(0.0, 1.0)) as f64
    }

    /// Inverse of [`value`]: where the controller should start for a given parameter value.
    pub fn position(&self, value: f64) -> f32 {
        let span = self.max - self.min;
        let p = if span.abs() < 1e-6 {
            0.5
        } else {
            ((value as f32 - self.min) / span).clamp(0.0, 1.0)
        };
        if self.invert { 1.0 - p } else { p }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Mapping {
    pub x: AxisMap,
    pub y: AxisMap,
    /// Fraction of the full range covered per 1000 physical pixels of mouse travel.
    pub sensitivity: f32,
}

impl Default for Mapping {
    fn default() -> Self {
        Mapping {
            x: AxisMap::default(),
            y: AxisMap::default(),
            sensitivity: 1.5,
        }
    }
}

/// Live controller state.
#[derive(Default)]
pub struct XyState {
    pub engaged: bool,
    pub pos: [f32; 2],
}

impl XyState {
    /// Applies a raw mouse delta (pixels). Mouse up = Y increases. Returns true if it moved.
    pub fn apply_delta(&mut self, mapping: &Mapping, dx: f32, dy: f32) -> bool {
        let k = mapping.sensitivity / 1000.0;
        let old = self.pos;
        self.pos[0] = (self.pos[0] + dx * k).clamp(0.0, 1.0);
        self.pos[1] = (self.pos[1] - dy * k).clamp(0.0, 1.0);
        old != self.pos
    }
}

/// Current Caps Lock toggle state, read from the OS.
#[cfg(windows)]
pub fn caps_lock_on() -> bool {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_CAPITAL};
    unsafe { GetKeyState(VK_CAPITAL as i32) & 1 != 0 }
}

#[cfg(target_os = "linux")]
pub fn caps_lock_on() -> bool {
    use std::sync::OnceLock;
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{ConnectionExt, KeyButMask};
    static CONN: OnceLock<Option<(x11rb::rust_connection::RustConnection, u32)>> = OnceLock::new();
    let Some((c, root)) = CONN
        .get_or_init(|| {
            x11rb::connect(None).ok().map(|(c, screen)| {
                let root = c.setup().roots[screen].root;
                (c, root)
            })
        })
        .as_ref()
    else {
        return false;
    };
    c.query_pointer(*root)
        .ok()
        .and_then(|cookie| cookie.reply().ok())
        .is_some_and(|r| u16::from(r.mask) & u16::from(KeyButMask::LOCK) != 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn axis_roundtrip() {
        let a = AxisMap {
            param_id: Some(1),
            min: 0.2,
            max: 0.8,
            invert: true,
            ..Default::default()
        };
        for i in 0..=10 {
            let p = i as f32 / 10.0;
            assert!((a.position(a.value(p)) - p).abs() < 1e-5);
        }
        assert!((a.value(0.0) - 0.8).abs() < 1e-6);
    }

    #[test]
    fn delta_clamps() {
        let m = Mapping::default();
        let mut s = XyState::default();
        s.apply_delta(&m, 1e6, -1e6);
        assert_eq!(s.pos, [1.0, 1.0]);
    }
}
