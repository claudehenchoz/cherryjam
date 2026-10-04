# CherryJam icons

## Which artwork is used at which size
- 16 px: pixel-snapped variant (`svg/cherryjam-16.svg`)
- 20–48 px: simplified variant (`svg/cherryjam-simplified.svg`)
- 64 px and up: full master (`svg/cherryjam.svg`)
- `svg/cherryjam-tilefree.svg`: no background tile, for themes that draw their own shape

## Windows
`windows/cherryjam.ico` holds 16, 20, 24, 32, 40, 48, 64, 128 and 256 px.

To embed it in the Rust executable, add `winres` (or `embed-resource`) as a build dependency and in `build.rs`:

```rust
fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").unwrap() == "windows" {
        let mut res = winres::WindowsResource::new();
        res.set_icon("assets/icons/windows/cherryjam.ico");
        res.compile().unwrap();
    }
}
```

## Linux
Copy `linux/hicolor/` into `/usr/share/icons/` (system) or `~/.local/share/icons/` (user), then run:

```sh
gtk-update-icon-cache -f -t ~/.local/share/icons/hicolor
```

In the `.desktop` file, refer to the icon by name only: `Icon=cherryjam`.

On Wayland the compositor matches the window to its `.desktop` file through the app ID, so set the window's app ID to the desktop file's name (e.g. `cherryjam` for `cherryjam.desktop`), or the icon won't show in the taskbar.

## Other
`png/` has loose PNGs from 16 to 1024 px for web pages, README badges, store listings and so on.
