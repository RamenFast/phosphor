#!/usr/bin/env bash
# Install the Phosphor Scope Cinnamon applet for the current user.
#
# It copies the applet into ~/.local/share/cinnamon/applets/ and bundles the
# feed helper plus the two Phosphor modules it imports, so the applet is
# self-contained and works whether or not the .deb is installed.
set -euo pipefail

uuid="phosphor-scope@phosphor"
here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/.." && pwd)"
dest="$HOME/.local/share/cinnamon/applets/$uuid"

mkdir -p "$dest"
cp -f "$here/$uuid"/* "$dest/"
# Bundle the headless feed + the modules it imports (kept in sync at install time).
cp -f "$repo/phosphor_applet_feed.py" \
      "$repo/phosphor_audio.py" \
      "$repo/phosphor_signal.py" \
      "$dest/"

echo "Installed Phosphor Scope to:"
echo "  $dest"
echo
echo "Next: right-click your panel -> Applets (or Menu -> Applets), find"
echo "'Phosphor Scope', and click + to add it. If it was already running,"
echo "remove and re-add it (or reload Cinnamon with Ctrl+Alt+Esc) to pick up changes."
