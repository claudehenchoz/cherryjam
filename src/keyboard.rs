//! Computer keyboard → notes.
//!
//! Keys are matched by *physical* position, so the layout works the same on QWERTY, QWERTZ and
//! AZERTY keyboards:
//!
//! ```text
//!  lower octave:   S D   G H J   L ;          upper octave:  2 3   5 6 7   9 0   =
//!                 Z X C V B N M , . /                       Q W E R T Y U I O P [ ]
//! ```

use std::collections::HashMap;

use egui::Key;

use crate::audio::{Msg, MsgSender};

/// Semitone offset (from the base note) for each physical key.
pub const KEYMAP: &[(Key, u8)] = &[
    // Lower octave: Z row = white keys, A row = black keys.
    (Key::Z, 0),
    (Key::S, 1),
    (Key::X, 2),
    (Key::D, 3),
    (Key::C, 4),
    (Key::V, 5),
    (Key::G, 6),
    (Key::B, 7),
    (Key::H, 8),
    (Key::N, 9),
    (Key::J, 10),
    (Key::M, 11),
    (Key::Comma, 12),
    (Key::L, 13),
    (Key::Period, 14),
    (Key::Semicolon, 15),
    (Key::Slash, 16),
    // Upper octave: Q row = white keys, number row = black keys.
    (Key::Q, 12),
    (Key::Num2, 13),
    (Key::W, 14),
    (Key::Num3, 15),
    (Key::E, 16),
    (Key::R, 17),
    (Key::Num5, 18),
    (Key::T, 19),
    (Key::Num6, 20),
    (Key::Y, 21),
    (Key::Num7, 22),
    (Key::U, 23),
    (Key::I, 24),
    (Key::Num9, 25),
    (Key::O, 26),
    (Key::Num0, 27),
    (Key::P, 28),
    (Key::OpenBracket, 29),
    (Key::Equals, 30),
    (Key::CloseBracket, 31),
];

/// Set when F11 is seen outside egui (Windows keyboard hook, Linux X11 poller); the app toggles
/// fullscreen on its next frame.
pub static FULLSCREEN_REQUESTED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// PC/AT scan code set 1 ↔ physical key (named after the US layout, like egui does). For these
/// keys the Linux evdev key codes are the same numbers.
#[rustfmt::skip]
pub(crate) const SCANCODES: &[(u32, Key)] = {
    use Key::*;
    &[
        (0x02, Num1), (0x03, Num2), (0x04, Num3), (0x05, Num4), (0x06, Num5),
        (0x07, Num6), (0x08, Num7), (0x09, Num8), (0x0A, Num9), (0x0B, Num0),
        (0x0C, Minus), (0x0D, Equals),
        (0x10, Q), (0x11, W), (0x12, E), (0x13, R), (0x14, T), (0x15, Y), (0x16, U),
        (0x17, I), (0x18, O), (0x19, P), (0x1A, OpenBracket), (0x1B, CloseBracket),
        (0x1E, A), (0x1F, S), (0x20, D), (0x21, F), (0x22, G), (0x23, H), (0x24, J),
        (0x25, K), (0x26, L), (0x27, Semicolon),
        (0x2C, Z), (0x2D, X), (0x2E, C), (0x2F, V), (0x30, B), (0x31, N), (0x32, M),
        (0x33, Comma), (0x34, Period), (0x35, Slash),
    ]
};

pub(crate) fn key_to_scancode(key: Key) -> Option<u32> {
    SCANCODES.iter().find(|(_, k)| *k == key).map(|(s, _)| *s)
}

pub fn offset_for(key: Key) -> Option<u8> {
    KEYMAP.iter().find(|(k, _)| *k == key).map(|(_, o)| *o)
}

/// Short label printed on the on-screen piano for a semitone offset (prefers the upper row
/// from the octave break on).
pub fn label_for(offset: u8) -> Option<&'static str> {
    let lower = KEYMAP.iter().take(17).find(|(_, o)| *o == offset);
    let upper = KEYMAP.iter().skip(17).find(|(_, o)| *o == offset);
    let key = if offset >= 12 {
        upper.or(lower)
    } else {
        lower.or(upper)
    };
    key.map(|(k, _)| key_label(*k))
}

/// Second label for keys reachable from both rows (offsets 12..=16).
pub fn alt_label_for(offset: u8) -> Option<&'static str> {
    if !(12..=16).contains(&offset) {
        return None;
    }
    KEYMAP
        .iter()
        .take(17)
        .find(|(_, o)| *o == offset)
        .map(|(k, _)| key_label(*k))
}

