//! Stereo feedback delay with a low-pass filter in the loop and optional ping-pong.

use super::DelaySettings;

const MAX_SECONDS: f32 = 2.0;

pub struct Delay {
    sr: f32,
    buf: [Vec<f32>; 2],
    pos: usize,
    /// Smoothed delay time in samples (avoids zipper noise when the time changes).
    cur_delay: f32,
    lp: [f32; 2],
}

impl Delay {
    pub fn new(sample_rate: f32) -> Self {
        let len = (sample_rate * MAX_SECONDS) as usize + 2;
        Delay {
            sr: sample_rate,
            buf: [vec![0.0; len], vec![0.0; len]],
            pos: 0,
            cur_delay: 0.0,
            lp: [0.0; 2],
        }
    }

    pub fn reset(&mut self) {
        for b in &mut self.buf {
            b.fill(0.0);
        }
        self.lp = [0.0; 2];
    }

    fn read(&self, ch: usize, delay: f32) -> f32 {
        let len = self.buf[ch].len();
        let d = delay.clamp(1.0, (len - 2) as f32);
        let i = d.floor() as usize;
        let frac = d - i as f32;
        let a = self.buf[ch][(self.pos + len - i) % len];
        let b = self.buf[ch][(self.pos + len - i - 1) % len];
        a + (b - a) * frac
    }

    pub fn process(&mut self, s: &DelaySettings, left: &mut [f32], right: &mut [f32]) {
        let target = (s.time_ms.clamp(1.0, MAX_SECONDS * 1000.0) * 0.001 * self.sr).max(1.0);
        if self.cur_delay == 0.0 {
            self.cur_delay = target;
        }
        let fb = s.feedback.clamp(0.0, 0.95);
        let coef =
            1.0 - (-2.0 * std::f32::consts::PI * s.tone_hz.clamp(200.0, 20000.0) / self.sr).exp();
        let mix = s.mix.clamp(0.0, 1.0);
        let len = self.buf[0].len();

        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            self.cur_delay += (target - self.cur_delay) * 0.0005;
            let wl = self.read(0, self.cur_delay);
            let wr = self.read(1, self.cur_delay);
            self.lp[0] += (wl - self.lp[0]) * coef;
            self.lp[1] += (wr - self.lp[1]) * coef;

            let (in_l, in_r) = if s.ping_pong {
                // Mono input into the left line; feedback crosses over each repeat.
                ((*l + *r) * 0.5 + self.lp[1] * fb, self.lp[0] * fb)
            } else {
                (*l + self.lp[0] * fb, *r + self.lp[1] * fb)
            };
            self.buf[0][self.pos] = in_l;
            self.buf[1][self.pos] = in_r;
            self.pos = (self.pos + 1) % len;

            *l += wl * mix;
            *r += wr * mix;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn impulse_echoes_and_decays() {
        let sr = 48000.0;
        let mut d = Delay::new(sr);
        let s = DelaySettings {
            enabled: true,
            time_ms: 100.0,
            feedback: 0.5,
            mix: 1.0,
            ping_pong: false,
            ..Default::default()
        };
        let n = 48000;
        let mut l = vec![0.0; n];
        let mut r = vec![0.0; n];
        l[0] = 1.0;
        r[0] = 1.0;
        d.process(&s, &mut l, &mut r);
        assert!(l.iter().all(|x| x.is_finite()));
        // First echo near 100 ms.
        let echo = l[4790..4810].iter().fold(0.0f32, |a, &b| a.max(b.abs()));
        assert!(echo > 0.3, "echo {echo}");
        // Later echoes are quieter.
        let late = l[40000..].iter().fold(0.0f32, |a, &b| a.max(b.abs()));
        assert!(late < echo);
    }
}
