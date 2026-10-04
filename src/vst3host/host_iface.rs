//! Host-side COM objects handed to plugins.

use std::cell::UnsafeCell;
use std::collections::HashMap;
use std::ffi::{CStr, CString, c_void};
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use parking_lot::Mutex;
use vst3::Steinberg::Vst::{
    Event, IAttributeList, IAttributeListTrait, IComponentHandler, IComponentHandlerTrait,
    IEventList, IEventListTrait, IHostApplication, IHostApplicationTrait, IMessage, IMessageTrait,
    IParamValueQueue, IParamValueQueueTrait, IParameterChanges, IParameterChangesTrait, ParamID,
    ParamValue, String128,
};
use vst3::Steinberg::{
    FIDString, IBStream, IBStreamTrait, IPlugFrame, IPlugFrameTrait, IPlugView, IPlugViewTrait,
    TUID, ViewRect, int32, int64, kInvalidArgument, kNoInterface, kResultFalse, kResultOk, tresult,
};
use vst3::{Class, ComPtr, ComWrapper, Interface};

use crate::audio::{Msg, MsgSender};

fn write_string128(dst: *mut String128, s: &str) {
    unsafe {
        let dst = &mut *dst;
        let mut n = 0;
        for (i, c) in s.encode_utf16().take(127).enumerate() {
            dst[i] = c;
            n = i + 1;
        }
        dst[n] = 0;
    }
}

pub fn string128_to_string(s: &String128) -> String {
    let end = s.iter().position(|&c| c == 0).unwrap_or(s.len());
    String::from_utf16_lossy(&s[..end])
}

fn tuid_bytes(t: &TUID) -> [u8; 16] {
    t.map(|b| b as u8)
}

// ---------------------------------------------------------------------------------------------
// IHostApplication

pub struct HostApp;

impl Class for HostApp {
    type Interfaces = (IHostApplication,);
}

impl IHostApplicationTrait for HostApp {
    unsafe fn getName(&self, name: *mut String128) -> tresult {
        write_string128(name, "CherryJam");
        kResultOk
    }

    unsafe fn createInstance(
        &self,
        cid: *mut TUID,
        iid: *mut TUID,
        obj: *mut *mut c_void,
    ) -> tresult {
        unsafe {
            if cid.is_null() || iid.is_null() || obj.is_null() {
                return kInvalidArgument;
            }
            let cid = tuid_bytes(&*cid);
            let unknown: Option<ComPtr<vst3::Steinberg::FUnknown>> = if cid == IMessage::IID {
                ComWrapper::new(Message::default())
                    .to_com_ptr::<IMessage>()
                    .map(|p| p.upcast())
            } else if cid == IAttributeList::IID {
                ComWrapper::new(AttributeList::default())
                    .to_com_ptr::<IAttributeList>()
                    .map(|p| p.upcast())
            } else {
                None
            };
            let Some(unknown) = unknown else {
                *obj = std::ptr::null_mut();
                return kNoInterface;
            };
            let u = unknown.as_ptr();
            ((*(*u).vtbl).queryInterface)(u, iid as *const TUID, obj)
        }
    }
}

// ---------------------------------------------------------------------------------------------
// IMessage / IAttributeList (used by plugins to talk between component and controller)

pub struct Message {
    id: Mutex<Option<CString>>,
    attrs: ComWrapper<AttributeList>,
}

impl Default for Message {
    fn default() -> Self {
        Message {
            id: Mutex::new(None),
            attrs: ComWrapper::new(AttributeList::default()),
        }
    }
}

impl Class for Message {
    type Interfaces = (IMessage,);
}

impl IMessageTrait for Message {
    unsafe fn getMessageID(&self) -> FIDString {
        self.id
            .lock()
            .as_ref()
            .map_or(std::ptr::null(), |s| s.as_ptr())
    }

    unsafe fn setMessageID(&self, id: FIDString) {
        *self.id.lock() = (!id.is_null()).then(|| unsafe { CStr::from_ptr(id) }.to_owned());
    }

    unsafe fn getAttributes(&self) -> *mut IAttributeList {
        self.attrs.as_com_ref::<IAttributeList>().unwrap().as_ptr()
    }
}

enum Attr {
    Int(i64),
    Float(f64),
    Str(Vec<u16>),
    Bin(Vec<u8>),
}

#[derive(Default)]
pub struct AttributeList {
    map: Mutex<HashMap<CString, Attr>>,
}

impl Class for AttributeList {
    type Interfaces = (IAttributeList,);
}

fn key(id: *const std::ffi::c_char) -> Option<CString> {
    (!id.is_null()).then(|| unsafe { CStr::from_ptr(id) }.to_owned())
}