/// What is printed on the user's keycap at this physical position (e.g. `Y` for the
/// QWERTY-`Z` position on a German/Swiss keyboard).
fn key_label(key: Key) -> &'static str {
    use std::sync::OnceLock;
    static LABELS: OnceLock<HashMap<Key, &'static str>> = OnceLock::new();
    let labels = LABELS.get_or_init(|| {
        KEYMAP
            .iter()
            .map(|(k, _)| {
                let l = layout_label(*k).map_or(k.symbol_or_name(), |s| &*s.leak());
                (*k, l)
            })
            .collect()
    });
    labels.get(&key).copied().unwrap_or(key.symbol_or_name())
}

#[cfg(windows)]
fn layout_label(key: Key) -> Option<String> {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        MAPVK_VK_TO_CHAR, MAPVK_VSC_TO_VK, MapVirtualKeyW,
    };
    let sc = key_to_scancode(key)?;
    unsafe {
        let vk = MapVirtualKeyW(sc, MAPVK_VSC_TO_VK);
        // The high bit marks dead keys; the low word is the unshifted character.
        let ch = MapVirtualKeyW(vk, MAPVK_VK_TO_CHAR) & 0xFFFF;
        let c = char::from_u32(ch).filter(|c| !c.is_control() && *c != ' ')?;
        Some(c.to_uppercase().collect())
    }
}

#[cfg(target_os = "linux")]
fn layout_label(key: Key) -> Option<String> {
    use x11rb::protocol::xproto::ConnectionExt;
    let code = key_to_scancode(key)? + x11::KEYCODE_OFFSET;
    let (conn, _) = x11rb::connect(None).ok()?;
    let map = conn
        .get_keyboard_mapping(code as u8, 1)
        .ok()?
        .reply()
        .ok()?;
    let sym = *map.keysyms.first()?;
    let c = match sym {
        0x20..=0x7e | 0xa0..=0xff => char::from_u32(sym)?,
        0x0100_0000..=0x0110_ffff => char::from_u32(sym - 0x0100_0000)?,
        // Dead keys, common on European layouts.
        0xfe50 => '`',
        0xfe51 => '´',
        0xfe52 => '^',
        0xfe53 => '~',
        0xfe57 => '¨',
        _ => return None,
    };
    (!c.is_control() && c != ' ').then(|| c.to_uppercase().collect())
}

#[cfg(not(any(windows, target_os = "linux")))]
fn layout_label(_key: Key) -> Option<String> {
    None
}

pub struct KeyboardState {
    pub base_note: u8,
    pub velocity: f32,
    /// Notes held by computer keys, by key.
    held_keys: HashMap<Key, u8>,
    /// Count of sources (keys, mouse) holding each note, so overlapping sources don't cut notes.
    note_refs: [u8; 128],
    sender: MsgSender,
}

impl KeyboardState {
    pub fn new(sender: MsgSender) -> Self {
        KeyboardState {
            base_note: 48,
            velocity: 0.8,
            held_keys: HashMap::new(),
            note_refs: [0; 128],
            sender,
        }
    }

    pub fn is_note_on(&self, note: u8) -> bool {
        self.note_refs.get(note as usize).is_some_and(|&n| n > 0)
    }

    pub fn note_on(&mut self, note: u8) {
        if note > 127 {
            return;
        }
        let r = &mut self.note_refs[note as usize];
        if *r > 0 {
            // Retrigger: release first so the plugin sees a clean on/off pair.
            self.sender.send(Msg::NoteOff { pitch: note });
        }
        *r = r.saturating_add(1);
        self.sender.send(Msg::NoteOn {
            pitch: note,
            velocity: self.velocity,
        });
    }

    pub fn note_off(&mut self, note: u8) {
        if note > 127 {
            return;
        }
        let r = &mut self.note_refs[note as usize];
        if *r == 0 {
            return;
        }
        *r -= 1;
        if *r == 0 {
            self.sender.send(Msg::NoteOff { pitch: note });
        }
    }

    pub fn all_notes_off(&mut self) {
        for n in 0..128u8 {
            if self.note_refs[n as usize] > 0 {
                self.note_refs[n as usize] = 0;
                self.sender.send(Msg::NoteOff { pitch: n });
            }
        }
        self.held_keys.clear();
    }

    pub fn shift_octave(&mut self, delta: i32) {
        let new = (self.base_note as i32 + delta * 12).clamp(0, 96);
        self.base_note = new as u8;
    }

