//! The CherryJam window: plugin editor on top, piano below, side panel for everything else.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use egui::{Color32, RichText, Sense, Vec2};
use parking_lot::Mutex;

use crate::audio::{self, AudioOut, Msg, MsgSender, ProcessorSlot};
use crate::config::Config;
use crate::controller::{AxisMap, Mapping, XyState, caps_lock_on};
use crate::fx::FxSettings;
use crate::keyboard::KeyboardState;
use crate::piano::Piano;
use crate::preset::{self, Preset};
use crate::scan::{self, PluginEntry};
use crate::theme;
use crate::vst3host::editor::{self, Editor, PxRect};
use crate::vst3host::module::{tuid_from_hex, tuid_to_hex};
use crate::vst3host::plugin::{ParamInfo, Plugin};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Instruments,
    Presets,
    Effects,
    Controller,
    Settings,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Axis {
    X,
    Y,
}

struct Loaded {
    plugin: Plugin,
    editor: Option<Editor>,
    /// The window was already enlarged once to fit this editor.
    grown: bool,
    params: Vec<ParamInfo>,
    bundle: PathBuf,
}

enum Pending {
    Entry(PluginEntry),
    Preset(Preset),
}

pub struct App {
    config: Config,
    entries: Vec<PluginEntry>,
    filter: String,

    slot: ProcessorSlot,
    sender: Option<MsgSender>,
    audio: Option<AudioOut>,
    audio_error: Option<String>,

    keyboard: Arc<Mutex<KeyboardState>>,
    piano: Piano,
    loaded: Option<Loaded>,
    parent: usize,
    dpi: f32,
    pending: Option<(Pending, u8)>,

    fx: FxSettings,
    mapping: Mapping,
    xy: XyState,
    learning: Option<Axis>,
    param_picker: Option<Axis>,
    param_filter: String,

    presets: Vec<String>,
    preset_name: String,
    current_preset: Option<String>,

    tab: Tab,
    show_panel: bool,
    new_folder: String,
    status: String,
    window_focused: bool,
    /// eframe's GL context, restored before every paint (plugins may switch contexts).
    gl_guard: Option<crate::gl_guard::GlGuard>,
    gl_restore_logged: bool,
    /// App icon: simplified artwork for the top bar, full artwork for the welcome screen.
    logo_small: egui::TextureHandle,
    logo_large: egui::TextureHandle,
}

impl App {
    /// `open` is an optional `.vst3` bundle or preset `.json` given on the command line.
    pub fn new(cc: &eframe::CreationContext<'_>, open: Option<PathBuf>) -> Self {
        theme::apply(&cc.egui_ctx);
        let config = Config::load();

        let slot: ProcessorSlot = Arc::new(Mutex::new(None));
        let (audio, sender, audio_error) = match audio::start(slot.clone()) {
            Ok((a, s)) => (Some(a), Some(s), None),
            Err(e) => (None, None, Some(e)),
        };
        // Without audio there is still a UI; notes go nowhere.
        let sender = sender.unwrap_or_else(audio::null_sender);
        let mut kb = KeyboardState::new(sender.clone());
        kb.velocity = config.velocity;
        let keyboard = Arc::new(Mutex::new(kb));

        #[cfg(windows)]
        crate::keyboard::hook::install(keyboard.clone(), cc.egui_ctx.clone());

        let parent = parent_handle(cc);
        #[cfg(target_os = "linux")]
        crate::keyboard::x11::start(keyboard.clone(), cc.egui_ctx.clone(), parent);
        let entries = config.scan();

        let from_arg = open.and_then(|path| {
            if path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("json"))
            {
                preset::load_file(&path).ok().map(Pending::Preset)
            } else {
                let binary = scan::binary_path(&path)?;
                let name = path.file_stem()?.to_string_lossy().into_owned();
                Some(Pending::Entry(PluginEntry {
                    name,
                    vendor: String::new(),
                    bundle: path,
                    binary,
                    is_instrument: None,
                }))
            }
        });
        let pending = from_arg
            .or_else(|| {
                config
                    .last_preset
                    .as_deref()
                    .and_then(|n| preset::load(n).ok())
                    .map(Pending::Preset)
            })
            .map(|p| (p, 0));