impl IAttributeListTrait for AttributeList {
    unsafe fn setInt(&self, id: *const std::ffi::c_char, value: int64) -> tresult {
        let Some(k) = key(id) else {
            return kInvalidArgument;
        };
        self.map.lock().insert(k, Attr::Int(value));
        kResultOk
    }

    unsafe fn getInt(&self, id: *const std::ffi::c_char, value: *mut int64) -> tresult {
        let Some(k) = key(id) else {
            return kInvalidArgument;
        };
        match self.map.lock().get(&k) {
            Some(Attr::Int(v)) => {
                unsafe { *value = *v };
                kResultOk
            }
            _ => kResultFalse,
        }
    }

    unsafe fn setFloat(&self, id: *const std::ffi::c_char, value: f64) -> tresult {
        let Some(k) = key(id) else {
            return kInvalidArgument;
        };
        self.map.lock().insert(k, Attr::Float(value));
        kResultOk
    }

    unsafe fn getFloat(&self, id: *const std::ffi::c_char, value: *mut f64) -> tresult {
        let Some(k) = key(id) else {
            return kInvalidArgument;
        };
        match self.map.lock().get(&k) {
            Some(Attr::Float(v)) => {
                unsafe { *value = *v };
                kResultOk
            }
            _ => kResultFalse,
        }
    }

    unsafe fn setString(&self, id: *const std::ffi::c_char, string: *const u16) -> tresult {
        let Some(k) = key(id) else {
            return kInvalidArgument;
        };
        if string.is_null() {
            return kInvalidArgument;
        }
        let mut v = Vec::new();
        unsafe {
            let mut p = string;
            while *p != 0 {
                v.push(*p);
                p = p.add(1);
            }
        }
        v.push(0);
        self.map.lock().insert(k, Attr::Str(v));
        kResultOk
    }

    unsafe fn getString(
        &self,
        id: *const std::ffi::c_char,
        string: *mut u16,
        size_in_bytes: u32,
    ) -> tresult {
        let Some(k) = key(id) else {
            return kInvalidArgument;
        };
        match self.map.lock().get(&k) {
            Some(Attr::Str(v)) => {
                let cap = size_in_bytes as usize / 2;
                if cap == 0 {
                    return kResultFalse;
                }
                let n = v.len().min(cap);
                unsafe {
                    std::ptr::copy_nonoverlapping(v.as_ptr(), string, n);
                    *string.add(n - 1) = 0;
                }
                kResultOk
            }
            _ => kResultFalse,
        }
    }

    unsafe fn setBinary(
        &self,
        id: *const std::ffi::c_char,
        data: *const c_void,
        size_in_bytes: u32,
    ) -> tresult {
        let Some(k) = key(id) else {
            return kInvalidArgument;
        };
        let bytes = if data.is_null() || size_in_bytes == 0 {
            Vec::new()
        } else {
            unsafe { std::slice::from_raw_parts(data as *const u8, size_in_bytes as usize) }
                .to_vec()
        };
        self.map.lock().insert(k, Attr::Bin(bytes));
        kResultOk
    }

    unsafe fn getBinary(
        &self,
        id: *const std::ffi::c_char,
        data: *mut *const c_void,
        size_in_bytes: *mut u32,
    ) -> tresult {
        let Some(k) = key(id) else {
            return kInvalidArgument;
        };
        match self.map.lock().get(&k) {
            Some(Attr::Bin(v)) => {
                // The buffer stays valid as long as the attribute is not overwritten.
                unsafe {
                    *data = v.as_ptr() as *const c_void;
                    *size_in_bytes = v.len() as u32;
                }
                kResultOk
            }
            _ => kResultFalse,
        }
    }
}

// ---------------------------------------------------------------------------------------------
// IBStream backed by memory (plugin state)

pub struct MemStream {
    inner: Mutex<(Vec<u8>, usize)>,
}

impl Class for MemStream {
    type Interfaces = (IBStream,);
}

impl MemStream {
    pub fn new(data: Vec<u8>) -> ComWrapper<MemStream> {
        ComWrapper::new(MemStream {
            inner: Mutex::new((data, 0)),
        })
    }
    pub fn data(&self) -> Vec<u8> {
        self.inner.lock().0.clone()
    }
    pub fn rewind(&self) {
        self.inner.lock().1 = 0;
    }
}

