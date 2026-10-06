//! Audio output: runs the plugin, then delay, reverb and a soft limiter.

use std::sync::Arc;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use parking_lot::Mutex;

use crate::arp::{Arp, ArpSettings, NoteEvent};
use crate::fx::{FxSettings, delay::Delay, reverb::Reverb, soft_limit};
use crate::vst3host::processor::{MAX_BLOCK, Processor};

#[derive(Clone, Copy, Debug)]
pub enum Msg {
    NoteOn {
        pitch: u8,
        velocity: f32,
    },
    NoteOff {
        pitch: u8,
    },
    Param {
        id: u32,
        value: f64,
    },
    Fx(FxSettings),
    Arp(ArpSettings),
    /// Global tempo in BPM (arpeggiator, synced effects, host tempo for plugins).
    Tempo(f32),
}

/// Cloneable handle for sending messages to the audio thread.
#[derive(Clone)]
pub struct MsgSender(Arc<Mutex<rtrb::Producer<Msg>>>);

impl MsgSender {
    pub fn send(&self, msg: Msg) {
        // A full queue means the audio thread is stalled; dropping is the only sane option.
        let _ = self.0.lock().push(msg);
    }
}

/// A sender whose messages are never consumed (used when no audio device is available).
pub fn null_sender() -> MsgSender {
    let (tx, rx) = rtrb::RingBuffer::new(1);
    std::mem::forget(rx);
    MsgSender(Arc::new(Mutex::new(tx)))
}

#[cfg(test)]
pub fn test_sender(tx: rtrb::Producer<Msg>) -> MsgSender {
    MsgSender(Arc::new(Mutex::new(tx)))
}

/// The processor slot shared between the UI (which swaps plugins) and the audio thread.
pub type ProcessorSlot = Arc<Mutex<Option<Processor>>>;

pub struct AudioOut {
    _stream: cpal::Stream,
    pub sample_rate: f64,
    pub device_name: String,
}

struct Renderer {
    slot: ProcessorSlot,
    rx: rtrb::Consumer<Msg>,
    fx: FxSettings,
    delay: Delay,
    reverb: Reverb,
    arp: Arp,
    bpm: f32,
    sample_rate: f64,
    left: Vec<f32>,
    right: Vec<f32>,
}

impl Renderer {
    fn render(&mut self, out: &mut [f32], channels: usize) {
        let frames = out.len() / channels;
        let mut slot = self.slot.try_lock();

        while let Ok(msg) = self.rx.pop() {
            match (msg, slot.as_mut().and_then(|s| s.as_mut())) {
                (Msg::Fx(fx), _) => {
                    if fx.delay.enabled && !self.fx.delay.enabled {
                        self.delay.reset();
                    }
                    if fx.reverb.enabled && !self.fx.reverb.enabled {
                        self.reverb.reset();
                    }
                    self.fx = fx;
                }
                // Notes always go through the arpeggiator (it passes them on when disabled).
                (Msg::NoteOn { pitch, velocity }, _) => self.arp.note_on(pitch, velocity),
                (Msg::NoteOff { pitch }, _) => self.arp.note_off(pitch),
                (Msg::Arp(s), _) => self.arp.set_settings(s),
                (Msg::Tempo(bpm), _) => self.bpm = bpm,
                (Msg::Param { id, value }, Some(p)) => p.set_param(id, value),
                (Msg::Param { .. }, None) => {}
            }
        }

        let mut done = 0;
        while done < frames {
            let n = (frames - done).min(MAX_BLOCK);
            let (l, r) = (&mut self.left[..n], &mut self.right[..n]);
            let mut proc = slot.as_mut().and_then(|s| s.as_mut());
            self.arp
                .process(n, self.sample_rate, self.bpm, |offset, e| {
                    if let Some(p) = proc.as_mut() {
                        match e {
                            NoteEvent::On { pitch, velocity } => {
                                p.note_on_at(offset, pitch, velocity)
                            }
                            NoteEvent::Off { pitch } => p.note_off_at(offset, pitch),
                        }
                    }
                });
            match proc {
                Some(p) => {
                    p.set_tempo(self.bpm);
                    p.process(l, r);
                }
                None => {
                    l.fill(0.0);
                    r.fill(0.0);
                }
            }
            if self.fx.delay.enabled {
                let mut d = self.fx.delay;
                d.time_ms = d.effective_time_ms(self.bpm);
                self.delay.process(&d, l, r);
            }
            if self.fx.reverb.enabled {
                let mut rv = self.fx.reverb;
                rv.predelay_ms = rv.effective_predelay_ms(self.bpm);
                self.reverb.process(&rv, l, r);
            }
            let g = self.fx.gain;
            for i in 0..n {
                let frame = &mut out[(done + i) * channels..(done + i + 1) * channels];
                let (sl, sr) = (soft_limit(l[i] * g), soft_limit(r[i] * g));
                match channels {
                    1 => frame[0] = (sl + sr) * 0.5,
                    _ => {
                        frame[0] = sl;
                        frame[1] = sr;
                        frame[2..].fill(0.0);
                    }
                }
            }
            done += n;
        }
    }
}

