#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Mini-furniture receipts — BUGLOG #19 (Ben's UX round, 2026-07-10):
# a press over egui furniture in the mini (playlist slide-over,
# postcard dialog, kit cards) must reach EGUI, never start the WM
# move-grab that made "the window move around with my mouse". The
# bare-scope drag-to-move law must survive the same gate. All
# furniture rides ONE predicate (occlusion-aware scope_hovered), so
# the pane receipts cover the dialog path.
#
#   tests/receipts/mini-furniture.sh [path-to-phosphor-binary]

set -u
BIN="${1:-$(dirname "$0")/../../target/release/phosphor}"
BIN="$(readlink -f "$BIN")"
WORK="$(mktemp -d /tmp/phos-minifurn.XXXXXX)"
RT="$WORK/rt"; SCRATCH_HOME="$WORK/home"
mkdir -p -m 700 "$RT" "$SCRATCH_HOME"
REAL_RT="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"
DPY=":95"

PASS=0; FAIL=0
ok()  { PASS=$((PASS+1)); echo "  ok   $1"; }
bad() { FAIL=$((FAIL+1)); echo "  FAIL $1"; }

cleanup() {
    [ -n "${APP_PID:-}" ] && kill "$APP_PID" 2>/dev/null
    [ -n "${MUF_PID:-}" ] && kill "$MUF_PID" 2>/dev/null
    [ -n "${XVFB_PID:-}" ] && kill "$XVFB_PID" 2>/dev/null
    rm -rf "$WORK"
}
trap cleanup EXIT

Xvfb "$DPY" -screen 0 2560x1440x24 > "$WORK/xvfb.log" 2>&1 &
XVFB_PID=$!
sleep 1.5
DISPLAY="$DPY" dbus-run-session -- muffin --x11 --sm-disable \
    > "$WORK/muffin.log" 2>&1 &
MUF_PID=$!
sleep 3
if ! DISPLAY="$DPY" wmctrl -m 2>/dev/null | grep -q Muffin; then
    echo "SKIP: nested Muffin did not come up (see $WORK/muffin.log)"
    exit 0
fi
export DISPLAY="$DPY"

# two sibling tones → a 2-row playlist (folder-sibling law)
python3 - "$WORK" <<'EOF'
import math, struct, sys, wave
for name, hz in (("a-tone", 440), ("b-tone", 660)):
    with wave.open(f"{sys.argv[1]}/{name}.wav", "wb") as w:
        w.setnchannels(2); w.setsampwidth(2); w.setframerate(48000)
        frames = 48000 * 300
        data = bytearray()
        for i in range(frames):
            s = int(math.sin(i / 48000 * hz * math.tau) * 0.2 * 32767)
            data += struct.pack("<hh", s, s)
        w.writeframes(bytes(data))
EOF

# a roomy mini so the pane and its rows have space
mkdir -p "$SCRATCH_HOME/.config/phosphor"
cat > "$SCRATCH_HOME/.config/phosphor/settings.json" <<'EOF'
{ "mini_size": 640 }
EOF

env DISPLAY="$DPY" XDG_RUNTIME_DIR="$RT" HOME="$SCRATCH_HOME" \
    PIPEWIRE_RUNTIME_DIR="$REAL_RT" \
    PULSE_SERVER="unix:$REAL_RT/pulse/native" \
    PHOSPHOR_NO_SINGLE_INSTANCE=1 PHOSPHOR_GEOM_LOG=1 \
    "$BIN" > "$WORK/app.log" 2>&1 &
APP_PID=$!
sleep 4
ctl() { env XDG_RUNTIME_DIR="$RT" "$BIN" ctl "$@"; }
probe() { env XDG_RUNTIME_DIR="$RT" "$BIN" probe --json; }
ctl volume 0 > /dev/null
ctl open "$WORK/a-tone.wav" > /dev/null
sleep 2

W=$(xdotool search --class phosphor | head -1)
xdotool windowactivate "$W"; sleep 0.5