impl IBStreamTrait for MemStream {
    unsafe fn read(&self, buffer: *mut c_void, num_bytes: int32, read: *mut int32) -> tresult {
        let mut g = self.inner.lock();
        let (data, pos) = &mut *g;
        let n = (num_bytes.max(0) as usize).min(data.len().saturating_sub(*pos));
        unsafe {
            std::ptr::copy_nonoverlapping(data.as_ptr().add(*pos), buffer as *mut u8, n);
            if !read.is_null() {
                *read = n as int32;
            }
        }
        *pos += n;
        kResultOk
    }

    unsafe fn write(&self, buffer: *mut c_void, num_bytes: int32, written: *mut int32) -> tresult {
        let mut g = self.inner.lock();
        let (data, pos) = &mut *g;
        let n = num_bytes.max(0) as usize;
        if data.len() < *pos + n {
            data.resize(*pos + n, 0);
        }
        unsafe {
            std::ptr::copy_nonoverlapping(buffer as *const u8, data.as_mut_ptr().add(*pos), n);
            if !written.is_null() {
                *written = n as int32;
            }
        }
        *pos += n;
        kResultOk
    }

    unsafe fn seek(&self, pos: int64, mode: int32, result: *mut int64) -> tresult {
        use vst3::Steinberg::IBStream_::IStreamSeekMode_::{kIBSeekCur, kIBSeekEnd, kIBSeekSet};
        let mut g = self.inner.lock();
        let (data, cur) = &mut *g;
        let base = if mode == kIBSeekSet as int32 {
            0i64
        } else if mode == kIBSeekCur as int32 {
            *cur as i64
        } else if mode == kIBSeekEnd as int32 {
            data.len() as i64
        } else {
            return kInvalidArgument;
        };
        let new = (base + pos).max(0) as usize;
        if new > data.len() {
            data.resize(new, 0);
        }
        *cur = new;
        if !result.is_null() {
            unsafe { *result = new as i64 };
        }
        kResultOk
    }

    unsafe fn tell(&self, pos: *mut int64) -> tresult {
        if pos.is_null() {
            return kInvalidArgument;
        }
        unsafe { *pos = self.inner.lock().1 as i64 };
        kResultOk
    }
}

// ---------------------------------------------------------------------------------------------
// IComponentHandler: edits coming from the plugin GUI

pub struct ComponentHandler {
    sender: MsgSender,
    /// Last parameter touched in the plugin GUI (-1 = none). Used for "learn" in the mapping UI.
    pub last_touched: Arc<AtomicI64>,
    pub restart_flags: Arc<AtomicI64>,
}

impl ComponentHandler {
    pub fn new(sender: MsgSender) -> Self {
        ComponentHandler {
            sender,
            last_touched: Arc::new(AtomicI64::new(-1)),
            restart_flags: Arc::new(AtomicI64::new(0)),
        }
    }
}

impl Class for ComponentHandler {
    type Interfaces = (IComponentHandler,);
}

impl IComponentHandlerTrait for ComponentHandler {
    unsafe fn beginEdit(&self, id: ParamID) -> tresult {
        self.last_touched.store(id as i64, Ordering::Relaxed);
        kResultOk
    }

    unsafe fn performEdit(&self, id: ParamID, value: ParamValue) -> tresult {
        self.last_touched.store(id as i64, Ordering::Relaxed);
        self.sender.send(Msg::Param { id, value });
        kResultOk
    }

    unsafe fn endEdit(&self, _id: ParamID) -> tresult {
        kResultOk
    }

    unsafe fn restartComponent(&self, flags: int32) -> tresult {
        self.restart_flags.fetch_or(flags as i64, Ordering::Relaxed);
        kResultOk
    }
}

// ---------------------------------------------------------------------------------------------
// IPlugFrame (+ IRunLoop on Linux): the plugin asks us to resize its editor

#[derive(Default)]
pub struct FrameState {
    /// Size of the plugin view in physical pixels, as last reported/negotiated.
    pub size: Option<(i32, i32)>,
    /// Native child window that hosts the plugin view.
    pub child: usize,
}

pub struct PlugFrame {
    pub state: Arc<Mutex<FrameState>>,
    #[cfg(target_os = "linux")]
    pub run_loop: RunLoop,
}

impl PlugFrame {
    pub fn new() -> Self {
        PlugFrame {
            state: Default::default(),
            #[cfg(target_os = "linux")]
            run_loop: RunLoop::default(),
        }
    }
}

#[cfg(not(target_os = "linux"))]
impl Class for PlugFrame {
    type Interfaces = (IPlugFrame,);
}

#[cfg(target_os = "linux")]
impl Class for PlugFrame {
    type Interfaces = (IPlugFrame, vst3::Steinberg::Linux::IRunLoop);
}