pub fn start(slot: ProcessorSlot) -> Result<(AudioOut, MsgSender), String> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or("no audio output device")?;
    let device_name = device
        .description()
        .map(|d| d.to_string())
        .unwrap_or_else(|_| "default".into());
    let supported = device.default_output_config().map_err(|e| e.to_string())?;
    let format = supported.sample_format();
    let mut config = supported.config();
    let sample_rate = config.sample_rate as f64;
    let channels = config.channels as usize;
    // Ask for a small buffer for low latency; fall back to the device default if refused.
    config.buffer_size = cpal::BufferSize::Fixed(256);

    let (tx, rx) = rtrb::RingBuffer::new(4096);
    let renderer = Renderer {
        slot,
        rx,
        fx: FxSettings::default(),
        delay: Delay::new(sample_rate as f32),
        reverb: Reverb::new(sample_rate as f32),
        arp: Arp::default(),
        bpm: crate::tempo::DEFAULT_BPM,
        sample_rate,
        left: vec![0.0; MAX_BLOCK],
        right: vec![0.0; MAX_BLOCK],
    };
    let renderer = Arc::new(Mutex::new(renderer));

    let stream = build(&device, &config, format, channels, renderer.clone()).or_else(|_| {
        config.buffer_size = cpal::BufferSize::Default;
        build(&device, &config, format, channels, renderer)
    })?;
    stream.play().map_err(|e| e.to_string())?;

    Ok((
        AudioOut {
            _stream: stream,
            sample_rate,
            device_name,
        },
        MsgSender(Arc::new(Mutex::new(tx))),
    ))
}

fn build(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    format: cpal::SampleFormat,
    channels: usize,
    renderer: Arc<Mutex<Renderer>>,
) -> Result<cpal::Stream, String> {
    use cpal::SampleFormat as F;
    match format {
        F::F32 => build_typed::<f32>(device, config, channels, renderer),
        F::I16 => build_typed::<i16>(device, config, channels, renderer),
        F::I32 => build_typed::<i32>(device, config, channels, renderer),
        F::U16 => build_typed::<u16>(device, config, channels, renderer),
        other => Err(format!("unsupported sample format {other:?}")),
    }
}

fn build_typed<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    channels: usize,
    renderer: Arc<Mutex<Renderer>>,
) -> Result<cpal::Stream, String>
where
    T: cpal::SizedSample + cpal::FromSample<f32>,
{
    let mut scratch: Vec<f32> = Vec::new();
    device
        .build_output_stream::<T, _, _>(
            *config,
            move |out: &mut [T], _| {
                if scratch.len() < out.len() {
                    scratch.resize(out.len(), 0.0);
                }
                let buf = &mut scratch[..out.len()];
                renderer.lock().render(buf, channels);
                for (o, s) in out.iter_mut().zip(buf.iter()) {
                    *o = T::from_sample(*s);
                }
            },
            |e| eprintln!("audio stream error: {e}"),
            None,
        )
        .map_err(|e| e.to_string())
}
