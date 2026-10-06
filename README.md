<p align="center">
  <img src="icons/svg/cherryjam.svg" alt="CherryJam" width="140">
</p>

<h1 align="center">CherryJam</h1>

<p align="center"><b>Load a synth. Hit a key. Jam.</b><br>
<sub>No DAW. No routing. No setup. Just you, your VST instruments and your computer keyboard.</sub></p>

---

You've got a folder full of gorgeous VST instruments: vintage polysynths, monster modulars, sparkling e-pianos. Every time inspiration strikes, you have to boot a DAW, create a project, add an instrument track, arm it, wire up MIDI and hunt for the monitoring button… and by then the riff is gone.

**CherryJam skips all of that.**

1. **Fire it up.**
2. **Pick an instrument or a preset.**
3. **JAM!!!**

That's the whole signal chain. The synth's own panel opens big and beautiful in the window, a two-octave keybed waits underneath, and your QWERTY is now a playable controller. Noodle a bassline, stack some pads, sweep the filter like it's 1983. Your computer keyboard is the controller and CherryJam takes care of the rest.

When you've dialed in a sound you love, save it as a preset. Next time you open CherryJam it's right there, ready to go, exactly where you left it.

## ✨ Why CherryJam?

Plenty of apps can host a VST. Some cost a small fortune; the free ones usually make you play audio engineer first: audio routing, MIDI routing, device setup, the works. CherryJam is built for one thing only: **getting from idea to sound in seconds.**

* **Zero setup.** It finds your plugins in the usual folders automatically. You just need a VST3 instrument.
* **Plays from the keys you already have.** No MIDI controller needed.
* **Tiny and fast.** Written in Rust, about a 6 MB executable, low-latency audio.
* **Looks the part.** A clean, dark, cherry-red interface that gets out of the way of the instrument.

---

## 🎹 Features

### The instrument, front and center

The plugin's own interface fills the top of the window. Resizable editors stretch to fill the space, editors that support it are scaled to fit, and if a plugin's minimum size is bigger than the window, CherryJam grows the window to fit it. You tweak knobs directly on the instrument, just like on the real hardware.

Hit **F11** (or the ⛶ button in the top bar) for **fullscreen**: no taskbar, no distractions, just you and the synth. F11 again brings the window back. It works even after you've clicked into the plugin.

### Your keyboard is a keybed

Two full octaves (plus a few extra notes on top), laid out the way a piano player would expect:

| Octave | White keys | Black keys |
| ------ | ---------- | ---------- |
| lower  | bottom letter row (`Z X C V B N M , . /` on QWERTY) | home row (`S D` · `G H J` · `L ;`) |
| upper  | top letter row (`Q W E R T Y U I O P [ ]`) | number row (`2 3` · `5 6 7` · `9 0` · `=`) |

* **Works on any layout.** Keys are matched by their *position*, so QWERTY, QWERTZ and AZERTY players all get the same feel, and the on-screen piano labels show *your* keycaps.
* **Octave shift** with `←` / `→` (or `PgDn` / `PgUp`), or the arrows in the top bar.
* **Velocity** slider in the top bar.
* **Keeps playing while you tweak.** Click into the plugin to turn a knob and your keys still play notes.
* **Typing is safe.** While the cursor is in a text field (search, preset name), keys type instead of playing.

### Clickable on-screen piano

The keybed under the instrument lights up with every note you play and shows which computer key triggers which note. Click it to play, or drag across it for glissandos.

### XY controller: your mouse as a performance pad

Map any two plugin parameters to the mouse's X and Y axes, say filter cutoff on X and resonance on Y. Then **flip on Caps Lock**: the cursor vanishes and every mouse or touchpad move sweeps those parameters live, while you keep playing with the other hand. Flip Caps Lock off and the cursor is back.

* **Learn:** click *Learn*, wiggle a knob in the plugin, done.
* **Choose:** pick a parameter from a searchable list of everything the plugin exposes.
* **Range and invert** per axis, so you can keep the sweep in the sweet spot.
* **Sensitivity** control for big, dramatic sweeps or fine, subtle moves.
* A live **XY pad** shows where you are.

### Arpeggiator: hold a chord, get a groove

Hit **ARP** in the top bar, hold down a chord, and CherryJam turns it into a rhythmic pattern, locked to the tempo. That's it: instant 80s sequencer lines, trance gates and rolling synth-pop basslines without playing a single fast note.

When you want more, the *Arp* tab has the controls:

