//! Embedding the plugin's editor (IPlugView) in a native child window of the main window.

use vst3::ComPtr;
use vst3::Steinberg::Vst::IEditControllerTrait;
use vst3::Steinberg::{
    IPlugFrame, IPlugView, IPlugViewContentScaleSupport, IPlugViewContentScaleSupportTrait,
    IPlugViewTrait, ViewRect, kResultOk, kResultTrue,
};

use super::plugin::Plugin;

pub struct Editor {
    view: ComPtr<IPlugView>,
    scale_support: Option<ComPtr<IPlugViewContentScaleSupport>>,
    child: usize,
    can_resize: bool,
    /// Content scale currently applied to the view.
    scale: f32,
    /// DPI scale of the window (pixels per point).
    dpi: f32,
    last_area: Option<[i32; 4]>,
    last_fit: bool,
    /// Size (physical px) the editor needs when it does not fit the area it was given.
    pub overflow: Option<(i32, i32)>,
}

/// Physical-pixel rectangle inside the main window's client area.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PxRect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Editor {
    pub fn open(plugin: &Plugin, parent: usize, dpi: f32) -> Result<Editor, String> {
        let ctl = plugin
            .controller
            .as_ref()
            .ok_or("plugin has no controller")?;
        unsafe {
            let view = ComPtr::from_raw(ctl.createView(vst3::Steinberg::Vst::ViewType::kEditor))
                .ok_or("plugin has no editor")?;
            if view.isPlatformTypeSupported(platform::PLATFORM_TYPE) != kResultTrue {
                return Err("plugin editor does not support this platform".into());
            }
            let child = platform::create_child(parent).ok_or("could not create editor window")?;
            plugin.frame.state.lock().child = child;
            view.setFrame(plugin.frame.as_com_ref::<IPlugFrame>().unwrap().as_ptr());

            let scale_support = view.cast::<IPlugViewContentScaleSupport>();
            if let Some(s) = &scale_support {
                s.setContentScaleFactor(dpi);
            }
            if view.attached(child as *mut _, platform::PLATFORM_TYPE) != kResultOk {
                view.setFrame(std::ptr::null_mut());
                platform::destroy_child(child);
                plugin.frame.state.lock().child = 0;
                return Err("plugin editor failed to attach".into());
            }
            let mut r: ViewRect = std::mem::zeroed();
            if view.getSize(&mut r) == kResultOk {
                let (w, h) = (r.right - r.left, r.bottom - r.top);
                plugin.frame.state.lock().size = Some((w, h));
                platform::resize_child(child, w, h);
            }
            let can_resize = view.canResize() == kResultTrue;
            Ok(Editor {
                view,
                scale_support,
                child,
                can_resize,
                scale: dpi,
                dpi,
                last_area: None,
                last_fit: false,
                overflow: None,
            })
        }
    }

    /// Fits/centres the editor inside `area`. With `fit`, non-resizable editors are scaled
    /// through IPlugViewContentScaleSupport where the plugin supports it.
    pub fn layout(&mut self, plugin: &Plugin, area: PxRect, fit: bool) {
        if area.w <= 0 || area.h <= 0 {
            return;
        }
        let key = [area.x, area.y, area.w, area.h];
        let size_now = plugin.frame.state.lock().size;
        if self.last_area == Some(key) && self.last_fit == fit {
            // Only the plugin itself may have changed size; re-centre.
            if let Some((w, h)) = size_now {
                self.place(area, w, h);
            }
            return;
        }
        self.last_area = Some(key);
        self.last_fit = fit;

        unsafe {
            if self.can_resize && fit {
                // Start from the window's DPI scale; shrink the content if even the editor's
                // minimum size does not fit.
                self.set_scale(self.dpi);
                let mut r = self.constrain(area.w, area.h);
                let (cw, ch) = (r.right - r.left, r.bottom - r.top);
                if (cw > area.w || ch > area.h) && self.scale_support.is_some() {
                    let k = (area.w as f32 / cw as f32).min(area.h as f32 / ch as f32);
                    self.set_scale((self.scale * k).max(0.25));
                    r = self.constrain(area.w, area.h);
                }
                if self.view.onSize(&mut r) == kResultOk {
                    plugin.frame.state.lock().size = Some((r.right - r.left, r.bottom - r.top));
                }
            } else if let (Some(ss), Some((w, h))) = (&self.scale_support, size_now) {
                let target = if fit {
                    let base_w = w as f32 / self.scale;
                    let base_h = h as f32 / self.scale;
                    (area.w as f32 / base_w)
                        .min(area.h as f32 / base_h)
                        .clamp(0.5, 4.0)
                } else {
                    self.dpi
                };
                if (target - self.scale).abs() > 0.01
                    && ss.setContentScaleFactor(target) == kResultOk
                {
                    self.scale = target;
                    // The plugin usually calls resizeView itself; ask for the size to be sure.
                    let mut r: ViewRect = std::mem::zeroed();
                    if self.view.getSize(&mut r) == kResultOk {
                        let (nw, nh) = (r.right - r.left, r.bottom - r.top);
                        if (nw, nh) != (w, h) {
                            self.view.onSize(&mut r);
                        }
                        plugin.frame.state.lock().size = Some((nw, nh));
                    }
                }
            }
        }
        let size = plugin.frame.state.lock().size;
        if let Some((w, h)) = size {
            self.overflow = (w > area.w || h > area.h).then_some((w, h));
            self.place(area, w, h);
        }
    }

    fn set_scale(&mut self, scale: f32) {
        if let Some(ss) = &self.scale_support
            && (scale - self.scale).abs() > 0.01
            && unsafe { ss.setContentScaleFactor(scale) } == kResultOk
        {
            self.scale = scale;
        }
    }

    fn constrain(&self, w: i32, h: i32) -> ViewRect {
        let mut r = ViewRect {
            left: 0,
            top: 0,
            right: w,
            bottom: h,
        };
        unsafe { self.view.checkSizeConstraint(&mut r) };
        r
    }

    /// Centres the editor in `area`. The window is clipped to the area so an editor that is
    /// larger than the space available never covers the piano or the side panel.
    fn place(&self, area: PxRect, w: i32, h: i32) {
        let (w, h) = (w.min(area.w), h.min(area.h));
        let x = area.x + (area.w - w) / 2;
        let y = area.y + (area.h - h) / 2;
        platform::place_child(self.child, x, y, w, h);
    }

    pub fn set_visible(&self, visible: bool) {
        platform::show_child(self.child, visible);
    }

    pub fn close(self, plugin: &Plugin) {
        unsafe {
            self.view.removed();
            self.view.setFrame(std::ptr::null_mut());
        }
        drop(self.scale_support);
        drop(self.view);
        plugin.frame.state.lock().child = 0;
        platform::destroy_child(self.child);
    }
}

