//! Arpeggiator. Runs on the audio thread between note input and the plugin so steps land
//! sample-accurately inside each processing block.
//!
//! The engine sees every note-on/off, also while disabled (then it just passes notes through),
//! so switching it on or off mid-chord never leaves notes hanging or missing.

use std::sync::atomic::{AtomicI32, AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

use crate::tempo::Division;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mode {
    Up,
    Down,
    UpDown,
    DownUp,
    AsPlayed,
    Random,
    Chord,
}

impl Mode {
    pub const ALL: [Mode; 7] = [
        Mode::Up,
        Mode::Down,
        Mode::UpDown,
        Mode::DownUp,
        Mode::AsPlayed,
        Mode::Random,
        Mode::Chord,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Mode::Up => "Up",
            Mode::Down => "Down",
            Mode::UpDown => "Up ↕ Down",
            Mode::DownUp => "Down ↕ Up",
            Mode::AsPlayed => "As played",
            Mode::Random => "Random",
            Mode::Chord => "Chord",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ArpSettings {
    pub enabled: bool,
    /// Keep playing after the keys are released; the next chord replaces the old one.
    pub latch: bool,
    pub rate: Division,
    pub mode: Mode,
    pub octaves: u8,
    /// Note length as a fraction of the step (1.0 = legato).
    pub gate: f32,
    /// 0 = straight, 0.5 = maximum shuffle (every second step pushed late by half a step).
    pub swing: f32,
    /// Pattern length in steps (1..=16).
    pub steps: u8,
    /// Bit `i` set = step `i` plays; cleared = rest.
    pub pattern: u16,
    /// Restart the pattern from step 1 whenever a new chord starts.
    pub retrigger: bool,
}

impl Default for ArpSettings {
    fn default() -> Self {
        ArpSettings {
            enabled: false,
            latch: false,
            rate: Division::Sixteenth,
            mode: Mode::Up,
            octaves: 1,
            gate: 0.6,
            swing: 0.0,
            steps: 16,
            pattern: u16::MAX,
            retrigger: true,
        }
    }
}

impl ArpSettings {
    pub fn step_on(&self, step: usize) -> bool {
        self.pattern & (1 << (step % 16)) != 0
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum NoteEvent {
    On { pitch: u8, velocity: f32 },
    Off { pitch: u8 },
}

/// Live state for the UI: current pattern step (-1 = idle) and the notes the arp is sounding.
pub static ARP_STEP: AtomicI32 = AtomicI32::new(-1);
static ARP_NOTES: [AtomicU64; 2] = [AtomicU64::new(0), AtomicU64::new(0)];

pub fn arp_note_on(pitch: u8) -> bool {
    let p = pitch as usize & 127;
    ARP_NOTES[p / 64].load(Ordering::Relaxed) & (1 << (p % 64)) != 0
}

fn set_arp_note(pitch: u8, on: bool) {
    let p = pitch as usize & 127;
    if on {
        ARP_NOTES[p / 64].fetch_or(1 << (p % 64), Ordering::Relaxed);
    } else {
        ARP_NOTES[p / 64].fetch_and(!(1 << (p % 64)), Ordering::Relaxed);
    }
}

const CAP: usize = 128;

pub struct Arp {
    s: ArpSettings,
    /// Keys physically held, in press order.
    held: Vec<(u8, f32)>,
    /// The latched chord (used instead of `held` while latch is on).
    latched: Vec<(u8, f32)>,
    /// Notes currently sounding because of pass-through (arp off).
    direct: [bool; 128],
    /// Notes currently sounding from the arp.
    sounding: Vec<u8>,
    /// Events to emit at the start of the next block (input reactions, setting changes).
    pending: Vec<NoteEvent>,
    seq: Vec<(u8, f32)>,
    running: bool,
    /// Samples until the next step / until the sounding notes are released.
    to_step: f64,
    to_off: Option<f64>,
    /// Pattern step counter (rests included) and note-order position (rests excluded).
    step: usize,
    pos: usize,
    rng: u32,
}

impl Default for Arp {
    fn default() -> Self {
        Arp {
            s: ArpSettings::default(),
            held: Vec::with_capacity(CAP),
            latched: Vec::with_capacity(CAP),
            direct: [false; 128],
            sounding: Vec::with_capacity(CAP * 4),
            pending: Vec::with_capacity(CAP * 4),
            seq: Vec::with_capacity(CAP * 4),
            running: false,
            to_step: 0.0,
            to_off: None,
            step: 0,
            pos: 0,
            rng: 0x9E37_79B9,
        }
    }
}

impl Arp {
    fn active(&self) -> &[(u8, f32)] {
        if self.s.latch {
            &self.latched
        } else {
            &self.held
        }
    }

    fn start(&mut self) {
        if !self.running || self.s.retrigger {
            self.step = 0;
            self.pos = 0;
            self.to_step = 0.0;
        }
        self.running = true;
    }

    fn stop(&mut self) {
        self.release_sounding();
        self.running = false;
        self.to_off = None;
        ARP_STEP.store(-1, Ordering::Relaxed);
    }

    fn release_sounding(&mut self) {
        for p in self.sounding.drain(..) {
            self.pending.push(NoteEvent::Off { pitch: p });
            set_arp_note(p, false);
        }
    }

    pub fn note_on(&mut self, pitch: u8, velocity: f32) {
        let was_empty = self.active().is_empty();
        if self.s.latch && self.held.is_empty() {
            // A fresh gesture replaces the latched chord.
            self.latched.clear();
        }
        self.held.retain(|&(p, _)| p != pitch);
        if self.held.len() < CAP {
            self.held.push((pitch, velocity));
        }
        if self.s.latch
            && !self.latched.iter().any(|&(p, _)| p == pitch)
            && self.latched.len() < CAP
        {
            self.latched.push((pitch, velocity));
        }
        if !self.s.enabled {
            if self.direct[pitch as usize & 127] {
                self.pending.push(NoteEvent::Off { pitch });
            }
            self.direct[pitch as usize & 127] = true;
            self.pending.push(NoteEvent::On { pitch, velocity });
        } else if was_empty || (self.s.latch && self.latched.len() == 1) {
            self.start();
        }
    }

    pub fn note_off(&mut self, pitch: u8) {
        self.held.retain(|&(p, _)| p != pitch);
        if !self.s.enabled {
            if std::mem::take(&mut self.direct[pitch as usize & 127]) {
                self.pending.push(NoteEvent::Off { pitch });
            }
        } else if self.active().is_empty() {
            self.stop();
        }
    }

    pub fn set_settings(&mut self, mut n: ArpSettings) {
        n.octaves = n.octaves.clamp(1, 4);
        n.steps = n.steps.clamp(1, 16);
        n.gate = n.gate.clamp(0.05, 1.0);
        n.swing = n.swing.clamp(0.0, 0.5);
        let old = self.s;
        if n.latch && !old.latch {
            self.latched.clear();
            self.latched.extend_from_slice(&self.held);
        }
        self.s = n;

        if n.enabled && !old.enabled {
            for p in 0..128u8 {
                if std::mem::take(&mut self.direct[p as usize]) {
                    self.pending.push(NoteEvent::Off { pitch: p });
                }
            }
            if !self.active().is_empty() {
                self.running = false;
                self.start();
            }
        } else if !n.enabled && old.enabled {
            self.stop();
            for i in 0..self.held.len() {
                let (pitch, velocity) = self.held[i];
                self.direct[pitch as usize] = true;
                self.pending.push(NoteEvent::On { pitch, velocity });
            }
        } else if n.enabled && self.active().is_empty() && self.running {
            // Latch switched off with no keys down.
            self.stop();
        }
    }

    fn next_random(&mut self) -> u32 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        x
    }

    /// Fills `seq` with the notes to cycle through: sorted (except "as played"), octave-expanded.
    fn build_seq(&mut self) {
        self.seq.clear();
        let base: &[(u8, f32)] = if self.s.latch {
            &self.latched
        } else {
            &self.held
        };
        let start = self.seq.len();
        self.seq.extend_from_slice(base);
        if self.s.mode != Mode::AsPlayed {
            self.seq[start..].sort_by_key(|&(p, _)| p);
        }
        let n = self.seq.len();
        for o in 1..self.s.octaves as usize {
            for i in 0..n {
                let (p, v) = self.seq[i];
                let q = p as usize + 12 * o;
                if q <= 127 {
                    self.seq.push((q as u8, v));
                }
            }
        }
    }

    fn step_length(&self, samples_per_beat: f64) -> f64 {
        let base = self.s.rate.beats() * samples_per_beat;
        let swing = self.s.swing as f64;
        if self.step.is_multiple_of(2) {
            base * (1.0 + swing)
        } else {
            base * (1.0 - swing)
        }
    }

    /// Plays one step at the current position; returns its length in samples.
    fn do_step(
        &mut self,
        samples_per_beat: f64,
        at: u32,
        emit: &mut impl FnMut(u32, NoteEvent),
    ) -> f64 {
        let len = self.step_length(samples_per_beat).max(1.0);
        let pattern_step = self.step % self.s.steps as usize;
        ARP_STEP.store(pattern_step as i32, Ordering::Relaxed);
        // Anything still sounding (gate rounding) ends before the new step starts.
        for p in self.sounding.drain(..) {
            emit(at, NoteEvent::Off { pitch: p });
            set_arp_note(p, false);
        }
        self.to_off = None;
        if self.s.step_on(pattern_step) {
            self.build_seq();
            let n = self.seq.len();
            if n > 0 {
                let mut play = |arp: &mut Arp, (pitch, velocity): (u8, f32)| {
                    if !arp.sounding.contains(&pitch) {
                        emit(at, NoteEvent::On { pitch, velocity });
                        arp.sounding.push(pitch);
                        set_arp_note(pitch, true);
                    }
                };
                match self.s.mode {
                    Mode::Chord => {
                        for i in 0..n {
                            let note = self.seq[i];
                            play(self, note);
                        }
                    }
                    Mode::Random => {
                        let i = self.next_random() as usize % n;
                        let note = self.seq[i];
                        play(self, note);
                    }
                    mode => {
                        let i = index_for(mode, self.pos, n);
                        let note = self.seq[i];
                        play(self, note);
                    }
                }
                self.pos = self.pos.wrapping_add(1);
                let gate = self.s.gate as f64;
                // Legato: release exactly when the next step starts (offs are handled first).
                self.to_off = Some(if gate >= 1.0 {
                    len
                } else {
                    (len * gate).max(1.0)
                });
            }
        }
        self.step = self.step.wrapping_add(1);
        len
    }

    /// Emits this block's events (sample offsets within `0..frames`, in time order).
    pub fn process(
        &mut self,
        frames: usize,
        sample_rate: f64,
        bpm: f32,
        mut emit: impl FnMut(u32, NoteEvent),
    ) {
        for e in self.pending.drain(..) {
            emit(0, e);
        }
        if !self.s.enabled || !self.running {
            return;
        }
        let samples_per_beat =
            sample_rate * 60.0 / bpm.clamp(crate::tempo::MIN_BPM, crate::tempo::MAX_BPM) as f64;
        let frames_f = frames as f64;
        let mut t = 0.0f64;
        loop {
            let dt = self
                .to_off
                .map_or(self.to_step, |o| o.min(self.to_step))
                .max(0.0);
            if t + dt >= frames_f {
                let rest = frames_f - t;
                self.to_step -= rest;
                if let Some(o) = &mut self.to_off {
                    *o -= rest;
                }
                break;
            }
            t += dt;
            self.to_step -= dt;
            let at = (t as u32).min(frames as u32 - 1);
            if let Some(o) = self.to_off.map(|o| o - dt)
                && o <= 1e-9
            {
                for p in self.sounding.drain(..) {
                    emit(at, NoteEvent::Off { pitch: p });
                    set_arp_note(p, false);
                }
                self.to_off = None;
                continue;
            }
            if let Some(o) = &mut self.to_off {
                *o -= dt;
            }
            if self.to_step <= 1e-9 {
                self.to_step = self.do_step(samples_per_beat, at, &mut emit);
            }
        }
    }
}

/// Note index for the ordered modes at sequence position `pos` with `n` notes.
fn index_for(mode: Mode, pos: usize, n: usize) -> usize {
    let bounce = |pos: usize| {
        if n < 2 {
            return 0;
        }
        let cycle = 2 * n - 2;
        let i = pos % cycle;
        if i < n { i } else { cycle - i }
    };
    match mode {
        Mode::Down => n - 1 - pos % n,
        Mode::UpDown => bounce(pos),
        Mode::DownUp => n - 1 - bounce(pos),
        _ => pos % n,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f64 = 48000.0;

    fn run(arp: &mut Arp, blocks: usize, frames: usize) -> Vec<(u64, NoteEvent)> {
        let mut out = Vec::new();
        for b in 0..blocks {
            arp.process(frames, SR, 120.0, |off, e| {
                out.push(((b * frames) as u64 + off as u64, e))
            });
        }
        out
    }

    fn ons(ev: &[(u64, NoteEvent)]) -> Vec<(u64, u8)> {
        ev.iter()
            .filter_map(|&(t, e)| match e {
                NoteEvent::On { pitch, .. } => Some((t, pitch)),
                _ => None,
            })
            .collect()
    }

    fn enabled(f: impl FnOnce(&mut ArpSettings)) -> Arp {
        let mut a = Arp::default();
        let mut s = ArpSettings {
            enabled: true,
            ..Default::default()
        };
        f(&mut s);
        a.set_settings(s);
        a
    }

    /// Every note-on is matched by a note-off and no note is switched on twice.
    fn assert_balanced(ev: &[(u64, NoteEvent)]) {
        let mut on = [false; 128];
        for &(_, e) in ev {
            match e {
                NoteEvent::On { pitch, .. } => {
                    assert!(!on[pitch as usize], "double on {pitch}");
                    on[pitch as usize] = true;
                }
                NoteEvent::Off { pitch } => on[pitch as usize] = false,
            }
        }
        assert!(on.iter().all(|&x| !x), "hanging notes");
    }

    #[test]
    fn mode_orders() {
        assert_eq!(
            (0..6)
                .map(|p| index_for(Mode::Up, p, 3))
                .collect::<Vec<_>>(),
            [0, 1, 2, 0, 1, 2]
        );
        assert_eq!(
            (0..6)
                .map(|p| index_for(Mode::Down, p, 3))
                .collect::<Vec<_>>(),
            [2, 1, 0, 2, 1, 0]
        );
        assert_eq!(
            (0..8)
                .map(|p| index_for(Mode::UpDown, p, 3))
                .collect::<Vec<_>>(),
            [0, 1, 2, 1, 0, 1, 2, 1]
        );
        assert_eq!(
            (0..8)
                .map(|p| index_for(Mode::DownUp, p, 3))
                .collect::<Vec<_>>(),
            [2, 1, 0, 1, 2, 1, 0, 1]
        );
        assert_eq!(index_for(Mode::UpDown, 5, 1), 0);
    }

    #[test]
    fn sixteenths_at_120_bpm_across_blocks() {
        let mut a = enabled(|_| {});
        a.note_on(64, 0.8);
        a.note_on(60, 0.8);
        a.note_on(67, 0.8);
        // 1/16 at 120 BPM = 0.125 s = 6000 samples; blocks of 256 don't divide that evenly.
        let ev = run(&mut a, 100, 256);
        let o = ons(&ev);
        assert_eq!(&o[..4], &[(0, 60), (6000, 64), (12000, 67), (18000, 60)]);
    }

    #[test]
    fn as_played_and_octaves() {
        let mut a = enabled(|s| {
            s.mode = Mode::AsPlayed;
            s.octaves = 2;
        });
        a.note_on(64, 0.8);
        a.note_on(60, 0.8);
        let o: Vec<u8> = ons(&run(&mut a, 120, 256))
            .iter()
            .map(|&(_, p)| p)
            .take(5)
            .collect();
        assert_eq!(o, [64, 60, 76, 72, 64]);
    }

    #[test]
    fn gate_and_legato() {
        let mut a = enabled(|s| s.gate = 0.5);
        a.note_on(60, 0.8);
        let ev = run(&mut a, 30, 512);
        assert_eq!(
            ev[0],
            (
                0,
                NoteEvent::On {
                    pitch: 60,
                    velocity: 0.8
                }
            )
        );
        assert_eq!(ev[1], (3000, NoteEvent::Off { pitch: 60 }));

        let mut a = enabled(|s| s.gate = 1.0);
        a.note_on(60, 0.8);
        a.note_on(62, 0.8);
        let ev = run(&mut a, 30, 512);
        // The off of the first note comes right before the next on, at the same sample.
        assert_eq!(ev[1], (6000, NoteEvent::Off { pitch: 60 }));
        assert_eq!(
            ev[2],
            (
                6000,
                NoteEvent::On {
                    pitch: 62,
                    velocity: 0.8
                }
            )
        );
    }

    #[test]
    fn swing_pushes_every_second_step() {
        let mut a = enabled(|s| s.swing = 0.25);
        a.note_on(60, 0.8);
        let o = ons(&run(&mut a, 60, 512));
        assert_eq!(o[1].0, 7500); // 6000 * 1.25
        assert_eq!(o[2].0, 12000); // pair length unchanged
    }

    #[test]
    fn pattern_rests() {
        let mut a = enabled(|s| {
            s.steps = 4;
            s.pattern = 0b0101;
        });
        a.note_on(60, 0.8);
        a.note_on(64, 0.8);
        let o = ons(&run(&mut a, 200, 512));
        // Steps 0 and 2 play; the note order is not advanced by rests.
        assert_eq!(&o[..3], &[(0, 60), (12000, 64), (24000, 60)]);
    }

    #[test]
    fn latch_keeps_playing_and_new_chord_replaces() {
        let mut a = enabled(|s| s.latch = true);
        a.note_on(60, 0.8);
        a.note_off(60);
        let o = ons(&run(&mut a, 60, 512));
        assert!(o.len() >= 3 && o.iter().all(|&(_, p)| p == 60));
        a.note_on(67, 0.8);
        let o = ons(&run(&mut a, 60, 512));
        assert!(o.iter().all(|&(_, p)| p == 67));
    }

    #[test]
    fn release_stops_and_nothing_hangs() {
        let mut a = enabled(|s| s.gate = 1.0);
        a.note_on(60, 0.8);
        a.note_on(64, 0.8);
        let mut ev = run(&mut a, 20, 512);
        a.note_off(60);
        a.note_off(64);
        ev.extend(run(&mut a, 20, 512));
        assert_balanced(&ev);
    }

    #[test]
    fn disabled_passes_through() {
        let mut a = Arp::default();
        a.note_on(60, 0.7);
        a.note_off(60);
        let ev = run(&mut a, 1, 512);
        assert_eq!(
            ev,
            [
                (
                    0,
                    NoteEvent::On {
                        pitch: 60,
                        velocity: 0.7
                    }
                ),
                (0, NoteEvent::Off { pitch: 60 })
            ]
        );
    }

    #[test]
    fn toggling_mid_chord_is_clean() {
        let mut a = Arp::default();
        a.note_on(60, 0.8);
        a.note_on(64, 0.8);
        let mut ev = run(&mut a, 2, 512);
        let mut s = ArpSettings {
            enabled: true,
            ..Default::default()
        };
        a.set_settings(s);
        ev.extend(run(&mut a, 30, 512));
        s.enabled = false;
        a.set_settings(s);
        ev.extend(run(&mut a, 2, 512));
        // Held notes sound directly again after switching off.
        let last_ons: Vec<u8> = ons(&ev[ev.len() - 2..]).iter().map(|&(_, p)| p).collect();
        assert_eq!(last_ons, [60, 64]);
        a.note_off(60);
        a.note_off(64);
        ev.extend(run(&mut a, 1, 512));
        assert_balanced(&ev);
    }

    #[test]
    fn chord_mode_plays_all_notes() {
        let mut a = enabled(|s| s.mode = Mode::Chord);
        a.note_on(60, 0.8);
        a.note_on(64, 0.8);
        a.note_on(67, 0.8);
        let o = ons(&run(&mut a, 1, 512));
        assert_eq!(o, [(0, 60), (0, 64), (0, 67)]);
    }
}
