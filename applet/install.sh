#!/usr/bin/env bash
# Install the Phosphor Scope Cinnamon applet for the current user.
#
# The applet is engine-free: it draws by spawning `phosphor feed`, so it
# requires phosphor >= 4.0 on PATH (install the .deb). Nothing is bundled but
# the five applet files below. Upgrades from a pre-2.0 install must CLEAN the
# old bundled Python engine, not accrete alongside it.
set -euo pipefail

uuid="phosphor-scope@phosphor"
here="$(cd "$(dirname "$0")" && pwd)"
src="$here/$uuid"
dest="$HOME/.local/share/cinnamon/applets/$uuid"

mkdir -p "$dest"
cp -f "$src/applet.js" \
      "$src/metadata.json" \
      "$src/settings-schema.json" \
      "$src/stylesheet.css" \
      "$src/icon.png" \
      "$dest/"

# Clean a pre-2.0 install: the applet used to bundle its own v3 Python engine.
# Upgrades must clean, not accrete — remove the retired files if present.
rm -f "$dest/phosphor_applet_feed.py" \
      "$dest/phosphor_audio.py" \
      "$dest/phosphor_signal.py" \
      "$dest/phosphor_core.py" \
      "$dest/libphosphor_core.so"
rm -rf "$dest/__pycache__"

echo "Installed Phosphor Scope to:"
echo "  $dest"
echo
echo "Requires phosphor >= 4.0 on PATH (install the .deb)."
echo
echo "Next: right-click your panel -> Applets (or Menu -> Applets), find"
echo "'Phosphor Scope', and click + to add it. If it was already running,"
echo "remove and re-add it (or reload Cinnamon with Ctrl+Alt+Esc) to pick up changes."
