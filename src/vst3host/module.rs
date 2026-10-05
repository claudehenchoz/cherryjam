//! Loading VST3 shared libraries and enumerating their classes.

use std::collections::HashMap;
use std::ffi::{CStr, c_void};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use parking_lot::Mutex;
use vst3::ComPtr;
use vst3::Steinberg::{IPluginFactory, IPluginFactory2, IPluginFactory2Trait, IPluginFactoryTrait};
use vst3::Steinberg::{PClassInfo, PClassInfo2, TUID, kResultOk};

pub struct Module {
    // Kept alive for the life of the process: many plugins misbehave when unloaded and reloaded.
    _lib: libloading::Library,
    pub factory: ComPtr<IPluginFactory>,
}

// The factory is only used from the UI thread; the module itself is never dropped.
unsafe impl Send for Module {}
unsafe impl Sync for Module {}

#[derive(Clone, Debug)]
pub struct ClassInfo {
    pub cid: TUID,
    pub name: String,
    pub category: String,
    pub sub_categories: String,
}

impl ClassInfo {
    pub fn is_audio_module(&self) -> bool {
        self.category == "Audio Module Class"
    }
    pub fn is_instrument(&self) -> bool {
        self.is_audio_module() && self.sub_categories.contains("Instrument")
    }
}

fn cache() -> &'static Mutex<HashMap<PathBuf, Arc<Module>>> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, Arc<Module>>>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

/// Loads (or returns the already loaded) module at `binary`.
pub fn load(binary: &Path) -> Result<Arc<Module>, String> {
    if let Some(m) = cache().lock().get(binary) {
        return Ok(m.clone());
    }
    let module = Arc::new(unsafe { load_uncached(binary)? });
    cache().lock().insert(binary.to_path_buf(), module.clone());
    Ok(module)
}

type GetFactory = unsafe extern "system" fn() -> *mut IPluginFactory;

unsafe fn load_uncached(binary: &Path) -> Result<Module, String> {
    unsafe {
        let lib =
            open_library(binary).map_err(|e| format!("cannot load {}: {e}", binary.display()))?;

        #[cfg(windows)]
        if let Ok(init) = lib.get::<unsafe extern "system" fn() -> bool>(b"InitDll\0") {
            init();
        }
        // ModuleEntry gets the dlopen handle; JUCE plugins use it to locate their own files.
        #[cfg(target_os = "linux")]
        let lib = {
            let handle = libloading::os::unix::Library::from(lib).into_raw();
            let lib: libloading::Library = libloading::os::unix::Library::from_raw(handle).into();
            if let Ok(entry) =
                lib.get::<unsafe extern "C" fn(*mut c_void) -> bool>(b"ModuleEntry\0")
                && !entry(handle)
            {
                return Err("ModuleEntry failed".into());
            }
            lib
        };

        let get: libloading::Symbol<GetFactory> = lib
            .get(b"GetPluginFactory\0")
            .map_err(|_| "not a VST3 plugin (no GetPluginFactory)".to_string())?;
        let factory = ComPtr::from_raw(get()).ok_or("plugin returned no factory")?;
        Ok(Module { _lib: lib, factory })
    }
}

#[cfg(windows)]
unsafe fn open_library(path: &Path) -> Result<libloading::Library, libloading::Error> {
    // Lets the plugin find DLLs that sit next to it.
    const LOAD_WITH_ALTERED_SEARCH_PATH: u32 = 0x8;
    unsafe {
        libloading::os::windows::Library::load_with_flags(path, LOAD_WITH_ALTERED_SEARCH_PATH)
            .map(Into::into)
    }
}

#[cfg(not(windows))]
unsafe fn open_library(path: &Path) -> Result<libloading::Library, libloading::Error> {
    unsafe { libloading::Library::new(path) }
}

fn cstr(buf: &[std::ffi::c_char]) -> String {
    let bytes: Vec<u8> = buf
        .iter()
        .take_while(|&&c| c != 0)
        .map(|&c| c as u8)
        .collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

impl Module {
    pub fn classes(&self) -> Vec<ClassInfo> {
        let mut out = Vec::new();
        unsafe {
            let f2 = self.factory.cast::<IPluginFactory2>();
            let n = self.factory.countClasses();
            for i in 0..n {
                if let Some(f2) = &f2 {
                    let mut info: PClassInfo2 = std::mem::zeroed();
                    if f2.getClassInfo2(i, &mut info) == kResultOk {
                        out.push(ClassInfo {
                            cid: info.cid,
                            name: cstr(&info.name),
                            category: cstr(&info.category),
                            sub_categories: cstr(&info.subCategories),
                        });
                        continue;
                    }
                }
                let mut info: PClassInfo = std::mem::zeroed();
                if self.factory.getClassInfo(i, &mut info) == kResultOk {
                    out.push(ClassInfo {
                        cid: info.cid,
                        name: cstr(&info.name),
                        category: cstr(&info.category),
                        sub_categories: String::new(),
                    });
                }
            }
        }
        out
    }

    /// Creates an instance of class `cid` with interface `I`.
    pub fn create<I: vst3::Interface>(&self, cid: &TUID) -> Option<ComPtr<I>> {
        unsafe {
            let mut obj: *mut c_void = std::ptr::null_mut();
            let r = self.factory.createInstance(
                cid.as_ptr(),
                I::IID.as_ptr() as *const std::ffi::c_char,
                &mut obj,
            );
            if r == kResultOk {
                ComPtr::from_raw(obj as *mut I)
            } else {
                None
            }
        }
    }
}

pub fn tuid_to_hex(t: &TUID) -> String {
    t.iter().map(|b| format!("{:02X}", *b as u8)).collect()
}

pub fn tuid_from_hex(s: &str) -> Option<TUID> {
    if s.len() != 32 {
        return None;
    }
    let mut t: TUID = [0; 16];
    for (i, b) in t.iter_mut().enumerate() {
        *b = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok()? as std::ffi::c_char;
    }
    Some(t)
}

#[allow(dead_code)]
pub fn cstr_ptr(p: *const std::ffi::c_char) -> String {
    if p.is_null() {
        return String::new();
    }
    unsafe { CStr::from_ptr(p).to_string_lossy().into_owned() }
}
