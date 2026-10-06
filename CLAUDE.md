# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

CherryJam is a minimal VST3 instrument host in Rust (egui/eframe with the glow renderer, cpal audio). You load an instrument and play it from the computer keyboard. It has an on-screen piano, an XY mouse controller (engaged by Caps Lock), built-in delay/reverb, and JSON presets. It targets Windows and Linux (X11/XWayland). Simplicity of use is the core product goal (see README).

## Commands

```sh
cargo build                       # debug build
cargo build --release             # release (LTO, stripped, panic=abort) → ~6 MB
cargo test                        # unit tests (keymap, presets, fx, scan, controller)
cargo test keyboard::tests::note_refcount   # single test
cargo clippy                      # keep this warning-free on both platforms
cargo fmt

cargo run -- --list                          # list discovered plugins
cargo run -- --probe "C:/Program Files/Common Files/VST3/Mini V4.vst3"
#   headless: loads plugin, plays a note, prints peak level, round-trips state, shuts down.
#   PROBE_PARAM="Filter Cutoff" also checks that a param change reaches the audio.
#   PROBE_DELAY=<blocks> delays the note for plugins that initialize slowly.
cargo run -- "path/to/Synth.vst3"            # open an instrument (or a preset .json) directly
```

Windows-only code (`#[cfg(windows)]`) and Linux-only code (`#[cfg(target_os = "linux")]`) are both substantial, and compiling on one OS does not check the other. To verify the Linux build from Windows, use a container. CI uses the same apt packages:

```sh
podman run --rm -v "C:\CHDEV\cherryjam:/src:ro" -e CARGO_TARGET_DIR=/tmp/target -w /src docker.io/library/rust:latest bash -c \
  "apt-get update -qq && apt-get install -y -qq pkg-config libasound2-dev libxkbcommon-dev libgl1-mesa-dev libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev && rustup component add clippy && cargo clippy --locked --all-targets && cargo test --locked"
```

VST3 enum constants are `i32` on Windows but `u32` on Linux, so casts like `kRealtime as i32` are needed even though clippy flags them on Windows. Those files carry `#![allow(clippy::unnecessary_cast)]`.

## Releases

Pushing a `v*` tag runs `.github/workflows/release.yml`. It builds on windows-latest and ubuntu-22.04, runs tests, and publishes a GitHub release with:
- `…-windows-x86_64-portable.zip`
- `…-windows-x86_64-setup.exe`, built by NSIS from `packaging/windows/cherryjam.nsi`; the version comes from the tag
- `…-linux-x86_64.tar.gz`, which includes `packaging/cherryjam.desktop`, `packaging/install.sh` and the hicolor icons

`--locked` is used, so `Cargo.lock` must be committed. Bump `version` in `Cargo.toml` before tagging. `build.rs` embeds `icons/windows/cherryjam.ico` into the Windows exe via `winresource`.

## Architecture

**Threads and data flow.** Three contexts matter:
- **UI thread (eframe):** owns the `Plugin` (component + edit controller), the editor, and all VST3 calls except `process`. VST3 requires setup on the main thread, so plugins are loaded here. They load after a deferred `Pending` delay of a couple of frames, so the "Loading…" screen gets painted first.
- **Audio thread (cpal callback, `audio.rs`):** owns the `Processor`. That is the processing half of the plugin, with preallocated event and parameter lists. The `Processor` lives in a `ProcessorSlot = Arc<Mutex<Option<Processor>>>`. The audio thread `try_lock`s it (outputting silence if busy), and the UI only locks it to swap plugins. Everything else goes UI → audio as `Msg` values over an `rtrb` ring (`MsgSender`), covering notes, parameter changes and `FxSettings`. The chain is plugin → delay → reverb → master gain → `soft_limit`, processed in `MAX_BLOCK` (512) chunks.
- **Plugin → host callbacks:** `ComponentHandler::performEdit` forwards plugin GUI edits into the same `MsgSender`. It also records `last_touched`, which drives "Learn" in the XY mapping.