* **Rate:** 1/4, 1/8, 1/8T, 1/16, 1/16T, 1/32.
* **Mode:** Up, Down, Up ↕ Down, Down ↕ Up, As played, Random, or **Chord** (the whole chord re-struck on every step).
* **Octaves:** spread the pattern over 1 to 4 octaves.
* **Gate:** short staccato blips up to fully legato (100 %).
* **Swing:** from straight (50 %) to a deep shuffle (75 %).
* **Latch:** let go of the keys and the arp keeps running; play a new chord to swap it.
* **Step pattern:** up to 16 steps; click any step to turn it into a rest and get syncopated, stuttering rhythms. Quick buttons for *all steps*, *every other step* and a 🎲 random pattern.
* **Restart on new chord:** the pattern starts from step 1 with every new chord, or keeps flowing if you turn it off.

On the on-screen piano, the keys you hold glow dark red and the note the arp is playing right now lights up bright, so you can watch the pattern run.

### One tempo for everything

The **BPM** field in the top bar (drag it, type a value, or hit **Tap** in time a few times) sets the tempo for the arpeggiator, the synced effects *and* the instrument itself. Plugins with tempo-synced LFOs, arps or delays lock to CherryJam's tempo too.

### Built-in effects: delay & reverb

Two simple, musical effects sit after the instrument:

* **Delay:** time (10 ms – 2 s), feedback, tone (a low-pass in the feedback loop for warm, darkening repeats), mix and **ping-pong** for stereo bounce. Turn on **Sync** and pick a note value instead (1/4, dotted 1/8, 1/8 triplet …), and the echoes land on the beat at any tempo.
* **Reverb:** size, damping, width, pre-delay and mix, from a tight room to a huge wash. The pre-delay can **sync** to the tempo too (1/64 to 1/16), for a reverb that breathes with the groove.
* **Master** volume, followed by a soft limiter so things never clip harshly.

### Presets: one click back to your sound

A preset captures the **whole setup**: which instrument, its complete sound (the plugin's full internal state, not just the patch name), your XY mapping, your effect settings, the arpeggiator and the tempo. Load it and everything snaps back exactly as it was. CherryJam reopens your last preset automatically on startup.

Presets are plain JSON files in `%APPDATA%\cherryjam\presets` (Windows) or `~/.config/cherryjam/presets` (Linux), so they're easy to back up or share.

### Finds your plugins

CherryJam scans the standard VST3 locations:

* **Windows:** `%CommonProgramFiles%\VST3` and `%LOCALAPPDATA%\Programs\Common\VST3`
* **Linux:** `~/.vst3`, `/usr/lib/vst3`, `/usr/local/lib/vst3`

Add more folders under *Settings*. Plugins known to be effects (from their module info, or once you've loaded one) are hidden from the instrument list. Tick *Also list effects / unknown plugins* in *Settings* to see everything.

---

## 🚀 Getting started

### Download

Grab the latest build from the [Releases](../../releases) page:

* **Windows installer:** `cherryjam-…-windows-x86_64-setup.exe` installs CherryJam with a Start menu entry (and an optional desktop shortcut). Uninstall from *Apps & features*; your presets and settings are kept.
* **Windows portable:** `cherryjam-…-windows-x86_64-portable.zip`. Unzip anywhere and run `cherryjam.exe`. Nothing to install.
* **Linux:** `cherryjam-…-linux-x86_64.tar.gz`. Unpack and run `./cherryjam`, or run `./install.sh` to add it to your app menu (installs for your user only, no root needed).

### Build

```sh
cargo build --release        # → target/release/cherryjam(.exe), ~6 MB
```

On Linux you'll need the ALSA development headers (`libasound2-dev` / `alsa-lib-devel`) and an X server or XWayland.

### Run

```sh
cherryjam                                # opens your last preset
cherryjam "path/to/Synth.vst3"           # opens an instrument directly
cherryjam path/to/preset.json            # opens a preset file
cherryjam --list                         # lists the plugins CherryJam finds
cherryjam --probe "path/to/Synth.vst3"   # loads a plugin without the UI, plays a note, reports the output level
```

---

## 📝 Notes & limitations

* VST3 instruments only.
* Plugins run inside CherryJam's process, so a plugin that crashes takes CherryJam down with it.
* Some plugins don't redraw their knobs when the XY controller moves a parameter. The sound still changes.
* On Linux, CherryJam runs through X11 or XWayland because VST3 editors on Linux are X11 windows. Linux support hasn't been tested yet.

<p align="center"><sub>Made for people who'd rather play than patch cables. 🍒</sub></p>