impl IPlugFrameTrait for PlugFrame {
    unsafe fn resizeView(&self, view: *mut IPlugView, new_size: *mut ViewRect) -> tresult {
        if view.is_null() || new_size.is_null() {
            return kInvalidArgument;
        }
        unsafe {
            let r = *new_size;
            let (w, h) = (r.right - r.left, r.bottom - r.top);
            let child = {
                let mut st = self.state.lock();
                st.size = Some((w, h));
                st.child
            };
            if child != 0 {
                super::editor::resize_child(child, w, h);
            }
            if let Some(v) = vst3::ComRef::from_raw(view) {
                v.onSize(new_size);
            }
        }
        kResultOk
    }
}

#[cfg(target_os = "linux")]
#[derive(Default)]
pub struct RunLoop {
    handlers: Mutex<Vec<(ComPtr<vst3::Steinberg::Linux::IEventHandler>, i32)>>,
    timers: Mutex<
        Vec<(
            ComPtr<vst3::Steinberg::Linux::ITimerHandler>,
            u64,
            std::time::Instant,
        )>,
    >,
}

#[cfg(target_os = "linux")]
impl RunLoop {
    /// Called every UI frame: fires due timers and lets fd handlers process their events.
    pub fn pump(&self) {
        use vst3::Steinberg::Linux::{IEventHandlerTrait, ITimerHandlerTrait};
        let handlers: Vec<_> = self.handlers.lock().iter().cloned().collect();
        for (h, fd) in handlers {
            unsafe { h.onFDIsSet(fd) };
        }
        let now = std::time::Instant::now();
        let due: Vec<_> = {
            let mut t = self.timers.lock();
            t.iter_mut()
                .filter(|(_, ms, last)| now.duration_since(*last).as_millis() as u64 >= *ms)
                .map(|(h, _, last)| {
                    *last = now;
                    h.clone()
                })
                .collect()
        };
        for h in due {
            unsafe { h.onTimer() };
        }
    }
}

#[cfg(target_os = "linux")]
impl vst3::Steinberg::Linux::IRunLoopTrait for PlugFrame {
    unsafe fn registerEventHandler(
        &self,
        handler: *mut vst3::Steinberg::Linux::IEventHandler,
        fd: vst3::Steinberg::Linux::FileDescriptor,
    ) -> tresult {
        match unsafe { vst3::ComRef::from_raw(handler) } {
            Some(h) => {
                self.run_loop.handlers.lock().push((h.to_com_ptr(), fd));
                kResultOk
            }
            None => kInvalidArgument,
        }
    }

    unsafe fn unregisterEventHandler(
        &self,
        handler: *mut vst3::Steinberg::Linux::IEventHandler,
    ) -> tresult {
        self.run_loop
            .handlers
            .lock()
            .retain(|(h, _)| h.as_ptr() != handler);
        kResultOk
    }

    unsafe fn registerTimer(
        &self,
        handler: *mut vst3::Steinberg::Linux::ITimerHandler,
        ms: vst3::Steinberg::Linux::TimerInterval,
    ) -> tresult {
        match unsafe { vst3::ComRef::from_raw(handler) } {
            Some(h) => {
                self.run_loop
                    .timers
                    .lock()
                    .push((h.to_com_ptr(), ms, std::time::Instant::now()));
                kResultOk
            }
            None => kInvalidArgument,
        }
    }

    unsafe fn unregisterTimer(
        &self,
        handler: *mut vst3::Steinberg::Linux::ITimerHandler,
    ) -> tresult {
        self.run_loop
            .timers
            .lock()
            .retain(|(h, _, _)| h.as_ptr() != handler);
        kResultOk
    }
}

// ---------------------------------------------------------------------------------------------
// Audio-thread objects. They are only ever touched from the audio thread while `process` runs,
// so interior mutability without locking is fine.

pub struct AudioCell<T>(UnsafeCell<T>);
unsafe impl<T> Sync for AudioCell<T> {}
unsafe impl<T> Send for AudioCell<T> {}

impl<T> AudioCell<T> {
    pub fn new(v: T) -> Self {
        AudioCell(UnsafeCell::new(v))
    }
    #[allow(clippy::mut_from_ref)]
    pub fn get(&self) -> &mut T {
        unsafe { &mut *self.0.get() }
    }
}

pub struct EventList {
    pub events: AudioCell<Vec<Event>>,
}

impl EventList {
    pub fn new() -> Self {
        EventList {
            events: AudioCell::new(Vec::with_capacity(1024)),
        }
    }
}

impl Class for EventList {
    type Interfaces = (IEventList,);
}

impl IEventListTrait for EventList {
    unsafe fn getEventCount(&self) -> int32 {
        self.events.get().len() as int32
    }

