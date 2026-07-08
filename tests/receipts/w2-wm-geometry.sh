#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-or-later
# W2 receipts — window geometry against a REAL reparenting WM.
# BUGLOG #9/#10/#11 territory: mode-switch convergence, the settle
# machinery, persistence truth. Xvfb receipts without a WM prove
# nothing about frame insets or focus dances, so this rig boots its
# own nested Muffin (Ben's actual WM) at 2560x1440 and drives the
# user's real gestures: drags, double-clicks, M inside the settle
# window, quits mid-state, and a SIGSTOP-pulsed (hitching) WM.
#
#   tests/receipts/w2-wm-geometry.sh [path-to-phosphor-binary]
#
# All position comparisons are CLIENT root coordinates (the pixels
# the user sees) — outer/frame math mixes extent theories (xprop's
# visible extents vs winit's invisible-border accounting) and lies.
# settings.json is read only after a quit has flushed it.
# PHOSPHOR_GEOM_LOG=1 streams every geometry decision to the logs.

set -u
BIN="${1:-$(dirname "$0")/../../target/release/phosphor}"
BIN="$(readlink -f "$BIN")"
WORK="$(mktemp -d /tmp/phos-w2.XXXXXX)"
RT="$WORK/rt"; SCRATCH_HOME="$WORK/home"
mkdir -p -m 700 "$RT" "$SCRATCH_HOME"
REAL_RT="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"
DPY=":97"

PASS=0; FAIL=0
ok()  { PASS=$((PASS+1)); echo "  ok   $1"; }
bad() { FAIL=$((FAIL+1)); echo "  FAIL $1"; }

cleanup() {
    [ -n "${APP_PID:-}" ] && kill "$APP_PID" 2>/dev/null
    [ -n "${PULSER:-}" ] && kill "$PULSER" 2>/dev/null
    [ -n "${MUF_PID:-}" ] && kill -CONT "$MUF_PID" 2>/dev/null
    [ -n "${MUF_PID:-}" ] && kill "$MUF_PID" 2>/dev/null
    [ -n "${XVFB_PID:-}" ] && kill "$XVFB_PID" 2>/dev/null
    rm -rf "$WORK"
}
trap cleanup EXIT

# ---- the rig: Xvfb at Ben's resolution + a real Muffin ----
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

launch() {
    env DISPLAY="$DPY" XDG_RUNTIME_DIR="$RT" HOME="$SCRATCH_HOME" \
        PIPEWIRE_RUNTIME_DIR="$REAL_RT" \
        PULSE_SERVER="unix:$REAL_RT/pulse/native" \
        PHOSPHOR_NO_SINGLE_INSTANCE=1 PHOSPHOR_GEOM_LOG=1 \
        "$BIN" > "$WORK/$1.log" 2>&1 &
    APP_PID=$!
    sleep 4
}
ctl() { env XDG_RUNTIME_DIR="$RT" "$BIN" ctl "$@"; }
setting() { python3 -c "import json;d=json.load(open('$SCRATCH_HOME/.config/phosphor/settings.json'));print(d.get('$1'))"; }
wid() { xdotool search --class phosphor | head -1; }

echo "== A. relaunch restores position AND size (client-true) =="
launch a1
W=$(wid)
wmctrl -i -r "$W" -e 0,500,300,-1,-1; sleep 0.8
wmctrl -i -r "$W" -e 0,-1,-1,1400,900; sleep 1.2
eval "$(xdotool getwindowgeometry --shell "$W")"
ACX=$X; ACY=$Y; ACW=$WIDTH; ACH=$HEIGHT
ctl quit >/dev/null; sleep 1.5
launch a2
W=$(wid)
eval "$(xdotool getwindowgeometry --shell "$W")"
[ "$X,$Y" = "$ACX,$ACY" ] && ok "relaunch client at $X,$Y == pre-quit" \
    || bad "relaunch client $X,$Y != $ACX,$ACY"
[ "$WIDTH,$HEIGHT" = "$ACW,$ACH" ] && ok "relaunch size ${WIDTH}x${HEIGHT} == pre-quit" \
    || bad "relaunch size ${WIDTH}x${HEIGHT} != ${ACW}x${ACH}"

echo "== B. 3 mini round-trips under a hitching WM =="
( for i in $(seq 1 60); do kill -STOP "$MUF_PID"; sleep 0.25
                           kill -CONT "$MUF_PID"; sleep 0.15; done ) &
PULSER=$!
xdotool windowactivate "$W"; sleep 0.4
eval "$(xdotool getwindowgeometry --shell "$W")"
BX=$X; BY=$Y; BW=$WIDTH; BH=$HEIGHT
STABLE=1
for round in 1 2 3; do
    xdotool key m; sleep 2.4
    eval "$(xdotool getwindowgeometry --shell "$W")"
    [ "$WIDTH" = "$HEIGHT" ] || { STABLE=0; echo "  (mini $round skewed: ${WIDTH}x${HEIGHT})"; }
    xdotool key m; sleep 2.8
    eval "$(xdotool getwindowgeometry --shell "$W")"
    [ "$X,$Y,$WIDTH,$HEIGHT" = "$BX,$BY,$BW,$BH" ] \
        || { STABLE=0; echo "  (back $round: $X,$Y ${WIDTH}x${HEIGHT} want $BX,$BY ${BW}x${BH})"; }