        let mut app = App {
            entries,
            filter: String::new(),
            slot,
            sender: Some(sender),
            audio,
            audio_error,
            keyboard,
            piano: Piano::default(),
            loaded: None,
            parent,
            dpi: cc.egui_ctx.pixels_per_point(),
            pending,
            fx: FxSettings::default(),
            mapping: Mapping::default(),
            xy: XyState::default(),
            learning: None,
            param_picker: None,
            param_filter: String::new(),
            presets: preset::list(),
            preset_name: String::new(),
            current_preset: None,
            tab: Tab::Instruments,
            show_panel: true,
            new_folder: String::new(),
            status: String::new(),
            window_focused: true,
            gl_guard: crate::gl_guard::GlGuard::capture(),
            gl_restore_logged: false,
            logo_small: load_icon(
                &cc.egui_ctx,
                "logo-small",
                include_bytes!("../icons/png/cherryjam-48.png"),
            ),
            logo_large: load_icon(
                &cc.egui_ctx,
                "logo-large",
                include_bytes!("../icons/png/cherryjam-256.png"),
            ),
            config,
        };
        app.status = format!("Found {} plugins", app.entries.len());
        app
    }

    fn sender(&self) -> &MsgSender {
        self.sender.as_ref().unwrap()
    }

    fn sample_rate(&self) -> f64 {
        self.audio.as_ref().map_or(48000.0, |a| a.sample_rate)
    }

    // -----------------------------------------------------------------------------------------
    // Plugin lifecycle

    fn unload(&mut self) {
        self.keyboard.lock().all_notes_off();
        if let Some(mut l) = self.loaded.take() {
            if let Some(ed) = l.editor.take() {
                ed.close(&l.plugin);
            }
            let proc = self.slot.lock().take();
            l.plugin.shutdown(proc);
        }
    }

    fn load_entry(&mut self, entry: &PluginEntry, preset: Option<&Preset>) -> Result<(), String> {
        self.unload();
        let cid = preset.and_then(|p| tuid_from_hex(&p.class_id));
        let (plugin, proc) = Plugin::load(
            &entry.binary,
            cid,
            self.sample_rate(),
            self.sender().clone(),
        )?;
        if let Some(p) = preset {
            let (comp, ctl) = p.state();
            plugin.set_state(&comp, &ctl);
        }
        *self.slot.lock() = Some(proc);
        let params = plugin.params();
        let editor = if self.parent != 0 {
            match Editor::open(&plugin, self.parent, self.dpi) {
                Ok(e) => Some(e),
                Err(e) => {
                    self.status = format!("{}: {e}", plugin.name);
                    None
                }
            }
        } else {
            None
        };
        if !plugin.is_instrument && !self.config.known_effects.contains(&entry.bundle) {
            self.config.known_effects.push(entry.bundle.clone());
            self.config.save();
            if let Some(e) = self.entries.iter_mut().find(|e| e.bundle == entry.bundle) {
                e.is_instrument = Some(false);
            }
        }
        self.loaded = Some(Loaded {
            plugin,
            editor,
            grown: false,
            params,
            bundle: entry.bundle.clone(),
        });
        self.sync_xy_from_params();
        Ok(())
    }

    fn apply_preset(&mut self, p: Preset) {
        let Some(binary) = scan::binary_path(&p.plugin_path) else {
            self.status = format!(
                "Preset \"{}\": plugin not found at {}",
                p.name,
                p.plugin_path.display()
            );
            return;
        };
        let entry = PluginEntry {
            name: p.plugin_name.clone(),
            vendor: String::new(),
            bundle: p.plugin_path.clone(),
            binary,
            is_instrument: None,
        };
        self.mapping = p.mapping.clone();
        self.fx = p.fx;
        self.sender().send(Msg::Fx(self.fx));
        match self.load_entry(&entry, Some(&p)) {
            Ok(()) => {
                self.status = format!("Loaded preset \"{}\"", p.name);
                self.preset_name = p.name.clone();
                self.current_preset = Some(p.name.clone());
                self.config.last_preset = Some(p.name);
                self.config.save();
            }
            Err(e) => self.status = format!("Could not load {}: {e}", p.plugin_name),
        }
    }

    fn save_preset(&mut self) {
        let name = self.preset_name.trim().to_string();
        if name.is_empty() {
            self.status = "Enter a preset name first".into();
            return;
        }
        let Some(l) = &self.loaded else {
            self.status = "Load an instrument first".into();
            return;
        };
        let mut p = Preset {
            name: name.clone(),
            plugin_name: l.plugin.name.clone(),
            plugin_path: l.bundle.clone(),
            class_id: tuid_to_hex(&l.plugin.cid),
            mapping: self.mapping.clone(),
            fx: self.fx,
            ..Default::default()
        };
        let (comp, ctl) = l.plugin.get_state();
        p.set_state(&comp, &ctl);
        match preset::save(&p) {
            Ok(()) => {
                self.status = format!("Saved preset \"{name}\"");
                self.presets = preset::list();
                self.current_preset = Some(name.clone());
                self.config.last_preset = Some(name);
                self.config.save();
            }
            Err(e) => self.status = format!("Could not save preset: {e}"),
        }
    }

    fn run_pending(&mut self, ctx: &egui::Context) {
        // Give the UI a couple of frames to show "Loading…" before blocking on the plugin.
        let Some((_, frames)) = &mut self.pending else {
            return;
        };
        if *frames < 2 {
            *frames += 1;
            ctx.request_repaint();
            return;
        }
        let (pending, _) = self.pending.take().unwrap();
        match pending {
            Pending::Entry(e) => {
                let name = e.name.clone();
                match self.load_entry(&e, None) {
                    Ok(()) => {
                        let instrument =
                            self.loaded.as_ref().is_some_and(|l| l.plugin.is_instrument);
                        self.status = if instrument {
                            format!("Loaded {name}")
                        } else {
                            format!(
                                "Loaded {name} (not an instrument, it may not respond to notes)"
                            )
                        };
                        self.current_preset = None;
                        self.mapping = Mapping::default();
                    }
                    Err(err) => self.status = format!("Could not load {name}: {err}"),
                }
            }
            Pending::Preset(p) => self.apply_preset(p),
        }
    }

    // -----------------------------------------------------------------------------------------
    // XY controller

    fn sync_xy_from_params(&mut self) {
        let Some(l) = &self.loaded else { return };
        for (i, axis) in [&self.mapping.x, &self.mapping.y].into_iter().enumerate() {
            if let Some(id) = axis.param_id {
                self.xy.pos[i] = axis.position(l.plugin.param_value(id));
            }
        }
    }

    fn send_axis(&self, axis: &AxisMap, pos: f32) {
        let (Some(l), Some(id)) = (&self.loaded, axis.param_id) else {
            return;
        };
        let v = axis.value(pos);
        l.plugin.set_param_ui(id, v);
        self.sender().send(Msg::Param { id, value: v });
    }

    fn update_controller(&mut self, ctx: &egui::Context) {
        let caps = caps_lock_on();
        if caps != self.xy.engaged {
            self.xy.engaged = caps;
            if caps {
                self.sync_xy_from_params();
                editor::focus_parent();
                capture::begin(self.parent, ctx);
            } else {
                capture::end(self.parent, ctx);
            }
        }
        if !self.xy.engaged {
            // Follow knob changes made in the plugin GUI so engaging never jumps.
            self.sync_xy_from_params();
            return;
        }
        let (dx, dy) = capture::delta(self.parent, ctx);
        if (dx != 0.0 || dy != 0.0) && self.xy.apply_delta(&self.mapping, dx, dy) {
            self.send_axis(&self.mapping.x, self.xy.pos[0]);
            self.send_axis(&self.mapping.y, self.xy.pos[1]);
        }
        // Smooth control while engaged.
        ctx.request_repaint();
    }

    fn poll_plugin_events(&mut self) {
        let Some(l) = &mut self.loaded else { return };
        #[cfg(target_os = "linux")]
        l.plugin.frame.run_loop.pump();
        let h = &l.plugin.handler;
        let flags = h.restart_flags.swap(0, Ordering::Relaxed);
        if flags & vst3::Steinberg::Vst::RestartFlags_::kParamTitlesChanged as i64 != 0 {
            l.params = l.plugin.params();
        }
        if let Some(axis) = self.learning {
            let id = h.last_touched.swap(-1, Ordering::Relaxed);
            if id >= 0 {
                let id = id as u32;
                let name = l
                    .params
                    .iter()
                    .find(|p| p.id == id)
                    .map_or(format!("#{id}"), |p| p.title.clone());
                let a = match axis {
                    Axis::X => &mut self.mapping.x,
                    Axis::Y => &mut self.mapping.y,
                };
                a.param_id = Some(id);
                a.param_name = name;
                self.learning = None;
                self.sync_xy_from_params();
            }
        }
    }

    // -----------------------------------------------------------------------------------------
    // Keyboard

    fn handle_keys(&mut self, ctx: &egui::Context) {
        // A click on our own UI must take keyboard focus back from the plugin editor; Windows
        // leaves it on the child window otherwise, and typing would end up as notes.
        if ctx.input(|i| i.pointer.any_pressed()) && editor::is_child_focused() {
            editor::focus_parent();
        }
        // F11 toggles fullscreen, even while typing in a text field.
        let f11 = ctx.input(|i| {
            i.events.iter().any(|e| {
                matches!(
                    e,
                    egui::Event::Key {
                        key: egui::Key::F11,
                        pressed: true,
                        repeat: false,
                        ..
                    }
                )
            })
        });
        let typing = ctx.egui_wants_keyboard_input();
        // With the X11 poller running, it is the only source of key presses (F11 included).
        #[cfg(target_os = "linux")]
        let polled = {
            crate::keyboard::x11::TEXT_INPUT.store(typing, Ordering::Relaxed);
            crate::keyboard::x11::ACTIVE.load(Ordering::Relaxed)
        };
        #[cfg(not(target_os = "linux"))]
        let polled = false;

        let hooked = crate::keyboard::FULLSCREEN_REQUESTED.swap(false, Ordering::Relaxed);
        if (f11 && !polled) || hooked {
            toggle_fullscreen(ctx);
        }
        if polled {
            // The poller releases everything by itself when focus leaves our windows.
            return;
        }
        let focused = ctx.input(|i| i.focused);
        if self.window_focused && !focused && !editor::is_child_focused() {
            // Key-ups would get lost while another app has focus.
            self.keyboard.lock().all_notes_off();
        }
        self.window_focused = focused;
        if typing {
            return;
        }
        let events = ctx.input(|i| i.events.clone());
        let mut kb = self.keyboard.lock();
        for e in events {
            if let egui::Event::Key {
                key,
                physical_key,
                pressed,
                repeat,
                modifiers,
            } = e
            {
                if modifiers.ctrl || modifiers.alt || modifiers.command {
                    continue;
                }
                kb.key_event(physical_key.unwrap_or(key), pressed, repeat);
            }
        }
    }

    // -----------------------------------------------------------------------------------------
    // UI

    fn top_bar(&mut self, root: &mut egui::Ui) {
        egui::Panel::top("top").exact_size(40.0).show(root, |ui| {
            ui.horizontal_centered(|ui| {
                ui.add(egui::Image::new(&self.logo_small).fit_to_exact_size(Vec2::splat(24.0)));
                ui.label(
                    RichText::new("CherryJam")
                        .size(18.0)
                        .strong()
                        .color(theme::ACCENT),
                );
                ui.separator();
                match &self.loaded {
                    Some(l) => {
                        ui.label(RichText::new(&l.plugin.name).size(15.0).strong());
                    }
                    None => {
                        ui.label(RichText::new("No instrument").weak());
                    }
                }
                if let Some(p) = &self.current_preset {
                    ui.label(RichText::new(format!("· {p}")).weak());
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let label = if self.show_panel {
                        "Hide panel ▶"
                    } else {
                        "◀ Panel"
                    };
                    if ui.button(label).clicked() {
                        self.show_panel = !self.show_panel;
                    }
                    let fullscreen = ui.ctx().input(|i| i.viewport().fullscreen.unwrap_or(false));
                    let tip = if fullscreen {
                        "Exit fullscreen (F11)"
                    } else {
                        "Fullscreen (F11)"
                    };
                    if ui.button("⛶").on_hover_text(tip).clicked() {
                        toggle_fullscreen(ui.ctx());
                    }
                    ui.separator();
                    let (text, col) = if self.xy.engaged {
                        ("XY ACTIVE", theme::ACCENT)
                    } else {
                        ("XY: Caps Lock", Color32::GRAY)
                    };
                    ui.label(RichText::new(text).color(col).strong());
                    ui.separator();
                    let mut kb = self.keyboard.lock();
                    ui.add(egui::Slider::new(&mut kb.velocity, 0.05..=1.0).show_value(false));
                    ui.label("Velocity");
                    if (kb.velocity - self.config.velocity).abs() > f32::EPSILON {
                        self.config.velocity = kb.velocity;
                    }
                    ui.separator();
                    if ui
                        .small_button("▶")
                        .on_hover_text("Octave up (→ / PgUp)")
                        .clicked()
                    {
                        kb.shift_octave(1);
                    }
                    ui.label(format!("C{}", kb.base_note as i32 / 12 - 1));
                    if ui
                        .small_button("◀")
                        .on_hover_text("Octave down (← / PgDn)")
                        .clicked()
                    {
                        kb.shift_octave(-1);
                    }
                    ui.label("Octave");
                });
            });
        });
    }

    fn bottom_bar(&mut self, root: &mut egui::Ui) {
        egui::Panel::bottom("status")
            .exact_size(22.0)
            .show(root, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.label(RichText::new(&self.status).small().weak());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        match (&self.audio, &self.audio_error) {
                            (Some(a), _) => ui.label(
                                RichText::new(format!("{} · {} Hz", a.device_name, a.sample_rate))
                                    .small()
                                    .weak(),
                            ),
                            (None, Some(e)) => ui.label(
                                RichText::new(format!("Audio: {e}"))
                                    .small()
                                    .color(theme::ACCENT),
                            ),
                            _ => ui.label(""),
                        };
                    });
                });
            });
        egui::Panel::bottom("piano")
            .exact_size(130.0)
            .frame(
                egui::Frame::NONE
                    .fill(Color32::from_rgb(0x12, 0x11, 0x15))
                    .inner_margin(8.0),
            )
            .show(root, |ui| {
                let mut kb = self.keyboard.lock();
                self.piano.ui(ui, &mut kb, 114.0);
            });
    }

    fn side_panel(&mut self, root: &mut egui::Ui) {
        if !self.show_panel {
            return;
        }
        egui::Panel::right("side")
            .resizable(true)
            .default_size(320.0)
            .size_range(260.0..=520.0)
            .show(root, |ui| {
                ui.add_space(6.0);
                ui.horizontal_wrapped(|ui| {
                    for (t, name) in [
                        (Tab::Instruments, "Instruments"),
                        (Tab::Presets, "Presets"),
                        (Tab::Effects, "Effects"),
                        (Tab::Controller, "XY"),
                        (Tab::Settings, "Settings"),
                    ] {
                        ui.selectable_value(&mut self.tab, t, name);
                    }
                });
                ui.separator();
                match self.tab {
                    Tab::Instruments => self.instruments_tab(ui),
                    Tab::Presets => self.presets_tab(ui),
                    Tab::Effects => self.effects_tab(ui),
                    Tab::Controller => self.controller_tab(ui),
                    Tab::Settings => self.settings_tab(ui),
                }
            });
    }

    fn instruments_tab(&mut self, ui: &mut egui::Ui) {
        ui.add(egui::TextEdit::singleline(&mut self.filter).hint_text("🔍 Search instruments"));
        ui.add_space(4.0);
        let f = self.filter.to_lowercase();
        let show_all = self.config.show_all_plugins;
        let current = self.loaded.as_ref().map(|l| l.bundle.clone());
        let mut clicked = None;
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                ui.with_layout(egui::Layout::top_down_justified(egui::Align::LEFT), |ui| {
                    for e in &self.entries {
                        if !show_all && e.is_instrument == Some(false) {
                            continue;
                        }
                        if !f.is_empty()
                            && !e.name.to_lowercase().contains(&f)
                            && !e.vendor.to_lowercase().contains(&f)
                        {
                            continue;
                        }
                        let selected = current.as_ref() == Some(&e.bundle);
                        let mut text = RichText::new(&e.name);
                        if e.is_instrument == Some(false) {
                            text = text.weak();
                        }
                        let r = ui
                            .selectable_label(selected, text)
                            .on_hover_text(e.bundle.display().to_string());
                        if r.clicked() {
                            clicked = Some(e.clone());
                        }
                    }
                });
            });
        if let Some(e) = clicked {
            self.status = format!("Loading {}…", e.name);
            self.pending = Some((Pending::Entry(e), 0));
        }
    }

    fn presets_tab(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.preset_name)
                    .hint_text("Preset name")
                    .desired_width(170.0),
            );
            if ui
                .add_enabled(self.loaded.is_some(), egui::Button::new("💾 Save"))
                .clicked()
            {
                self.save_preset();
            }
        });
        ui.label(
            RichText::new("Saves instrument + sound, XY mapping and effects.")
                .small()
                .weak(),
        );
        ui.separator();
        let mut load = None;
        let mut delete = None;
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                if self.presets.is_empty() {
                    ui.label(RichText::new("No presets yet.").weak());
                }
                for name in &self.presets {
                    ui.horizontal(|ui| {
                        let selected = self.current_preset.as_ref() == Some(name);
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui
                                .small_button("🗑")
                                .on_hover_text("Delete preset")
                                .clicked()
                            {
                                delete = Some(name.clone());
                            }
                            ui.with_layout(
                                egui::Layout::top_down_justified(egui::Align::LEFT),
                                |ui| {
                                    if ui.selectable_label(selected, name).clicked() {
                                        load = Some(name.clone());
                                    }
                                },
                            );
                        });
                    });
                }
            });
        if let Some(n) = load {
            match preset::load(&n) {
                Ok(p) => {
                    self.status = format!("Loading preset \"{n}\"…");
                    self.pending = Some((Pending::Preset(p), 0));
                }
                Err(e) => self.status = format!("Could not read preset: {e}"),
            }
        }
        if let Some(n) = delete {
            match preset::delete(&n) {
                Ok(()) => self.status = format!("Deleted preset \"{n}\""),
                Err(e) => self.status = format!("Could not delete: {e}"),
            }
            self.presets = preset::list();
            if self.current_preset.as_ref() == Some(&n) {
                self.current_preset = None;
            }
        }
    }

    fn effects_tab(&mut self, ui: &mut egui::Ui) {
        let before = self.fx;
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                let d = &mut self.fx.delay;
                section(ui, "Delay", &mut d.enabled, |ui| {
                    egui::Grid::new("delay").num_columns(2).show(ui, |ui| {
                        ui.label("Time");
                        ui.add(
                            egui::Slider::new(&mut d.time_ms, 10.0..=2000.0)
                                .suffix(" ms")
                                .logarithmic(true),
                        );
                        ui.end_row();
                        ui.label("Feedback");
                        ui.add(egui::Slider::new(&mut d.feedback, 0.0..=0.95));
                        ui.end_row();
                        ui.label("Tone");
                        ui.add(
                            egui::Slider::new(&mut d.tone_hz, 300.0..=18000.0)
                                .suffix(" Hz")
                                .logarithmic(true),
                        );
                        ui.end_row();
                        ui.label("Mix");
                        ui.add(egui::Slider::new(&mut d.mix, 0.0..=1.0));
                        ui.end_row();
                        ui.label("Ping-pong");
                        ui.checkbox(&mut d.ping_pong, "");
                        ui.end_row();
                    });
                });
                ui.add_space(8.0);
                let r = &mut self.fx.reverb;
                section(ui, "Reverb", &mut r.enabled, |ui| {
                    egui::Grid::new("reverb").num_columns(2).show(ui, |ui| {
                        ui.label("Size");
                        ui.add(egui::Slider::new(&mut r.size, 0.0..=1.0));
                        ui.end_row();
                        ui.label("Damping");
                        ui.add(egui::Slider::new(&mut r.damping, 0.0..=1.0));
                        ui.end_row();
                        ui.label("Width");
                        ui.add(egui::Slider::new(&mut r.width, 0.0..=1.0));
                        ui.end_row();
                        ui.label("Pre-delay");
                        ui.add(egui::Slider::new(&mut r.predelay_ms, 0.0..=200.0).suffix(" ms"));
                        ui.end_row();
                        ui.label("Mix");
                        ui.add(egui::Slider::new(&mut r.mix, 0.0..=1.0));
                        ui.end_row();
                    });
                });
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.label("Master");
                    let mut db = 20.0 * self.fx.gain.max(1e-4).log10();
                    if ui
                        .add(egui::Slider::new(&mut db, -30.0..=6.0).suffix(" dB"))
                        .changed()
                    {
                        self.fx.gain = 10f32.powf(db / 20.0);
                    }
                });
            });
        if self.fx != before {
            self.sender().send(Msg::Fx(self.fx));
        }
    }

    fn controller_tab(&mut self, ui: &mut egui::Ui) {
        ui.label(
            RichText::new("Turn on Caps Lock: the mouse controls the two mapped parameters. Turn it off to get the cursor back.")
                .small()
                .weak(),
        );
        ui.add_space(4.0);
        if self.loaded.is_none() {
            ui.label(RichText::new("Load an instrument to map its parameters.").weak());
            return;
        }
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                self.xy_pad(ui);
                ui.add_space(6.0);
                for axis in [Axis::X, Axis::Y] {
                    self.axis_ui(ui, axis);
                    ui.add_space(6.0);
                }
                ui.horizontal(|ui| {
                    ui.label("Sensitivity");
                    ui.add(
                        egui::Slider::new(&mut self.mapping.sensitivity, 0.2..=6.0)
                            .logarithmic(true),
                    );
                });
            });
    }

    fn xy_pad(&mut self, ui: &mut egui::Ui) {
        let size = ui.available_width().min(220.0);
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
        let p = ui.painter_at(rect);
        p.rect_filled(rect, 8.0, ui.visuals().extreme_bg_color);
        for i in 1..4 {
            let t = i as f32 / 4.0;
            let c = Color32::from_gray(45);
            p.line_segment(
                [
                    rect.lerp_inside(Vec2::new(t, 0.0)),
                    rect.lerp_inside(Vec2::new(t, 1.0)),
                ],
                (1.0, c),
            );
            p.line_segment(
                [
                    rect.lerp_inside(Vec2::new(0.0, t)),
                    rect.lerp_inside(Vec2::new(1.0, t)),
                ],
                (1.0, c),
            );
        }
        let dot = rect.lerp_inside(Vec2::new(self.xy.pos[0], 1.0 - self.xy.pos[1]));
        let col = if self.xy.engaged {
            theme::ACCENT
        } else {
            Color32::GRAY
        };
        p.circle_filled(dot, 7.0, col);
        p.circle_stroke(dot, 12.0, (1.5, col.gamma_multiply(0.5)));
    }

    fn axis_ui(&mut self, ui: &mut egui::Ui, axis: Axis) {
        let title = match axis {
            Axis::X => "X axis (left ↔ right)",
            Axis::Y => "Y axis (down ↕ up)",
        };
        let Some(l) = &self.loaded else { return };
        let current = {
            let a = if axis == Axis::X {
                &self.mapping.x
            } else {
                &self.mapping.y
            };
            a.param_id.map(|id| {
                let v = l.plugin.param_value(id);
                (a.param_name.clone(), l.plugin.param_display(id, v))
            })
        };
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(title).strong());
            match &current {
                Some((name, val)) => ui.label(format!("{name}  =  {val}")),
                None => ui.label(RichText::new("not mapped").weak()),
            };
            ui.horizontal(|ui| {
                let learning = self.learning == Some(axis);
                let learn_text = if learning {
                    "Move a knob in the plugin…"
                } else {
                    "🎯 Learn"
                };
                if ui.selectable_label(learning, learn_text).clicked() {
                    self.learning = if learning { None } else { Some(axis) };
                    if let Some(l) = &self.loaded {
                        l.plugin.handler.last_touched.store(-1, Ordering::Relaxed);
                    }
                }
                let picking = self.param_picker == Some(axis);
                if ui.selectable_label(picking, "📋 Choose").clicked() {
                    self.param_picker = if picking { None } else { Some(axis) };
                    self.param_filter.clear();
                }
                if current.is_some() && ui.button("✖").on_hover_text("Clear mapping").clicked() {
                    *self.axis_mut(axis) = AxisMap::default();
                }
            });
            if self.param_picker == Some(axis) {
                self.param_list(ui, axis);
            }
            let a = self.axis_mut(axis);
            ui.horizontal(|ui| {
                ui.label("Range");
                ui.add(
                    egui::DragValue::new(&mut a.min)
                        .range(0.0..=1.0)
                        .speed(0.005)
                        .fixed_decimals(2),
                );
                ui.label("–");
                ui.add(
                    egui::DragValue::new(&mut a.max)
                        .range(0.0..=1.0)
                        .speed(0.005)
                        .fixed_decimals(2),
                );
                ui.checkbox(&mut a.invert, "Invert");
            });
        });
    }

    fn axis_mut(&mut self, axis: Axis) -> &mut AxisMap {
        match axis {
            Axis::X => &mut self.mapping.x,
            Axis::Y => &mut self.mapping.y,
        }
    }

    fn param_list(&mut self, ui: &mut egui::Ui, axis: Axis) {
        let Some(l) = &self.loaded else { return };
        ui.add(
            egui::TextEdit::singleline(&mut self.param_filter).hint_text("🔍 Filter parameters"),
        );
        let f = self.param_filter.to_lowercase();
        let mut chosen = None;
        egui::ScrollArea::vertical()
            .id_salt(("params", idx_of(axis)))
            .max_height(220.0)
            .show(ui, |ui| {
                for p in l
                    .params
                    .iter()
                    .filter(|p| f.is_empty() || p.title.to_lowercase().contains(&f))
                {
                    let text = if p.units.is_empty() {
                        p.title.clone()
                    } else {
                        format!("{} [{}]", p.title, p.units)
                    };
                    if ui.selectable_label(false, text).clicked() {
                        chosen = Some(p.clone());
                    }
                }
            });
        if let Some(p) = chosen {
            let a = self.axis_mut(axis);
            a.param_id = Some(p.id);
            a.param_name = p.title;
            self.param_picker = None;
            self.sync_xy_from_params();
        }
    }

    fn settings_tab(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Plugin folders").strong());
        for f in scan::default_folders() {
            ui.label(RichText::new(f.display().to_string()).small().weak());
        }
        let mut remove = None;
        for (i, f) in self.config.extra_folders.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.label(RichText::new(f.display().to_string()).small());
                if ui.small_button("✖").clicked() {
                    remove = Some(i);
                }
            });
        }
        let mut changed = false;
        if let Some(i) = remove {
            self.config.extra_folders.remove(i);
            changed = true;
        }
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.new_folder)
                    .hint_text("Add folder path")
                    .desired_width(200.0),
            );
            if ui.button("Add").clicked() && !self.new_folder.trim().is_empty() {
                self.config
                    .extra_folders
                    .push(PathBuf::from(self.new_folder.trim()));
                self.new_folder.clear();
                changed = true;
            }
        });
        if ui.button("🔄 Rescan").clicked() || changed {
            self.entries = self.config.scan();
            self.status = format!("Found {} plugins", self.entries.len());
            self.config.save();
        }
        ui.separator();
        if ui
            .checkbox(
                &mut self.config.show_all_plugins,
                "Also list effects / unknown plugins",
            )
            .changed()
        {
            self.config.save();
        }
        if ui
            .checkbox(&mut self.config.fit_editor, "Scale plugin window to fit")
            .changed()
        {
            self.config.save();
        }
        ui.separator();
        ui.label(RichText::new("Keyboard").strong());
        ui.label(RichText::new(
            "Lower octave: Z-row (white) + A-row (black)\nUpper octave: Q-row (white) + number row (black)\nOctave: ← → or PgDn/PgUp\nXY controller: Caps Lock",
        ).small());
    }

    fn central(&mut self, root: &mut egui::Ui) {
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(Color32::from_rgb(0x0E, 0x0D, 0x10)))
            .show(root, |ui| {
                let rect = ui.max_rect();
                if self.pending.is_some() {
                    ui.centered_and_justified(|ui| {
                        ui.label(RichText::new(&self.status).size(20.0).color(theme::ACCENT));
                    });
                } else if self.loaded.is_none() {
                    ui.vertical_centered(|ui| {
                        ui.add_space(((rect.height() - 180.0) / 2.0).max(0.0));
                        ui.add(
                            egui::Image::new(&self.logo_large).fit_to_exact_size(Vec2::splat(96.0)),
                        );
                        ui.add_space(16.0);
                        ui.label(
                            RichText::new(
                                "Pick an instrument or a preset on the right,\nthen play with your keyboard.",
                            )
                            .size(18.0)
                            .weak(),
                        );
                    });
                } else if self.loaded.as_ref().is_some_and(|l| l.editor.is_none()) {
                    ui.centered_and_justified(|ui| {
                        ui.label(RichText::new("This instrument has no editor window.\nJust play!").size(16.0).weak());
                    });
                }

                let ctx = ui.ctx().clone();
                let ppp = ctx.pixels_per_point();
                self.dpi = ppp;
                // Native windows sit on top of everything egui draws, so hide the editor while
                // a popup or the "Loading" screen should be visible.
                let hide = self.pending.is_some() || ctx.any_popup_open();
                let fit = self.config.fit_editor;
                if let Some(l) = &mut self.loaded
                    && let Some(ed) = &mut l.editor {
                        ed.set_visible(!hide);
                        let area = PxRect {
                            x: (rect.left() * ppp).round() as i32,
                            y: (rect.top() * ppp).round() as i32,
                            w: (rect.width() * ppp).round() as i32,
                            h: (rect.height() * ppp).round() as i32,
                        };
                        ed.layout(&l.plugin, area, fit);
                        // Editor larger than the space we have: grow the window (once) so it fits.
                        if let (Some((w, h)), false) = (ed.overflow, l.grown) {
                            l.grown = true;
                            let (inner, monitor) =
                                ctx.input(|i| (i.viewport().inner_rect, i.viewport().monitor_size));
                            if let Some(inner) = inner {
                                let extra = Vec2::new(
                                    (w - area.w).max(0) as f32 / ppp,
                                    (h - area.h).max(0) as f32 / ppp,
                                );
                                let mut size = inner.size() + extra + Vec2::splat(2.0);
                                if let Some(m) = monitor {
                                    size = size.min(m * 0.95);
                                }
                                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
                            }
                        }
                    }
            });
    }
}