    unsafe fn getEvent(&self, index: int32, e: *mut Event) -> tresult {
        match self.events.get().get(index as usize) {
            Some(ev) if !e.is_null() => {
                unsafe { *e = *ev };
                kResultOk
            }
            _ => kInvalidArgument,
        }
    }

    unsafe fn addEvent(&self, e: *mut Event) -> tresult {
        let v = self.events.get();
        if e.is_null() || v.len() == v.capacity() {
            return kResultFalse;
        }
        v.push(unsafe { *e });
        kResultOk
    }
}

pub struct ParamQueue {
    pub id: AudioCell<ParamID>,
    pub points: AudioCell<Vec<(i32, f64)>>,
}

impl Class for ParamQueue {
    type Interfaces = (IParamValueQueue,);
}

impl IParamValueQueueTrait for ParamQueue {
    unsafe fn getParameterId(&self) -> ParamID {
        *self.id.get()
    }

    unsafe fn getPointCount(&self) -> int32 {
        self.points.get().len() as int32
    }

    unsafe fn getPoint(&self, index: int32, offset: *mut int32, value: *mut ParamValue) -> tresult {
        match self.points.get().get(index as usize) {
            Some(&(o, v)) => {
                unsafe {
                    if !offset.is_null() {
                        *offset = o;
                    }
                    if !value.is_null() {
                        *value = v;
                    }
                }
                kResultOk
            }
            None => kInvalidArgument,
        }
    }

    unsafe fn addPoint(&self, offset: int32, value: ParamValue, index: *mut int32) -> tresult {
        let p = self.points.get();
        if p.len() == p.capacity() {
            // Keep it allocation-free: overwrite the last point instead of growing.
            if let Some(last) = p.last_mut() {
                *last = (offset, value);
            }
        } else {
            p.push((offset, value));
        }
        if !index.is_null() {
            unsafe { *index = p.len() as int32 - 1 };
        }
        kResultOk
    }
}

pub struct ParamChanges {
    queues: Vec<ComWrapper<ParamQueue>>,
    used: AudioCell<usize>,
}

impl ParamChanges {
    pub fn new(capacity: usize) -> Self {
        let queues = (0..capacity)
            .map(|_| {
                ComWrapper::new(ParamQueue {
                    id: AudioCell::new(0),
                    points: AudioCell::new(Vec::with_capacity(16)),
                })
            })
            .collect();
        ParamChanges {
            queues,
            used: AudioCell::new(0),
        }
    }

    pub fn clear(&self) {
        for q in &self.queues[..*self.used.get()] {
            q.points.get().clear();
        }
        *self.used.get() = 0;
    }

    /// Adds (or replaces) a value at sample offset 0 for parameter `id`.
    pub fn set(&self, id: ParamID, value: f64) {
        let used = self.used.get();
        if let Some(q) = self.queues[..*used].iter().find(|q| *q.id.get() == id) {
            let p = q.points.get();
            p.clear();
            p.push((0, value));
            return;
        }
        if *used < self.queues.len() {
            let q = &self.queues[*used];
            *q.id.get() = id;
            q.points.get().clear();
            q.points.get().push((0, value));
            *used += 1;
        }
    }
}

impl Class for ParamChanges {
    type Interfaces = (IParameterChanges,);
}

impl IParameterChangesTrait for ParamChanges {
    unsafe fn getParameterCount(&self) -> int32 {
        *self.used.get() as int32
    }

    unsafe fn getParameterData(&self, index: int32) -> *mut IParamValueQueue {
        if (index as usize) < *self.used.get() {
            self.queues[index as usize]
                .as_com_ref::<IParamValueQueue>()
                .unwrap()
                .as_ptr()
        } else {
            std::ptr::null_mut()
        }
    }

    unsafe fn addParameterData(
        &self,
        id: *const ParamID,
        index: *mut int32,
    ) -> *mut IParamValueQueue {
        if id.is_null() {
            return std::ptr::null_mut();
        }
        let id = unsafe { *id };
        let used = self.used.get();
        let pos = match self.queues[..*used].iter().position(|q| *q.id.get() == id) {
            Some(p) => p,
            None if *used < self.queues.len() => {
                let q = &self.queues[*used];
                *q.id.get() = id;
                q.points.get().clear();
                *used += 1;
                *used - 1
            }
            None => return std::ptr::null_mut(),
        };
        if !index.is_null() {
            unsafe { *index = pos as int32 };
        }
        self.queues[pos]
            .as_com_ref::<IParamValueQueue>()
            .unwrap()
            .as_ptr()
    }
}