pub fn resize_child(child: usize, w: i32, h: i32) {
    platform::resize_child(child, w, h);
}

pub use platform::{focus_parent, is_child_focused};

#[cfg(windows)]
mod platform {
    use std::sync::Once;
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetFocus, SetFocus};
    use windows_sys::Win32::UI::WindowsAndMessaging::*;

    pub const PLATFORM_TYPE: vst3::Steinberg::FIDString = vst3::Steinberg::kPlatformTypeHWND;

    const CLASS: &[u16] = &[
        'C' as u16, 'h' as u16, 'e' as u16, 'r' as u16, 'r' as u16, 'y' as u16, 'J' as u16,
        'a' as u16, 'm' as u16, 'P' as u16, 'l' as u16, 'u' as u16, 'g' as u16, 0,
    ];

    unsafe extern "system" fn wndproc(h: HWND, m: u32, w: WPARAM, l: LPARAM) -> LRESULT {
        unsafe { DefWindowProcW(h, m, w, l) }
    }

    static mut PARENT: usize = 0;

    pub fn create_child(parent: usize) -> Option<usize> {
        static REGISTER: Once = Once::new();
        unsafe {
            let hinst =
                windows_sys::Win32::System::LibraryLoader::GetModuleHandleW(std::ptr::null());
            REGISTER.call_once(|| {
                let wc = WNDCLASSW {
                    style: 0,
                    lpfnWndProc: Some(wndproc),
                    cbClsExtra: 0,
                    cbWndExtra: 0,
                    hInstance: hinst,
                    hIcon: std::ptr::null_mut(),
                    hCursor: LoadCursorW(std::ptr::null_mut(), IDC_ARROW),
                    hbrBackground: std::ptr::null_mut(),
                    lpszMenuName: std::ptr::null(),
                    lpszClassName: CLASS.as_ptr(),
                };
                RegisterClassW(&wc);
            });
            // Without WS_CLIPCHILDREN the GL surface of the main window paints over the plugin.
            let p = parent as HWND;
            let style = GetWindowLongPtrW(p, GWL_STYLE);
            SetWindowLongPtrW(p, GWL_STYLE, style | (WS_CLIPCHILDREN as isize));
            PARENT = parent;

            let hwnd = CreateWindowExW(
                0,
                CLASS.as_ptr(),
                std::ptr::null(),
                WS_CHILD | WS_VISIBLE | WS_CLIPCHILDREN | WS_CLIPSIBLINGS,
                0,
                0,
                1,
                1,
                p,
                std::ptr::null_mut(),
                hinst,
                std::ptr::null(),
            );
            (!hwnd.is_null()).then_some(hwnd as usize)
        }
    }

    pub fn destroy_child(child: usize) {
        unsafe { DestroyWindow(child as HWND) };
    }

    pub fn resize_child(child: usize, w: i32, h: i32) {
        unsafe {
            SetWindowPos(
                child as HWND,
                std::ptr::null_mut(),
                0,
                0,
                w,
                h,
                SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
    }

    pub fn place_child(child: usize, x: i32, y: i32, w: i32, h: i32) {
        unsafe {
            let mut r = std::mem::zeroed();
            GetWindowRect(child as HWND, &mut r);
            let mut pt = windows_sys::Win32::Foundation::POINT {
                x: r.left,
                y: r.top,
            };
            windows_sys::Win32::Graphics::Gdi::ScreenToClient(GetParent(child as HWND), &mut pt);
            if pt.x != x || pt.y != y || r.right - r.left != w || r.bottom - r.top != h {
                SetWindowPos(
                    child as HWND,
                    std::ptr::null_mut(),
                    x,
                    y,
                    w,
                    h,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }
        }
    }

    pub fn show_child(child: usize, visible: bool) {
        unsafe {
            let is = IsWindowVisible(child as HWND) != 0;
            if is != visible {
                ShowWindow(child as HWND, if visible { SW_SHOWNA } else { SW_HIDE });
            }
        }
    }

    /// True if keyboard focus is inside the plugin editor (not on the main window).
    pub fn is_child_focused() -> bool {
        unsafe {
            let parent = PARENT as HWND;
            let f = GetFocus();
            !parent.is_null() && !f.is_null() && f != parent && IsChild(parent, f) != 0
        }
    }

    pub fn focus_parent() {
        unsafe {
            let parent = PARENT as HWND;
            if !parent.is_null() {
                SetFocus(parent);
            }
        }
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use std::sync::OnceLock;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use x11rb::CURRENT_TIME;
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{
        ConfigureWindowAux, ConnectionExt, CreateWindowAux, InputFocus, WindowClass,
    };
    use x11rb::rust_connection::RustConnection;

    pub const PLATFORM_TYPE: vst3::Steinberg::FIDString =
        vst3::Steinberg::kPlatformTypeX11EmbedWindowID;

    fn conn() -> Option<&'static RustConnection> {
        static CONN: OnceLock<Option<RustConnection>> = OnceLock::new();
        CONN.get_or_init(|| x11rb::connect(None).ok().map(|(c, _)| c))
            .as_ref()
    }

    /// The main window, for focus handling.
    static PARENT: AtomicUsize = AtomicUsize::new(0);

    pub fn create_child(parent: usize) -> Option<usize> {
        let c = conn()?;
        PARENT.store(parent, Ordering::Relaxed);
        let id = c.generate_id().ok()?;
        c.create_window(
            x11rb::COPY_DEPTH_FROM_PARENT,
            id,
            parent as u32,
            0,
            0,
            1,
            1,
            0,
            WindowClass::INPUT_OUTPUT,
            x11rb::COPY_FROM_PARENT,
            &CreateWindowAux::new(),
        )
        .ok()?;
        c.map_window(id).ok()?;
        c.flush().ok()?;
        Some(id as usize)
    }

    pub fn destroy_child(child: usize) {
        if let Some(c) = conn() {
            let _ = c.destroy_window(child as u32);
            let _ = c.flush();
        }
    }

    pub fn resize_child(child: usize, w: i32, h: i32) {
        if let Some(c) = conn() {
            let aux = ConfigureWindowAux::new()
                .width(w.max(1) as u32)
                .height(h.max(1) as u32);
            let _ = c.configure_window(child as u32, &aux);
            let _ = c.flush();
        }
    }

    pub fn place_child(child: usize, x: i32, y: i32, w: i32, h: i32) {
        if let Some(c) = conn() {
            let aux = ConfigureWindowAux::new()
                .x(x)
                .y(y)
                .width(w.max(1) as u32)
                .height(h.max(1) as u32);
            let _ = c.configure_window(child as u32, &aux);
            let _ = c.flush();
        }
    }

    pub fn show_child(child: usize, visible: bool) {
        if let Some(c) = conn() {
            let _ = if visible {
                c.map_window(child as u32)
            } else {
                c.unmap_window(child as u32)
            };
            let _ = c.flush();
        }
    }

    /// True if keyboard focus is inside the plugin editor (not on the main window).
    pub fn is_child_focused() -> bool {
        let parent = PARENT.load(Ordering::Relaxed) as u32;
        let Some(c) = conn() else { return false };
        if parent == 0 {
            return false;
        }
        let Some(focus) = c.get_input_focus().ok().and_then(|r| r.reply().ok()) else {
            return false;
        };
        focus.focus != parent && crate::keyboard::x11::is_inside(c, focus.focus, parent)
    }

    pub fn focus_parent() {
        let parent = PARENT.load(Ordering::Relaxed) as u32;
        if let (Some(c), true) = (conn(), parent != 0) {
            let _ = c.set_input_focus(InputFocus::PARENT, parent, CURRENT_TIME);
            let _ = c.flush();
        }
    }
}
