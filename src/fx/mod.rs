//! Built-in effects applied after the instrument.

pub mod delay;
pub mod reverb;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DelaySettings {
    pub enabled: bool,
    pub time_ms: f32,
    pub feedback: f32,
    /// Low-pass cutoff in the feedback loop, Hz.
    pub tone_hz: f32,
    pub mix: f32,
    pub ping_pong: bool,
}

impl Default for DelaySettings {
    fn default() -> Self {
        DelaySettings {
            enabled: false,
            time_ms: 375.0,
            feedback: 0.35,
            tone_hz: 6000.0,
            mix: 0.3,
            ping_pong: true,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ReverbSettings {
    pub enabled: bool,
    pub size: f32,
    pub damping: f32,
    pub width: f32,
    pub predelay_ms: f32,
    pub mix: f32,
}

impl Default for ReverbSettings {
    fn default() -> Self {
        ReverbSettings {
            enabled: false,
            size: 0.6,
            damping: 0.4,
            width: 1.0,
            predelay_ms: 20.0,
            mix: 0.25,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FxSettings {
    pub delay: DelaySettings,
    pub reverb: ReverbSettings,
    /// Master output gain (linear).
    pub gain: f32,
}

impl Default for FxSettings {
    fn default() -> Self {
        FxSettings {
            delay: Default::default(),
            reverb: Default::default(),
            gain: 1.0,
        }
    }
}

/// Soft knee limiter: transparent below 0.9, never exceeds 1.0.
#[inline]
pub fn soft_limit(x: f32) -> f32 {
    let a = x.abs();
    if a <= 0.9 {
        x
    } else {
        x.signum() * (0.9 + 0.1 * ((a - 0.9) / 0.1).tanh())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limiter_bounds() {
        for i in -100..100 {
            let x = i as f32 * 0.1;
            let y = soft_limit(x);
            assert!(y.abs() <= 1.0);
            if x.abs() <= 0.9 {
                assert_eq!(x, y);
            }
        }
    }
}