/// Mouse capture for the XY controller.
///
/// Windows: the cursor is hidden and snapped back to an anchor point every frame; the offset is
/// the delta. Unlike raw input this keeps working when the plugin editor holds keyboard focus.
/// Elsewhere: cursor lock + raw mouse motion from egui.
#[cfg(windows)]
mod capture {
    use std::sync::atomic::{AtomicI64, Ordering};
    use windows_sys::Win32::Foundation::POINT;
    use windows_sys::Win32::Graphics::Gdi::ClientToScreen;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetClientRect, GetCursorPos, SetCursorPos, ShowCursor,
    };

    /// Cursor position before engaging, packed as (x << 32 | y).
    static SAVED: AtomicI64 = AtomicI64::new(0);

    fn anchor(parent: usize) -> Option<POINT> {
        unsafe {
            let mut r = std::mem::zeroed();
            if parent == 0 || GetClientRect(parent as _, &mut r) == 0 {
                return None;
            }
            // Top bar, horizontally centred: clicking there by accident does nothing harmful.
            let mut p = POINT {
                x: (r.right - r.left) / 2,
                y: 20,
            };
            ClientToScreen(parent as _, &mut p);
            Some(p)
        }
    }

    pub fn begin(parent: usize, _ctx: &egui::Context) {
        unsafe {
            let mut p = POINT { x: 0, y: 0 };
            GetCursorPos(&mut p);
            SAVED.store(
                ((p.x as i64) << 32) | (p.y as u32 as i64),
                Ordering::Relaxed,
            );
            ShowCursor(0);
            if let Some(a) = anchor(parent) {
                SetCursorPos(a.x, a.y);
            }
        }
    }

    pub fn end(_parent: usize, _ctx: &egui::Context) {
        unsafe {
            let v = SAVED.load(Ordering::Relaxed);
            SetCursorPos((v >> 32) as i32, v as i32);
            ShowCursor(1);
        }
    }

    pub fn delta(parent: usize, _ctx: &egui::Context) -> (f32, f32) {
        let Some(a) = anchor(parent) else {
            return (0.0, 0.0);
        };
        unsafe {
            let mut p = POINT { x: 0, y: 0 };
            GetCursorPos(&mut p);
            SetCursorPos(a.x, a.y);
            ((p.x - a.x) as f32, (p.y - a.y) as f32)
        }
    }
}

