#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod audio;
mod config;
mod controller;
mod fx;
mod gl_guard;
mod keyboard;
mod piano;
mod preset;
mod scan;
mod theme;
mod vst3host;

fn main() -> eframe::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--list") {
        list_plugins();
        return Ok(());
    }
    if let Some(i) = args.iter().position(|a| a == "--probe") {
        probe(args.get(i + 1).map(String::as_str).unwrap_or(""));
        return Ok(());
    }

    // VST3 editors on Linux are X11 windows; they can only be embedded in an X11 (or XWayland)
    // window. Hiding the Wayland display makes winit pick X11.
    #[cfg(target_os = "linux")]
    unsafe {
        std::env::remove_var("WAYLAND_DISPLAY");
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("CherryJam")
            .with_app_id("cherryjam")
            .with_inner_size([1280.0, 860.0])
            .with_min_inner_size([720.0, 480.0])
            .with_icon(
                eframe::icon_data::from_png_bytes(include_bytes!("../icons/png/cherryjam-256.png"))
                    .unwrap_or_default(),
            ),
        ..Default::default()
    };
    let open = args
        .get(1)
        .filter(|a| !a.starts_with("--"))
        .map(std::path::PathBuf::from);
    eframe::run_native(
        "CherryJam",
        options,
        Box::new(|cc| Ok(Box::new(app::App::new(cc, open)))),
    )
}

fn list_plugins() {
    let cfg = config::Config::load();
    let entries = scan::scan(&cfg.extra_folders);
    for e in &entries {
        let kind = match e.is_instrument {
            Some(true) => "instrument",
            Some(false) => "effect",
            None => "unknown",
        };
        println!("{:<40} {:<11} {}", e.name, kind, e.bundle.display());
    }
    println!("{} plugins", entries.len());
}

/// Loads a plugin headlessly, plays a note and reports the output level (diagnostics).
fn probe(bundle: &str) {
    let Some(binary) = scan::binary_path(std::path::Path::new(bundle)) else {
        println!("not found: {bundle}");
        return;
    };
    let sender = audio::null_sender();
    let (plugin, mut proc) = match vst3host::plugin::Plugin::load(&binary, None, 48000.0, sender) {
        Ok(p) => p,
        Err(e) => {
            println!("load failed: {e}");
            return;
        }
    };
    println!(
        "loaded \"{}\", {} params",
        plugin.name,
        plugin.params().len()
    );
    let (comp, ctl) = plugin.get_state();
    println!(
        "state: component {} bytes, controller {} bytes",
        comp.len(),
        ctl.len()
    );
    plugin.set_state(&comp, &ctl);

    let mut l = vec![0.0f32; 512];
    let mut r = vec![0.0f32; 512];
    let mut peak = 0.0f32;
    let note_at: usize = std::env::var("PROBE_DELAY")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(10);
    for block in 0..note_at + 200 {
        match block {
            n if n == note_at => proc.note_on(60, 0.9),
            n if n == note_at + 90 => proc.note_off(60),
            _ => {}
        }
        pump_messages();
        proc.process(&mut l, &mut r);
        peak = l.iter().chain(r.iter()).fold(peak, |p, x| p.max(x.abs()));
    }
    println!("peak output after note: {peak:.4}");

    // Optional: check that a parameter change reaches the audio (e.g. PROBE_PARAM=Cutoff).
    if let Ok(name) = std::env::var("PROBE_PARAM") {
        let params = plugin.params();
        match params
            .iter()
            .find(|p| p.title.to_lowercase().contains(&name.to_lowercase()))
        {
            Some(p) => {
                let mut brightness = |value: f64| {
                    proc.set_param(p.id, value);
                    proc.note_on(60, 0.9);
                    let (mut diff, mut level) = (0.0f64, 0.0f64);
                    for block in 0..100 {
                        pump_messages();
                        proc.process(&mut l, &mut r);
                        if block >= 20 {
                            for w in l.windows(2) {
                                diff += (w[1] - w[0]).abs() as f64;
                                level += w[1].abs() as f64;
                            }
                        }
                    }
                    proc.note_off(60);
                    for _ in 0..100 {
                        proc.process(&mut l, &mut r);
                    }
                    if level > 0.0 { diff / level } else { 0.0 }
                };
                let lo = brightness(0.0);
                let hi = brightness(1.0);
                println!(
                    "param \"{}\": brightness at 0.0 = {lo:.4}, at 1.0 = {hi:.4}",
                    p.title
                );
            }
            None => println!("no parameter matching {name}"),
        }
    }

    plugin.shutdown(Some(proc));
    println!("shutdown ok");
}

/// Some plugins finish initializing via window messages; the probe has no event loop of its own.
fn pump_messages() {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            DispatchMessageW, MSG, PM_REMOVE, PeekMessageW, TranslateMessage,
        };
        let mut msg: MSG = std::mem::zeroed();
        while PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    std::thread::sleep(std::time::Duration::from_millis(2));
}