    /// Handles a physical key event. Returns true if the key was consumed.
    pub fn key_event(&mut self, key: Key, pressed: bool, repeat: bool) -> bool {
        match key {
            Key::ArrowLeft | Key::PageDown if pressed => {
                if !repeat {
                    self.shift_octave(-1);
                }
                return true;
            }
            Key::ArrowRight | Key::PageUp if pressed => {
                if !repeat {
                    self.shift_octave(1);
                }
                return true;
            }
            _ => {}
        }
        let Some(offset) = offset_for(key) else {
            return false;
        };
        if pressed {
            if !repeat && !self.held_keys.contains_key(&key) {
                let note = self.base_note.saturating_add(offset).min(127);
                self.held_keys.insert(key, note);
                self.note_on(note);
            }
        } else if let Some(note) = self.held_keys.remove(&key) {
            self.note_off(note);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keymap_layout() {
        assert_eq!(offset_for(Key::Z), Some(0));
        assert_eq!(offset_for(Key::M), Some(11));
        assert_eq!(offset_for(Key::Q), Some(12));
        assert_eq!(offset_for(Key::Comma), Some(12));
        assert_eq!(offset_for(Key::U), Some(23));
        assert_eq!(offset_for(Key::I), Some(24));
        assert_eq!(offset_for(Key::A), None);
        // Every offset in two octaves is reachable.
        for o in 0..=24 {
            assert!(KEYMAP.iter().any(|(_, x)| *x == o), "offset {o}");
        }
    }

    #[test]
    fn note_refcount() {
        let (tx, mut rx) = rtrb::RingBuffer::new(64);
        let sender = crate::audio::test_sender(tx);
        let mut k = KeyboardState::new(sender);
        k.key_event(Key::Z, true, false);
        k.key_event(Key::Z, true, true); // repeat ignored
        assert!(k.is_note_on(48));
        k.key_event(Key::Z, false, false);
        assert!(!k.is_note_on(48));
        let mut msgs = Vec::new();
        while let Ok(m) = rx.pop() {
            msgs.push(m);
        }
        assert_eq!(msgs.len(), 2);
    }
}

/// On Windows, keys pressed while the plugin editor has focus never reach egui. A thread-local
/// keyboard hook catches the note keys in that case so playing keeps working after clicking
/// into the plugin.
#[cfg(windows)]
pub mod hook {
    use std::sync::{Arc, OnceLock};

    use egui::Key;
    use parking_lot::Mutex;
    use windows_sys::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
    use windows_sys::Win32::System::Threading::GetCurrentThreadId;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, HC_ACTION, SetWindowsHookExW, WH_KEYBOARD,
    };

    use super::KeyboardState;

    struct Target {
        keyboard: Arc<Mutex<KeyboardState>>,
        ctx: egui::Context,
    }

    static TARGET: OnceLock<Target> = OnceLock::new();

    pub fn install(keyboard: Arc<Mutex<KeyboardState>>, ctx: egui::Context) {
        if TARGET.set(Target { keyboard, ctx }).is_ok() {
            unsafe {
                SetWindowsHookExW(
                    WH_KEYBOARD,
                    Some(proc),
                    std::ptr::null_mut(),
                    GetCurrentThreadId(),
                );
            }
        }
    }

    unsafe extern "system" fn proc(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
        if code == HC_ACTION as i32 && crate::vst3host::editor::is_child_focused() {
            let scancode = ((l >> 16) & 0xFF) as u32;
            let extended = (l >> 24) & 1 == 1;
            let was_down = (l >> 30) & 1 == 1;
            let released = (l >> 31) & 1 == 1;
            const F11: u32 = 0x57;
            if scancode == F11 && !extended {
                if !released && !was_down {
                    super::FULLSCREEN_REQUESTED.store(true, std::sync::atomic::Ordering::Relaxed);
                    if let Some(t) = TARGET.get() {
                        t.ctx.request_repaint();
                    }
                }
                return 1;
            }
            if let Some(key) = scancode_to_key(scancode, extended)
                && let Some(t) = TARGET.get()
                && t.keyboard
                    .lock()
                    .key_event(key, !released, was_down && !released)
            {
                t.ctx.request_repaint();
                return 1;
            }
        }
        unsafe { CallNextHookEx(std::ptr::null_mut(), code, w, l) }
    }

    fn scancode_to_key(sc: u32, extended: bool) -> Option<Key> {
        if extended {
            return match sc {
                0x4B => Some(Key::ArrowLeft),
                0x4D => Some(Key::ArrowRight),
                0x49 => Some(Key::PageUp),
                0x51 => Some(Key::PageDown),
                _ => None,
            };
        }
        super::SCANCODES
            .iter()
            .find(|(s, _)| *s == sc)
            .map(|(_, k)| *k)
    }
}

/// On Linux, keys are read straight from the X server instead of from window events.
///
/// Plugin editors are separate X11 windows that can take keyboard focus at any moment (some
/// plugins grab it as soon as they open). A background thread polls the physical key state every
/// couple of milliseconds while any CherryJam window (including the embedded editor) has focus,
/// so notes play no matter which of our windows holds focus.
#[cfg(target_os = "linux")]
pub mod x11 {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    use egui::Key;
    use parking_lot::Mutex;
    use x11rb::protocol::xproto::{ConnectionExt, Window};
    use x11rb::rust_connection::RustConnection;

