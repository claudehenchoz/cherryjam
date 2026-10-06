//! Keeps eframe's OpenGL context current on the UI thread.
//!
//! Plugin editors run on our UI thread, and many of them render with OpenGL. They make their own
//! context current and leave it that way. eframe assumes its context stays current, so after such
//! a plugin has run, egui would draw into the plugin's context and the CherryJam UI would stay
//! black. We remember our context at startup and switch back to it before every egui paint.

/// The GL context (and drawable) that was current when the app was created.
pub struct GlGuard(platform::Current);

impl GlGuard {
    /// Captures the context that is current right now (eframe's, when called from `App::new`).
    pub fn capture() -> Option<GlGuard> {
        platform::Current::get().map(GlGuard)
    }

    /// Makes our context current again if something else replaced it. Returns true if it had to.
    pub fn restore(&self) -> bool {
        if platform::Current::get().as_ref() == Some(&self.0) {
            return false;
        }
        self.0.make_current();
        true
    }
}

#[cfg(windows)]
mod platform {
    use windows_sys::Win32::Graphics::Gdi::HDC;
    use windows_sys::Win32::Graphics::OpenGL::{
        HGLRC, wglGetCurrentContext, wglGetCurrentDC, wglMakeCurrent,
    };

    #[derive(PartialEq)]
    pub struct Current {
        dc: HDC,
        rc: HGLRC,
    }

    impl Current {
        pub fn get() -> Option<Current> {
            unsafe {
                let rc = wglGetCurrentContext();
                (!rc.is_null()).then(|| Current {
                    dc: wglGetCurrentDC(),
                    rc,
                })
            }
        }

        pub fn make_current(&self) {
            unsafe { wglMakeCurrent(self.dc, self.rc) };
        }
    }
}

/// eframe uses GLX on X11 and falls back to EGL; whichever one is current at startup is ours.
/// Both libraries are loaded at runtime so neither becomes a hard dependency.
#[cfg(target_os = "linux")]
mod platform {
    use std::ffi::c_void;
    use std::sync::OnceLock;

    type P = *mut c_void;

    struct Glx {
        get_context: unsafe extern "C" fn() -> P,
        get_display: unsafe extern "C" fn() -> P,
        get_drawable: unsafe extern "C" fn() -> u64,
        make_current: unsafe extern "C" fn(P, u64, P) -> i32,
        _lib: libloading::Library,
    }

    struct Egl {
        get_context: unsafe extern "C" fn() -> P,
        get_display: unsafe extern "C" fn() -> P,
        get_surface: unsafe extern "C" fn(i32) -> P,
        make_current: unsafe extern "C" fn(P, P, P, P) -> u32,
        _lib: libloading::Library,
    }

    const EGL_DRAW: i32 = 0x3059;
    const EGL_READ: i32 = 0x305A;

    fn glx() -> Option<&'static Glx> {
        static GLX: OnceLock<Option<Glx>> = OnceLock::new();
        GLX.get_or_init(|| unsafe {
            let lib = libloading::Library::new("libGL.so.1").ok()?;
            Some(Glx {
                get_context: *lib.get(b"glXGetCurrentContext\0").ok()?,
                get_display: *lib.get(b"glXGetCurrentDisplay\0").ok()?,
                get_drawable: *lib.get(b"glXGetCurrentDrawable\0").ok()?,
                make_current: *lib.get(b"glXMakeCurrent\0").ok()?,
                _lib: lib,
            })
        })
        .as_ref()
    }

    fn egl() -> Option<&'static Egl> {
        static EGL: OnceLock<Option<Egl>> = OnceLock::new();
        EGL.get_or_init(|| unsafe {
            let lib = libloading::Library::new("libEGL.so.1").ok()?;
            Some(Egl {
                get_context: *lib.get(b"eglGetCurrentContext\0").ok()?,
                get_display: *lib.get(b"eglGetCurrentDisplay\0").ok()?,
                get_surface: *lib.get(b"eglGetCurrentSurface\0").ok()?,
                make_current: *lib.get(b"eglMakeCurrent\0").ok()?,
                _lib: lib,
            })
        })
        .as_ref()
    }

    #[derive(PartialEq)]
    pub enum Current {
        Glx {
            display: P,
            drawable: u64,
            context: P,
        },
        Egl {
            display: P,
            draw: P,
            read: P,
            context: P,
        },
    }

    impl Current {
        pub fn get() -> Option<Current> {
            unsafe {
                if let Some(g) = glx() {
                    let context = (g.get_context)();
                    if !context.is_null() {
                        return Some(Current::Glx {
                            display: (g.get_display)(),
                            drawable: (g.get_drawable)(),
                            context,
                        });
                    }
                }
                if let Some(e) = egl() {
                    let context = (e.get_context)();
                    if !context.is_null() {
                        return Some(Current::Egl {
                            display: (e.get_display)(),
                            draw: (e.get_surface)(EGL_DRAW),
                            read: (e.get_surface)(EGL_READ),
                            context,
                        });
                    }
                }
                None
            }
        }

        pub fn make_current(&self) {
            unsafe {
                match *self {
                    Current::Glx {
                        display,
                        drawable,
                        context,
                    } => {
                        if let Some(g) = glx() {
                            (g.make_current)(display, drawable, context);
                        }
                    }
                    Current::Egl {
                        display,
                        draw,
                        read,
                        context,
                    } => {
                        if let Some(e) = egl() {
                            (e.make_current)(display, draw, read, context);
                        }
                    }
                }
            }
        }
    }
}
