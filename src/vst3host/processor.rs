//! The audio-thread half of a loaded plugin.

// VST3 enum constants are i32 on Windows but u32 elsewhere, so these casts are needed.
#![allow(clippy::unnecessary_cast)]

use vst3::Steinberg::Vst::Event_::EventTypes_::{kNoteOffEvent, kNoteOnEvent};
use vst3::Steinberg::Vst::ProcessContext_::StatesAndFlags_::{
    kContTimeValid, kPlaying, kProjectTimeMusicValid, kTempoValid, kTimeSigValid,
};
use vst3::Steinberg::Vst::{
    AudioBusBuffers, AudioBusBuffers__type0, Event, IAudioProcessor, IAudioProcessorTrait,
    IEventList, IParameterChanges, NoteOffEvent, NoteOnEvent, ParamID, ProcessContext, ProcessData,
};
use vst3::{ComPtr, ComWrapper};

use super::host_iface::{EventList, ParamChanges};

pub const MAX_BLOCK: usize = 512;
const TEMPO: f64 = 120.0;

struct Bus {
    channels: Vec<Vec<f32>>,
    ptrs: Vec<*mut f32>,
}

impl Bus {
    fn new(channels: usize) -> Self {
        let mut channels: Vec<Vec<f32>> = (0..channels).map(|_| vec![0.0; MAX_BLOCK]).collect();
        let ptrs = channels.iter_mut().map(|c| c.as_mut_ptr()).collect();
        Bus { channels, ptrs }
    }
}

pub struct Processor {
    proc: ComPtr<IAudioProcessor>,
    events: ComWrapper<EventList>,
    out_events: ComWrapper<EventList>,
    in_params: ComWrapper<ParamChanges>,
    out_params: ComWrapper<ParamChanges>,
    inputs: Vec<Bus>,
    outputs: Vec<Bus>,
    in_bufs: Vec<AudioBusBuffers>,
    out_bufs: Vec<AudioBusBuffers>,
    sample_rate: f64,
    sample_pos: i64,
}

// The processor is moved to the audio thread once and only used there afterwards.
unsafe impl Send for Processor {}

impl Processor {
    pub fn new(
        proc: ComPtr<IAudioProcessor>,
        input_channels: &[usize],
        output_channels: &[usize],
        sample_rate: f64,
    ) -> Self {
        let inputs: Vec<Bus> = input_channels.iter().map(|&c| Bus::new(c)).collect();
        let outputs: Vec<Bus> = output_channels.iter().map(|&c| Bus::new(c)).collect();
        let empty = || AudioBusBuffers {
            numChannels: 0,
            silenceFlags: 0,
            __field0: AudioBusBuffers__type0 {
                channelBuffers32: std::ptr::null_mut(),
            },
        };
        Processor {
            proc,
            events: ComWrapper::new(EventList::new()),
            out_events: ComWrapper::new(EventList::new()),
            in_params: ComWrapper::new(ParamChanges::new(64)),
            out_params: ComWrapper::new(ParamChanges::new(64)),
            in_bufs: (0..inputs.len()).map(|_| empty()).collect(),
            out_bufs: (0..outputs.len()).map(|_| empty()).collect(),
            inputs,
            outputs,
            sample_rate,
            sample_pos: 0,
        }
    }

    pub fn note_on(&mut self, pitch: u8, velocity: f32) {
        let mut e: Event = unsafe { std::mem::zeroed() };
        e.r#type = kNoteOnEvent as u16;
        e.__field0.noteOn = NoteOnEvent {
            channel: 0,
            pitch: pitch as i16,
            tuning: 0.0,
            velocity,
            length: 0,
            noteId: -1,
        };
        self.push_event(e);
    }

    pub fn note_off(&mut self, pitch: u8) {
        let mut e: Event = unsafe { std::mem::zeroed() };
        e.r#type = kNoteOffEvent as u16;
        e.__field0.noteOff = NoteOffEvent {
            channel: 0,
            pitch: pitch as i16,
            velocity: 0.0,
            noteId: -1,
            tuning: 0.0,
        };
        self.push_event(e);
    }

