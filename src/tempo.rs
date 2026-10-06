//! Musical time: note divisions shared by the arpeggiator and the tempo-synced effects.

use serde::{Deserialize, Serialize};

pub const DEFAULT_BPM: f32 = 120.0;
pub const MIN_BPM: f32 = 40.0;
pub const MAX_BPM: f32 = 240.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Division {
    Whole,
    Half,
    Quarter,
    QuarterDotted,
    QuarterTriplet,
    Eighth,
    EighthDotted,
    EighthTriplet,
    Sixteenth,
    SixteenthDotted,
    SixteenthTriplet,
    ThirtySecond,
    SixtyFourth,
}

impl Division {
    /// Length in quarter-note beats.
    pub fn beats(self) -> f64 {
        use Division::*;
        match self {
            Whole => 4.0,
            Half => 2.0,
            Quarter => 1.0,
            QuarterDotted => 1.5,
            QuarterTriplet => 2.0 / 3.0,
            Eighth => 0.5,
            EighthDotted => 0.75,
            EighthTriplet => 1.0 / 3.0,
            Sixteenth => 0.25,
            SixteenthDotted => 0.375,
            SixteenthTriplet => 1.0 / 6.0,
            ThirtySecond => 0.125,
            SixtyFourth => 0.0625,
        }
    }

    pub fn seconds(self, bpm: f32) -> f64 {
        self.beats() * 60.0 / bpm.clamp(MIN_BPM, MAX_BPM) as f64
    }

    pub fn label(self) -> &'static str {
        use Division::*;
        match self {
            Whole => "1/1",
            Half => "1/2",
            Quarter => "1/4",
            QuarterDotted => "1/4.",
            QuarterTriplet => "1/4T",
            Eighth => "1/8",
            EighthDotted => "1/8.",
            EighthTriplet => "1/8T",
            Sixteenth => "1/16",
            SixteenthDotted => "1/16.",
            SixteenthTriplet => "1/16T",
            ThirtySecond => "1/32",
            SixtyFourth => "1/64",
        }
    }
}

/// Choices offered in the UI for each use.
pub const ARP_RATES: &[Division] = {
    use Division::*;
    &[
        Quarter,
        Eighth,
        EighthTriplet,
        Sixteenth,
        SixteenthTriplet,
        ThirtySecond,
    ]
};

pub const DELAY_DIVISIONS: &[Division] = {
    use Division::*;
    &[
        Quarter,
        QuarterDotted,
        QuarterTriplet,
        Eighth,
        EighthDotted,
        EighthTriplet,
        Sixteenth,
        SixteenthDotted,
        SixteenthTriplet,
    ]
};

pub const PREDELAY_DIVISIONS: &[Division] = {
    use Division::*;
    &[SixtyFourth, ThirtySecond, SixteenthTriplet, Sixteenth]
};

/// Tap tempo: averages the intervals between the last few taps.
#[derive(Default)]
pub struct TapTempo {
    taps: Vec<f64>,
}

impl TapTempo {
    /// Registers a tap at `now` (seconds). Returns a new BPM once there are at least two taps.
    pub fn tap(&mut self, now: f64) -> Option<f32> {
        // A long pause starts a new measurement.
        if self.taps.last().is_some_and(|&t| now - t > 2.0) {
            self.taps.clear();
        }
        self.taps.push(now);
        if self.taps.len() > 5 {
            self.taps.remove(0);
        }
        if self.taps.len() < 2 {
            return None;
        }
        let span = self.taps.last()? - self.taps.first()?;
        let avg = span / (self.taps.len() - 1) as f64;
        Some(((60.0 / avg) as f32).clamp(MIN_BPM, MAX_BPM).round())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn division_lengths() {
        assert!((Division::Quarter.seconds(120.0) - 0.5).abs() < 1e-9);
        assert!((Division::EighthDotted.seconds(120.0) - 0.375).abs() < 1e-9);
        assert!((Division::EighthTriplet.seconds(60.0) - 1.0 / 3.0).abs() < 1e-9);
        assert!((Division::Sixteenth.seconds(90.0) - 60.0 / 90.0 / 4.0).abs() < 1e-9);
    }

    #[test]
    fn tap_tempo() {
        let mut t = TapTempo::default();
        assert_eq!(t.tap(0.0), None);
        assert_eq!(t.tap(0.5), Some(120.0));
        assert_eq!(t.tap(1.0), Some(120.0));
        // After a long pause the measurement restarts.
        assert_eq!(t.tap(10.0), None);
        assert_eq!(t.tap(10.6), Some(100.0));
    }
}
