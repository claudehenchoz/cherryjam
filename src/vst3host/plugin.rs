//! A loaded VST3 instrument: component + edit controller, wired up and activated.

// VST3 enum constants are i32 on Windows but u32 elsewhere, so these casts are needed.
#![allow(clippy::unnecessary_cast)]

use std::path::Path;
use std::sync::Arc;

use vst3::Steinberg::Vst::BusDirections_::{kInput, kOutput};
use vst3::Steinberg::Vst::MediaTypes_::{kAudio, kEvent};
use vst3::Steinberg::Vst::ParameterInfo_::ParameterFlags_::{kIsHidden, kIsReadOnly};
use vst3::Steinberg::Vst::{
    BusInfo, IAudioProcessor, IAudioProcessorTrait, IComponent, IComponentHandler, IComponentTrait,
    IConnectionPoint, IConnectionPointTrait, IEditController, IEditControllerTrait,
    IHostApplication, ParameterInfo, ProcessSetup,
};
use vst3::Steinberg::{FUnknown, IBStream, IPluginBaseTrait, TUID, kResultOk};
use vst3::{ComPtr, ComWrapper};

use super::host_iface::{ComponentHandler, HostApp, MemStream, PlugFrame, string128_to_string};
use super::module::{self, Module};
use super::processor::{MAX_BLOCK, Processor};
use crate::audio::MsgSender;

#[derive(Clone, Debug)]
pub struct ParamInfo {
    pub id: u32,
    pub title: String,
    pub units: String,
}

pub struct Plugin {
    _module: Arc<Module>,
    pub name: String,
    pub cid: TUID,
    /// Whether the loaded class declares itself an instrument.
    pub is_instrument: bool,
    pub component: ComPtr<IComponent>,
    pub controller: Option<ComPtr<IEditController>>,
    separate_controller: bool,
    connections: Option<(ComPtr<IConnectionPoint>, ComPtr<IConnectionPoint>)>,
    processor: ComPtr<IAudioProcessor>,
    _host: ComWrapper<HostApp>,
    pub handler: ComWrapper<ComponentHandler>,
    pub frame: ComWrapper<PlugFrame>,
}

impl Plugin {
    /// Loads the plugin at `binary`. `cid` selects a class; otherwise the first instrument is used.
    pub fn load(
        binary: &Path,
        cid: Option<TUID>,
        sample_rate: f64,
        sender: MsgSender,
    ) -> Result<(Plugin, Processor), String> {
        let module = module::load(binary)?;
        let classes = module.classes();
        let class = match cid {
            Some(cid) => classes.iter().find(|c| c.cid == cid),
            None => classes
                .iter()
                .find(|c| c.is_instrument())
                .or_else(|| classes.iter().find(|c| c.is_audio_module())),
        }
        .ok_or("no matching audio module class in plugin")?
        .clone();

        let host = ComWrapper::new(HostApp);
        let host_ptr = host.as_com_ref::<IHostApplication>().unwrap().as_ptr() as *mut FUnknown;

        unsafe {
            let component: ComPtr<IComponent> = module
                .create(&class.cid)
                .ok_or("could not create component")?;
            if component.initialize(host_ptr) != kResultOk {
                return Err("component initialization failed".into());
            }
            let processor = component
                .cast::<IAudioProcessor>()
                .ok_or("component has no audio processor")?;

            // Controller is either the component itself or a separate class.
            let mut separate_controller = false;
            let controller = match component.cast::<IEditController>() {
                Some(c) => Some(c),
                None => {
                    let mut ctl_cid: TUID = [0; 16];
                    if component.getControllerClassId(&mut ctl_cid) == kResultOk {
                        module.create::<IEditController>(&ctl_cid).and_then(|c| {
                            (c.initialize(host_ptr) == kResultOk).then(|| {
                                separate_controller = true;
                                c
                            })
                        })
                    } else {
                        None
                    }
                }
            };

            let handler = ComWrapper::new(ComponentHandler::new(sender));
            let mut connections = None;
            if let Some(ctl) = &controller {
                ctl.setComponentHandler(
                    handler.as_com_ref::<IComponentHandler>().unwrap().as_ptr(),
                );
                if separate_controller
                    && let (Some(a), Some(b)) = (
                        component.cast::<IConnectionPoint>(),
                        ctl.cast::<IConnectionPoint>(),
                    )
                {
                    a.connect(b.as_ptr());
                    b.connect(a.as_ptr());
                    connections = Some((a, b));
                }
                // Bring the controller in sync with the component's initial state.
                let stream = MemStream::new(Vec::new());
                let s = stream.as_com_ref::<IBStream>().unwrap().as_ptr();
                if component.getState(s) == kResultOk {
                    stream.rewind();
                    ctl.setComponentState(s);
                }
            }

            // Busses: first event input and first audio output on, everything else off.
            let bus_channels = |dir| {
                let n = component.getBusCount(kAudio as _, dir as _);
                (0..n)
                    .map(|i| {
                        let mut info: BusInfo = std::mem::zeroed();
                        component.getBusInfo(kAudio as _, dir as _, i, &mut info);
                        info.channelCount.max(0) as usize
                    })
                    .collect::<Vec<_>>()
            };
            let ins = bus_channels(kInput);
            let outs = bus_channels(kOutput);
            for i in 0..ins.len() {
                component.activateBus(kAudio as _, kInput as _, i as i32, 0);
            }
            for i in 0..outs.len() {
                component.activateBus(kAudio as _, kOutput as _, i as i32, (i == 0) as u8);
            }
            if component.getBusCount(kEvent as _, kInput as _) > 0 {
                component.activateBus(kEvent as _, kInput as _, 0, 1);
            }

            let mut setup = ProcessSetup {
                processMode: vst3::Steinberg::Vst::ProcessModes_::kRealtime as i32,
                symbolicSampleSize: vst3::Steinberg::Vst::SymbolicSampleSizes_::kSample32 as i32,
                maxSamplesPerBlock: MAX_BLOCK as i32,
                sampleRate: sample_rate,
            };
            if processor.setupProcessing(&mut setup) != kResultOk {
                return Err("plugin refused processing setup".into());
            }
            if component.setActive(1) != kResultOk {
                return Err("plugin could not be activated".into());
            }
            processor.setProcessing(1);

            let proc = Processor::new(processor.clone(), &ins, &outs, sample_rate);
            let plugin = Plugin {
                _module: module,
                name: class.name.clone(),
                is_instrument: class.is_instrument(),
                cid: class.cid,
                component,
                controller,
                separate_controller,
                connections,
                processor,
                _host: host,
                handler,
                frame: ComWrapper::new(PlugFrame::new()),
            };
            Ok((plugin, proc))
        }
    }