echo "== A. enter mini, open the playlist slide-over =="
xdotool key m; sleep 2.5
eval "$(xdotool getwindowgeometry --shell "$W")"
[ "$WIDTH" = "$HEIGHT" ] && ok "mini is square (${WIDTH}px)" \
    || bad "mini skewed ${WIDTH}x${HEIGHT}"
xdotool key l; sleep 1.5
import -window root -crop 700x700+$((X))+$((Y)) "$WORK/pane.png" 2>/dev/null

grabs() { grep -ac "mini press: double" "$WORK/app.log"; }

echo "== B. drag over the pane: no WM grab, window still (BUGLOG #19) =="
eval "$(xdotool getwindowgeometry --shell "$W")"
B0="$X,$Y"; G0=$(grabs)
xdotool mousemove $((X + 100)) $((Y + 300)) mousedown 1
xdotool mousemove $((X + 220)) $((Y + 420)); sleep 0.6
xdotool mouseup 1; sleep 1.2
eval "$(xdotool getwindowgeometry --shell "$W")"
[ "$X,$Y" = "$B0" ] && ok "window stationary through a pane drag" \
    || bad "pane drag MOVED the window: $B0 -> $X,$Y"
[ "$(grabs)" = "$G0" ] && ok "no WM grab fired over the pane (geom log)" \
    || bad "the pane press still reached the WM-grab path"

echo "== C. a pane row click reaches egui (track switches) =="
BEFORE=$(probe | grep -o '"detail":"[^"]*"' | head -1)
# rows sit near the top of the pane (banked pane.png: a-tone ~y64,
# b-tone ~y97); walk the second row's candidates
for ROW_Y in 97 100 92; do
    xdotool mousemove $((X + 60)) $((Y + ROW_Y)) click 1; sleep 1.0
    AFTER=$(probe | grep -o '"detail":"[^"]*"' | head -1)
    [ "$AFTER" != "$BEFORE" ] && break
done
if [ "$AFTER" != "$BEFORE" ]; then
    ok "row click switched the track: $BEFORE -> $AFTER"
else
    bad "no pane row click landed (see $WORK/pane.png)"
fi
xdotool key l; sleep 1.0   # close the pane

echo "== D. bare-scope press still takes the WM-grab path (law intact) =="
# assert on the geom log, not final position — the magnetism settle
# may legitimately snap the mini back toward an edge after the drag
eval "$(xdotool getwindowgeometry --shell "$W")"
G0=$(grabs)
xdotool mousemove $((X + WIDTH - 150)) $((Y + HEIGHT / 2)) mousedown 1
xdotool mousemove $((X + WIDTH - 150 + 200)) $((Y + HEIGHT / 2 + 120))
sleep 0.8
xdotool mouseup 1; sleep 2.5
[ "$(grabs)" -gt "$G0" ] && ok "bare-scope press reached the WM-grab path" \
    || bad "bare-scope press no longer starts the mini drag"

echo "== E. leave mini: no stale resize cursor ride-along (soft leg) =="
# park the pointer on a corner (resize-hint zone), then M out — the
# cursor must be reset by set_mini_mode. XFixes cursor introspection
# isn't available headlessly; this leg receipts the gesture runs
# clean and banks a screenshot; Ben's desktop is the felt acceptance.
xdotool mousemove $((X + 4)) $((Y + 4)); sleep 0.5
xdotool key m; sleep 2.5
eval "$(xdotool getwindowgeometry --shell "$W")"
[ "$WIDTH" -gt 700 ] && ok "left mini clean (${WIDTH}x${HEIGHT})" \
    || bad "mini leave failed (${WIDTH}x${HEIGHT})"

ctl quit > /dev/null 2>&1
echo
echo "PASS=$PASS FAIL=$FAIL  (logs: $WORK — kept on failure)"
if [ "$FAIL" = "0" ]; then rm -rf "$WORK"; trap - EXIT
    kill "$MUF_PID" "$XVFB_PID" 2>/dev/null; exit 0
else trap - EXIT; kill "$APP_PID" "$MUF_PID" "$XVFB_PID" 2>/dev/null
    exit 1
fi
