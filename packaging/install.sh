#!/bin/sh
# Installs CherryJam for the current user (binary, desktop entry and icons).
set -e
here="$(cd "$(dirname "$0")" && pwd)"
bin="${HOME}/.local/bin"
apps="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
icons="${XDG_DATA_HOME:-$HOME/.local/share}/icons"

mkdir -p "$bin" "$apps" "$icons"
install -m 755 "$here/cherryjam" "$bin/cherryjam"
install -m 644 "$here/cherryjam.desktop" "$apps/cherryjam.desktop"
cp -r "$here/icons/hicolor" "$icons/"
command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -f -t "$icons/hicolor" || true
command -v update-desktop-database >/dev/null && update-desktop-database "$apps" || true
echo "CherryJam installed to $bin/cherryjam"