    fn push_event(&mut self, e: Event) {
        let v = self.events.events.get();
        if v.len() < v.capacity() {
            v.push(e);
        }
    }

    pub fn set_param(&mut self, id: ParamID, value: f64) {
        self.in_params.set(id, value);
    }

    /// Renders `left.len()` (<= MAX_BLOCK) frames of the main output bus into `left`/`right`.
    pub fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        let n = left.len().min(MAX_BLOCK);

        for (bus, buf) in self.inputs.iter_mut().zip(self.in_bufs.iter_mut()) {
            for c in &mut bus.channels {
                c[..n].fill(0.0);
            }
            buf.numChannels = bus.ptrs.len() as i32;
            buf.silenceFlags = u64::MAX;
            buf.__field0.channelBuffers32 = bus.ptrs.as_mut_ptr();
        }
        for (bus, buf) in self.outputs.iter_mut().zip(self.out_bufs.iter_mut()) {
            buf.numChannels = bus.ptrs.len() as i32;
            buf.silenceFlags = 0;
            buf.__field0.channelBuffers32 = bus.ptrs.as_mut_ptr();
        }

        let mut ctx: ProcessContext = unsafe { std::mem::zeroed() };
        ctx.state =
            (kPlaying | kTempoValid | kTimeSigValid | kContTimeValid | kProjectTimeMusicValid)
                as u32;
        ctx.sampleRate = self.sample_rate;
        ctx.projectTimeSamples = self.sample_pos;
        ctx.continousTimeSamples = self.sample_pos;
        ctx.projectTimeMusic = self.sample_pos as f64 / self.sample_rate * TEMPO / 60.0;
        ctx.tempo = TEMPO;
        ctx.timeSigNumerator = 4;
        ctx.timeSigDenominator = 4;

        self.out_events.events.get().clear();
        self.out_params.clear();

        let mut data = ProcessData {
            processMode: vst3::Steinberg::Vst::ProcessModes_::kRealtime as i32,
            symbolicSampleSize: vst3::Steinberg::Vst::SymbolicSampleSizes_::kSample32 as i32,
            numSamples: n as i32,
            numInputs: self.in_bufs.len() as i32,
            numOutputs: self.out_bufs.len() as i32,
            inputs: if self.in_bufs.is_empty() {
                std::ptr::null_mut()
            } else {
                self.in_bufs.as_mut_ptr()
            },
            outputs: if self.out_bufs.is_empty() {
                std::ptr::null_mut()
            } else {
                self.out_bufs.as_mut_ptr()
            },
            inputParameterChanges: self
                .in_params
                .as_com_ref::<IParameterChanges>()
                .unwrap()
                .as_ptr(),
            outputParameterChanges: self
                .out_params
                .as_com_ref::<IParameterChanges>()
                .unwrap()
                .as_ptr(),
            inputEvents: self.events.as_com_ref::<IEventList>().unwrap().as_ptr(),
            outputEvents: self.out_events.as_com_ref::<IEventList>().unwrap().as_ptr(),
            processContext: &mut ctx,
        };

        unsafe { self.proc.process(&mut data) };

        self.events.events.get().clear();
        self.in_params.clear();
        self.sample_pos += n as i64;

        match self.outputs.first() {
            Some(bus) if bus.channels.len() >= 2 => {
                left[..n].copy_from_slice(&bus.channels[0][..n]);
                right[..n].copy_from_slice(&bus.channels[1][..n]);
            }
            Some(bus) if bus.channels.len() == 1 => {
                left[..n].copy_from_slice(&bus.channels[0][..n]);
                right[..n].copy_from_slice(&bus.channels[0][..n]);
            }
            _ => {
                left[..n].fill(0.0);
                right[..n].fill(0.0);
            }
        }
    }
}
