//! Freeverb-style reverb: 8 parallel damped combs into 4 series allpasses per channel.

use super::ReverbSettings;

const COMB_TUNING: [usize; 8] = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617];
const ALLPASS_TUNING: [usize; 4] = [556, 441, 341, 225];
const STEREO_SPREAD: usize = 23;
const FIXED_GAIN: f32 = 0.015;
const MAX_PREDELAY_MS: f32 = 200.0;

struct Comb {
    buf: Vec<f32>,
    pos: usize,
    store: f32,
}

impl Comb {
    fn new(len: usize) -> Self {
        Comb {
            buf: vec![0.0; len.max(1)],
            pos: 0,
            store: 0.0,
        }
    }
    #[inline]
    fn tick(&mut self, input: f32, feedback: f32, damp: f32) -> f32 {
        let out = self.buf[self.pos];
        self.store = out * (1.0 - damp) + self.store * damp;
        self.buf[self.pos] = input + self.store * feedback;
        self.pos = (self.pos + 1) % self.buf.len();
        out
    }
}

struct Allpass {
    buf: Vec<f32>,
    pos: usize,
}

impl Allpass {
    fn new(len: usize) -> Self {
        Allpass {
            buf: vec![0.0; len.max(1)],
            pos: 0,
        }
    }
    #[inline]
    fn tick(&mut self, input: f32) -> f32 {
        let b = self.buf[self.pos];
        self.buf[self.pos] = input + b * 0.5;
        self.pos = (self.pos + 1) % self.buf.len();
        b - input
    }
}

pub struct Reverb {
    sr: f32,
    combs: [Vec<Comb>; 2],
    allpasses: [Vec<Allpass>; 2],
    pre: Vec<f32>,
    pre_pos: usize,
}

impl Reverb {
    pub fn new(sample_rate: f32) -> Self {
        let k = sample_rate / 44100.0;
        let scale = |n: usize| ((n as f32) * k) as usize;
        let combs = [
            COMB_TUNING.iter().map(|&n| Comb::new(scale(n))).collect(),
            COMB_TUNING
                .iter()
                .map(|&n| Comb::new(scale(n + STEREO_SPREAD)))
                .collect(),
        ];
        let allpasses = [
            ALLPASS_TUNING
                .iter()
                .map(|&n| Allpass::new(scale(n)))
                .collect(),
            ALLPASS_TUNING
                .iter()
                .map(|&n| Allpass::new(scale(n + STEREO_SPREAD)))
                .collect(),
        ];
        Reverb {
            sr: sample_rate,
            combs,
            allpasses,
            pre: vec![0.0; (sample_rate * MAX_PREDELAY_MS / 1000.0) as usize + 1],
            pre_pos: 0,
        }
    }

    pub fn reset(&mut self) {
        for ch in &mut self.combs {
            for c in ch {
                c.buf.fill(0.0);
                c.store = 0.0;
            }
        }
        for ch in &mut self.allpasses {
            for a in ch {
                a.buf.fill(0.0);
            }
        }
        self.pre.fill(0.0);
    }

    pub fn process(&mut self, s: &ReverbSettings, left: &mut [f32], right: &mut [f32]) {
        let feedback = s.size.clamp(0.0, 1.0) * 0.28 + 0.7;
        let damp = s.damping.clamp(0.0, 1.0) * 0.4;
        let mix = s.mix.clamp(0.0, 1.0);
        let wet = mix * 3.0;
        let width = s.width.clamp(0.0, 1.0);
        let wet1 = wet * (width / 2.0 + 0.5);
        let wet2 = wet * ((1.0 - width) / 2.0);
        let dry = 1.0 - mix * 0.5;
        let pre_len = self.pre.len();
        let pre_delay = ((s.predelay_ms.clamp(0.0, MAX_PREDELAY_MS) / 1000.0 * self.sr) as usize)
            .min(pre_len - 1);

        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            // Mono send through the pre-delay line.
            self.pre[self.pre_pos] = (*l + *r) * FIXED_GAIN;
            let input = self.pre[(self.pre_pos + pre_len - pre_delay) % pre_len];
            self.pre_pos = (self.pre_pos + 1) % pre_len;

            let mut out = [0.0f32; 2];
            for ((o, combs), allpasses) in
                out.iter_mut().zip(&mut self.combs).zip(&mut self.allpasses)
            {
                let mut acc = 0.0;
                for c in combs {
                    acc += c.tick(input, feedback, damp);
                }
                for a in allpasses {
                    acc = a.tick(acc);
                }
                *o = acc;
            }
            let wl = out[0] * wet1 + out[1] * wet2;
            let wr = out[1] * wet1 + out[0] * wet2;
            *l = *l * dry + wl;
            *r = *r * dry + wr;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn impulse_tail_is_finite_and_decays() {
        let sr = 48000.0;
        let mut rv = Reverb::new(sr);
        let s = ReverbSettings {
            enabled: true,
            mix: 1.0,
            ..Default::default()
        };
        let n = 48000 * 4;
        let mut l = vec![0.0; n];
        let mut r = vec![0.0; n];
        l[0] = 1.0;
        r[0] = 1.0;
        rv.process(&s, &mut l, &mut r);
        assert!(l.iter().chain(r.iter()).all(|x| x.is_finite()));
        let energy = |s: &[f32]| s.iter().map(|x| x * x).sum::<f32>();
        let early = energy(&l[4800..48000]);
        let late = energy(&l[n - 43200..]);
        assert!(early > 0.0);
        assert!(late < early * 0.1, "early {early} late {late}");
    }
}