    /// Parameters that make sense to map to the XY controller.
    pub fn params(&self) -> Vec<ParamInfo> {
        let Some(ctl) = &self.controller else {
            return Vec::new();
        };
        let mut out = Vec::new();
        unsafe {
            for i in 0..ctl.getParameterCount() {
                let mut info: ParameterInfo = std::mem::zeroed();
                if ctl.getParameterInfo(i, &mut info) != kResultOk {
                    continue;
                }
                if info.flags & (kIsHidden | kIsReadOnly) as i32 != 0 {
                    continue;
                }
                out.push(ParamInfo {
                    id: info.id,
                    title: string128_to_string(&info.title),
                    units: string128_to_string(&info.units),
                });
            }
        }
        out
    }

    pub fn param_value(&self, id: u32) -> f64 {
        match &self.controller {
            Some(c) => unsafe { c.getParamNormalized(id) },
            None => 0.0,
        }
    }

    pub fn param_display(&self, id: u32, value: f64) -> String {
        let Some(c) = &self.controller else {
            return String::new();
        };
        unsafe {
            let mut s = [0u16; 128];
            if c.getParamStringByValue(id, value, &mut s) == kResultOk {
                string128_to_string(&s)
            } else {
                format!("{:.2}", value)
            }
        }
    }

    /// Updates the controller's view of a parameter (the processor is updated separately).
    pub fn set_param_ui(&self, id: u32, value: f64) {
        if let Some(c) = &self.controller {
            unsafe { c.setParamNormalized(id, value) };
        }
    }

    /// Returns (component state, controller state).
    pub fn get_state(&self) -> (Vec<u8>, Vec<u8>) {
        unsafe {
            let comp = MemStream::new(Vec::new());
            self.component
                .getState(comp.as_com_ref::<IBStream>().unwrap().as_ptr());
            let ctl = MemStream::new(Vec::new());
            if let Some(c) = &self.controller {
                c.getState(ctl.as_com_ref::<IBStream>().unwrap().as_ptr());
            }
            (comp.data(), ctl.data())
        }
    }

    pub fn set_state(&self, comp: &[u8], ctl: &[u8]) {
        unsafe {
            if !comp.is_empty() {
                let s = MemStream::new(comp.to_vec());
                self.component
                    .setState(s.as_com_ref::<IBStream>().unwrap().as_ptr());
                if let Some(c) = &self.controller {
                    s.rewind();
                    c.setComponentState(s.as_com_ref::<IBStream>().unwrap().as_ptr());
                }
            }
            if !ctl.is_empty()
                && let Some(c) = &self.controller
            {
                let s = MemStream::new(ctl.to_vec());
                c.setState(s.as_com_ref::<IBStream>().unwrap().as_ptr());
            }
        }
    }

    /// Deactivates and terminates the plugin. The processor must already be out of the audio thread.
    pub fn shutdown(self, processor: Option<Processor>) {
        unsafe {
            self.processor.setProcessing(0);
            drop(processor);
            self.component.setActive(0);
            if let Some((a, b)) = &self.connections {
                a.disconnect(b.as_ptr());
                b.disconnect(a.as_ptr());
            }
            if let Some(c) = &self.controller {
                c.setComponentHandler(std::ptr::null_mut());
                if self.separate_controller {
                    c.terminate();
                }
            }
            self.component.terminate();
        }
    }
}