#[cfg(not(windows))]
mod capture {
    pub fn begin(_parent: usize, ctx: &egui::Context) {
        ctx.send_viewport_cmd(egui::ViewportCommand::CursorGrab(egui::CursorGrab::Locked));
        ctx.send_viewport_cmd(egui::ViewportCommand::CursorVisible(false));
    }

    pub fn end(_parent: usize, ctx: &egui::Context) {
        ctx.send_viewport_cmd(egui::ViewportCommand::CursorGrab(egui::CursorGrab::None));
        ctx.send_viewport_cmd(egui::ViewportCommand::CursorVisible(true));
    }

    pub fn delta(_parent: usize, ctx: &egui::Context) -> (f32, f32) {
        ctx.input(|i| {
            i.raw.events.iter().fold((0.0, 0.0), |(x, y), e| match e {
                egui::Event::MouseMoved(d) => (x + d.x, y + d.y),
                _ => (x, y),
            })
        })
    }
}

fn toggle_fullscreen(ctx: &egui::Context) {
    let fullscreen = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
    ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(!fullscreen));
}

fn load_icon(ctx: &egui::Context, name: &str, png: &[u8]) -> egui::TextureHandle {
    let icon = eframe::icon_data::from_png_bytes(png).unwrap_or_default();
    let image = egui::ColorImage::from_rgba_unmultiplied(
        [icon.width as usize, icon.height as usize],
        &icon.rgba,
    );
    ctx.load_texture(name, image, egui::TextureOptions::LINEAR)
}