**VST3 hosting (`src/vst3host/`)** uses the raw `vst3` crate (COM bindings via `ComPtr`/`ComWrapper`):
- `module.rs` loads libraries and never unloads them; they are cached for the life of the process.
- `host_iface.rs` implements the host-side COM objects. These are `HostApp` (which creates `IMessage`/`IAttributeList` for component↔controller messaging), `MemStream` (for state), `PlugFrame` (`resizeView`, plus `IRunLoop` on Linux, pumped every UI frame), and the audio-thread `EventList`/`ParamChanges`. The audio-thread objects use `AudioCell` (an `UnsafeCell`) because only the audio thread touches them.
- `plugin.rs` wires component and controller together, activates only event bus 0 and audio output bus 0, and handles state get/set.
- `editor.rs` embeds `IPlugView` in a native child window: an HWND child on Windows, an x11rb window on Linux. The editor is a native window drawn *on top of* the egui GL surface, which has consequences:
  - It is hidden while egui popups or the loading screen are shown.
  - It is clipped to its area so it never covers the piano or side panel.
  - Oversized editors make the app grow its window once (`Editor::overflow`).
  - Scaling uses `checkSizeConstraint` + `onSize` for resizable editors, or `IPlugViewContentScaleSupport`.
  - On Windows the parent gets `WS_CLIPCHILDREN`. Use the glow renderer, not wgpu, because DXGI flip-model swapchains overdraw child windows.
  - Plugin editors often render with their own OpenGL context on our UI thread and leave it current. eframe doesn't notice, and the whole egui UI goes black. `gl_guard.rs` captures eframe's context (WGL; GLX or EGL on Linux) in `App::new` and restores it at the end of every `App::ui`, right before eframe paints.

**Keyboard input (`keyboard.rs`)** is matched by *physical* key position (`KEYMAP`: offsets from `base_note`). The piano labels show the user's actual layout (from the Win32 keyboard layout, or the X11 keysyms on Linux). `KeyboardState` refcounts notes so the mouse and keys don't cut each other off. Key events reach it on three paths, depending on platform and focus:
- **Windows, egui focused:** egui `Event::Key` in `App::handle_keys`.
- **Windows, plugin editor focused:** a thread-local `WH_KEYBOARD` hook (`keyboard::hook`). It intercepts note, octave and F11 keys, because the child window would otherwise eat them.
- **Linux:** `keyboard::x11`, a background thread that polls `QueryKeymap` every 2 ms while any window inside the main window has X input focus. When the poller is `ACTIVE`, egui key events are ignored for notes. Plugins (e.g. JUCE ones) can steal X focus at any time.

Typing in egui text fields must not play notes. Mechanisms:
- `egui_wants_keyboard_input()` stops the egui path.
- `x11::TEXT_INPUT` pauses the Linux poller.
- Any click on egui calls `editor::focus_parent()` to take focus back from the plugin child.

F11 fullscreen requests from the hook or poller go through `keyboard::FULLSCREEN_REQUESTED`.

**XY controller (`controller.rs` + `capture` module in `app.rs`).** Caps Lock state is polled from the OS each frame. On Windows, mouse deltas come from hiding the cursor and re-centering it on an anchor point every frame. This works no matter which window has focus, whereas eframe only forwards raw motion while its window is focused. On Linux it uses a cursor grab plus egui `MouseMoved`. While not engaged, XY positions follow the plugin's current parameter values, so engaging never jumps.

**Persistence.** `config.rs` (`config.json`) and `preset.rs` (`presets/*.json`) live in `dirs::config_dir()/cherryjam`. A preset stores the bundle path, class id (hex TUID), base64 component and controller state, the `Mapping` and the `FxSettings`. All serde structs use `#[serde(default)]` so older files keep loading. Plugin discovery (`scan.rs`) only walks directories and reads `moduleinfo.json`, without loading code. Plugins found to be effects when loaded are remembered in `config.known_effects`.

## Testing notes

There is no automated GUI test. GUI behaviour was verified on Windows by launching the app with a plugin argument, sending input via `keybd_event`/`mouse_event` from PowerShell, and screenshotting the window. Linux GUI behaviour can only be tested by the user. Running the app writes to `%APPDATA%\cherryjam`; clean that up after manual tests.
