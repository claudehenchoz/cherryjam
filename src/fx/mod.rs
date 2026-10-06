//! Built-in effects applied after the instrument.

pub mod delay;
pub mod reverb;

use serde::{Deserialize, Serialize};

use crate::tempo::Division;

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
    /// Follow the global tempo: the time is `division` instead of `time_ms`.
    pub sync: bool,
    pub division: Division,
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
            sync: false,
            division: Division::EighthDotted,
        }
    }
}

impl DelaySettings {
    /// Delay time in ms, taking tempo sync into account.
    pub fn effective_time_ms(&self, bpm: f32) -> f32 {
        if self.sync {
            ((self.division.seconds(bpm) * 1000.0) as f32).min(delay::MAX_SECONDS * 1000.0)
        } else {
            self.time_ms
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
    /// Follow the global tempo: pre-delay is `predelay_division` instead of `predelay_ms`.
    pub predelay_sync: bool,
    pub predelay_division: Division,
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
            predelay_sync: false,
            predelay_division: Division::SixtyFourth,
        }
    }
}

impl ReverbSettings {
    /// Pre-delay in ms, taking tempo sync into account.
    pub fn effective_predelay_ms(&self, bpm: f32) -> f32 {
        if self.predelay_sync {
            ((self.predelay_division.seconds(bpm) * 1000.0) as f32).min(reverb::MAX_PREDELAY_MS)
        } else {
            self.predelay_ms
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
    fn synced_times_follow_tempo() {
        let d = DelaySettings {
            sync: true,
            division: Division::EighthDotted,
            ..Default::default()
        };
        assert_eq!(d.effective_time_ms(120.0), 375.0);
        assert_eq!(d.effective_time_ms(100.0), 450.0);
        // Long divisions at slow tempos are capped at the buffer length.
        let d = DelaySettings {
            sync: true,
            division: Division::Quarter,
            ..d
        };
        assert_eq!(d.effective_time_ms(20.0), 1500.0); // tempo clamps to 40 BPM
        let unsynced = DelaySettings {
            time_ms: 123.0,
            ..Default::default()
        };
        assert_eq!(unsynced.effective_time_ms(90.0), 123.0);
        let r = ReverbSettings {
            predelay_sync: true,
            predelay_division: Division::Sixteenth,
            ..Default::default()
        };
        assert_eq!(r.effective_predelay_ms(120.0), 125.0);
        assert_eq!(r.effective_predelay_ms(60.0), 200.0); // capped
    }

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