fn idx_of(axis: Axis) -> u8 {
    match axis {
        Axis::X => 0,
        Axis::Y => 1,
    }
}

fn section(ui: &mut egui::Ui, title: &str, enabled: &mut bool, body: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(RichText::new(title).strong().size(15.0));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.checkbox(enabled, if *enabled { "On" } else { "Off" });
            });
        });
        ui.add_enabled_ui(*enabled, body);
    });
}

fn parent_handle(cc: &eframe::CreationContext<'_>) -> usize {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    match cc.window_handle().map(|h| h.as_raw()) {
        #[cfg(windows)]
        Ok(RawWindowHandle::Win32(h)) => h.hwnd.get() as usize,
        #[cfg(target_os = "linux")]
        Ok(RawWindowHandle::Xlib(h)) => h.window as usize,
        #[cfg(target_os = "linux")]
        Ok(RawWindowHandle::Xcb(h)) => h.window.get() as usize,
        _ => 0,
    }
}

impl eframe::App for App {
    fn ui(&mut self, root: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = &root.ctx().clone();
        self.handle_keys(ctx);
        self.update_controller(ctx);
        self.poll_plugin_events();
        self.run_pending(ctx);

        self.top_bar(root);
        self.bottom_bar(root);
        self.side_panel(root);
        self.central(root);

        if !ctx.input(|i| i.pointer.any_down()) {
            self.piano.release(&mut self.keyboard.lock());
        }
        // Caps Lock is polled, and Linux plugin editors need their run loop pumped.
        let ms = if cfg!(target_os = "linux") && self.loaded.is_some() {
            16
        } else {
            40
        };
        ctx.request_repaint_after(std::time::Duration::from_millis(ms));

        // Plugin editors render with their own GL contexts on this thread; egui paints right
        // after this returns, so our context must be the current one again.
        if self.gl_guard.as_ref().is_some_and(|g| g.restore()) && !self.gl_restore_logged {
            self.gl_restore_logged = true;
            eprintln!("cherryjam: a plugin switched the OpenGL context; switching back each frame");
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.unload();
        self.config.save();
    }
}