    use super::{FULLSCREEN_REQUESTED, KeyboardState, SCANCODES};

    /// X11 keycodes are the evdev codes shifted by 8.
    pub const KEYCODE_OFFSET: u32 = 8;
    const F11: u32 = 87;
    const POLL: Duration = Duration::from_millis(2);

    /// The poller is running; window key events must not also play notes.
    pub static ACTIVE: AtomicBool = AtomicBool::new(false);
    /// A text field has keyboard focus: keys type instead of playing.
    pub static TEXT_INPUT: AtomicBool = AtomicBool::new(false);

    fn key_for(evdev: u32) -> Option<Key> {
        match evdev {
            105 => Some(Key::ArrowLeft),
            106 => Some(Key::ArrowRight),
            104 => Some(Key::PageUp),
            109 => Some(Key::PageDown),
            _ => SCANCODES.iter().find(|(s, _)| *s == evdev).map(|(_, k)| *k),
        }
    }

    pub fn start(keyboard: Arc<Mutex<KeyboardState>>, ctx: egui::Context, main_window: usize) {
        if main_window == 0 || ACTIVE.load(Ordering::Relaxed) {
            return;
        }
        let Ok((conn, _)) = x11rb::connect(None) else {
            return;
        };
        ACTIVE.store(true, Ordering::Relaxed);
        let spawned = std::thread::Builder::new()
            .name("cherryjam-keys".into())
            .spawn(move || run(&conn, &keyboard, &ctx, main_window as Window));
        if spawned.is_err() {
            ACTIVE.store(false, Ordering::Relaxed);
        }
    }

    /// True if `w` is `main` or one of its descendants (e.g. an embedded plugin editor).
    pub fn is_inside(conn: &RustConnection, mut w: Window, main: Window) -> bool {
        for _ in 0..64 {
            if w == main {
                return true;
            }
            // 0 = None, 1 = PointerRoot.
            if w <= 1 {
                return false;
            }
            let Some(tree) = conn.query_tree(w).ok().and_then(|c| c.reply().ok()) else {
                return false;
            };
            if tree.parent == 0 || tree.parent == tree.root {
                return false;
            }
            w = tree.parent;
        }
        false
    }

    fn run(
        conn: &RustConnection,
        keyboard: &Mutex<KeyboardState>,
        ctx: &egui::Context,
        main: Window,
    ) {
        let mut prev = [0u8; 32];
        let mut last_focus = Window::MAX;
        let mut ours = false;
        loop {
            std::thread::sleep(POLL);
            let Some(focus) = conn.get_input_focus().ok().and_then(|c| c.reply().ok()) else {
                continue;
            };
            if focus.focus != last_focus {
                last_focus = focus.focus;
                ours = is_inside(conn, focus.focus, main);
            }
            // Outside our windows (or while typing) everything counts as released, so notes never
            // hang when focus moves away mid-chord.
            let keys = if ours && !TEXT_INPUT.load(Ordering::Relaxed) {
                match conn.query_keymap().ok().and_then(|c| c.reply().ok()) {
                    Some(r) => r.keys,
                    None => continue,
                }
            } else {
                [0u8; 32]
            };
            if keys == prev {
                continue;
            }
            let mut changed = false;
            {
                let mut kb = keyboard.lock();
                for (byte, (&now, &before)) in keys.iter().zip(prev.iter()).enumerate() {
                    let diff = now ^ before;
                    for bit in (0..8).filter(|b| diff & (1 << b) != 0) {
                        let code = (byte * 8 + bit) as u32;
                        let pressed = now & (1 << bit) != 0;
                        let Some(evdev) = code.checked_sub(KEYCODE_OFFSET) else {
                            continue;
                        };
                        if evdev == F11 {
                            if pressed {
                                FULLSCREEN_REQUESTED.store(true, Ordering::Relaxed);
                                changed = true;
                            }
                        } else if let Some(key) = key_for(evdev) {
                            changed |= kb.key_event(key, pressed, false);
                        }
                    }
                }
            }
            prev = keys;
            if changed {
                ctx.request_repaint();
            }
        }
    }
}
