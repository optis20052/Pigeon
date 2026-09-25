#!/usr/bin/env bash
# Builds Pigeon in release mode and installs it for the current user.
set -euo pipefail
cd "$(dirname "$0")"

cargo build --release

install -Dm755 target/release/pigeon "$HOME/.local/bin/pigeon"
install -Dm644 data/dev.pigeon.Pigeon.desktop "$HOME/.local/share/applications/dev.pigeon.Pigeon.desktop"
# the icon is generated from code (src/ui/app_icon.rs); this writes the default variant
install -d "$HOME/.local/share/icons/hicolor/scalable/apps"
target/release/pigeon --export-icon "$HOME/.local/share/icons/hicolor/scalable/apps/dev.pigeon.Pigeon.svg"

# remove the install from when the app was called Courier
rm -f "$HOME/.local/bin/courier" \
      "$HOME/.local/share/applications/dev.courier.Courier.desktop" \
      "$HOME/.local/share/icons/hicolor/scalable/apps/dev.courier.Courier.svg"

gtk-update-icon-cache -q -t "$HOME/.local/share/icons/hicolor" 2>/dev/null || true
update-desktop-database -q "$HOME/.local/share/applications" 2>/dev/null || true

echo "Installed. Launch “Pigeon” from the app grid, or run: pigeon"