done
kill "$PULSER" 2>/dev/null; PULSER=""
kill -CONT "$MUF_PID" 2>/dev/null
[ "$STABLE" = "1" ] && ok "3 pulsed round-trips client-stable to the pixel" \
    || bad "pulsed round-trips drifted"

echo "== C. double-click restore = exactly ONE toggle (#11 owner law) =="
xdotool key m; sleep 2.2
eval "$(xdotool getwindowgeometry --shell "$W")"
xdotool mousemove $((X+120)) $((Y+120))
xdotool click 1; sleep 0.15; xdotool click 1; sleep 2.8
eval "$(xdotool getwindowgeometry --shell "$W")"
[ "$WIDTH,$HEIGHT" = "$BW,$BH" ] && ok "double-click restored ${WIDTH}x${HEIGHT}" \
    || bad "double-click ended ${WIDTH}x${HEIGHT} (state flap)"

echo "== D. drag mini, M inside the settle window (#11 settle law) =="
xdotool key m; sleep 2.2
eval "$(xdotool getwindowgeometry --shell "$W")"
xdotool mousemove $((X+120)) $((Y+120)) mousedown 1
for i in 1 2 3 4 5 6 7 8; do xdotool mousemove_relative 40 30; sleep 0.05; done
xdotool mouseup 1; sleep 0.05
eval "$(xdotool getwindowgeometry --shell "$W")"
DGX=$X; DGY=$Y
xdotool key m; sleep 2.8
eval "$(xdotool getwindowgeometry --shell "$W")"
[ "$X,$Y,$WIDTH,$HEIGHT" = "$BX,$BY,$BW,$BH" ] \
    && ok "restored window untouched by the settle" \
    || bad "restored $X,$Y ${WIDTH}x${HEIGHT} != $BX,$BY ${BW}x${BH}"

echo "== E. next M returns the mini to its dragged spot =="
xdotool key m; sleep 2.4
eval "$(xdotool getwindowgeometry --shell "$W")"
python3 -c "import sys; sys.exit(0 if abs($X-$DGX)<=8 and abs($Y-$DGY)<=8 else 1)" \
    && ok "mini re-entered at $X,$Y (dragged spot $DGX,$DGY)" \
    || bad "mini re-entered at $X,$Y != dragged $DGX,$DGY"
xdotool key m; sleep 2.6

echo "== F. F11 -> ONE M -> square mini -> M -> banked normal =="
xdotool key F11; sleep 1.8
xdotool key m; sleep 2.6
eval "$(xdotool getwindowgeometry --shell "$W")"
[ "$WIDTH" = "$HEIGHT" ] && ok "fullscreen -> single M gave a ${WIDTH}px square" \
    || bad "fullscreen->M gave ${WIDTH}x${HEIGHT}"
xdotool key m; sleep 2.8
eval "$(xdotool getwindowgeometry --shell "$W")"
[ "$X,$Y,$WIDTH,$HEIGHT" = "$BX,$BY,$BW,$BH" ] \
    && ok "restored banked normal $X,$Y ${WIDTH}x${HEIGHT}" \
    || bad "got $X,$Y ${WIDTH}x${HEIGHT} want $BX,$BY ${BW}x${BH}"

echo "== G. quit from mini; relaunch normal; mini spot survives =="
xdotool key m; sleep 2.2
eval "$(xdotool getwindowgeometry --shell "$W")"
GMX=$X; GMY=$Y
ctl quit >/dev/null; sleep 1.5
FMX=$(setting mini_x); FMY=$(setting mini_y)
launch g
W=$(wid)
eval "$(xdotool getwindowgeometry --shell "$W")"
[ "$X,$Y,$WIDTH,$HEIGHT" = "$BX,$BY,$BW,$BH" ] \
    && ok "relaunch after mini-quit: normal $X,$Y ${WIDTH}x${HEIGHT}" \
    || bad "relaunch: $X,$Y ${WIDTH}x${HEIGHT} want $BX,$BY ${BW}x${BH}"
xdotool windowactivate "$W"; sleep 0.4
xdotool key m; sleep 2.4
eval "$(xdotool getwindowgeometry --shell "$W")"
python3 -c "import sys; sys.exit(0 if abs($X-$GMX)<=8 and abs($Y-$GMY)<=8 else 1)" \
    && ok "mini spot survived the relaunch ($X,$Y ~ $GMX,$GMY; file $FMX,$FMY)" \
    || bad "mini spot lost: $X,$Y vs $GMX,$GMY (file $FMX,$FMY)"
xdotool key m; sleep 2.4
ctl quit >/dev/null; sleep 1
APP_PID=""

echo
echo "PASS=$PASS FAIL=$FAIL"
[ "$FAIL" = "0" ]
