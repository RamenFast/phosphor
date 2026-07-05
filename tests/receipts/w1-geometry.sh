#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-or-later
# W1 receipts — geometry truth through real source paths + the CLI
# front door + single instance. Run on a live desktop session; drives
# a PRIVATE phosphor instance (isolated control-socket dir, scratch
# HOME) — a concurrently running daily-drive instance is untouched.
#
#   tests/receipts/w1-geometry.sh [path-to-phosphor-binary]
#
# The squish law, numerically: a circle (L=sin,R=cos) must trace
# bbox aspect 1.00±0.03 in xy AND xy45 through the PLAYER and the
# CAPTURE path; L-only must be a horizontal line in xy; mono must be
# the M diagonal (a line, not a blob) in xy45. Sources: tap bbox.

set -u
BIN="${1:-$(dirname "$0")/../../target/release/phosphor}"
WORK="$(mktemp -d /tmp/phos-w1.XXXXXX)"
RT="$WORK/rt"; SCRATCH_HOME="$WORK/home"
mkdir -p -m 700 "$RT" "$SCRATCH_HOME"
REAL_RT="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"

# Isolation: private control-socket dir; audio pinned to the real
# daemons (PipeWire honors PIPEWIRE_RUNTIME_DIR; pulse tools get
# PULSE_SERVER) so the beam still hears.
PENV=(env "XDG_RUNTIME_DIR=$RT"
          "HOME=$SCRATCH_HOME"
          "PIPEWIRE_RUNTIME_DIR=$REAL_RT"
          "PULSE_SERVER=unix:$REAL_RT/pulse/native")

PASS=0; FAIL=0
ok()   { PASS=$((PASS+1)); echo "  ok   $1"; }
bad()  { FAIL=$((FAIL+1)); echo "  FAIL $1"; }
within() { awk -v v="$1" -v lo="$2" -v hi="$3" 'BEGIN{exit !(v>=lo && v<=hi)}'; }

cleanup() {
    [ -n "${GUI_PID:-}" ] && kill "$GUI_PID" 2>/dev/null
    [ -n "${TONE_PID:-}" ] && kill "$TONE_PID" 2>/dev/null
    [ -n "${MODULE_ID:-}" ] && pactl unload-module "$MODULE_ID" 2>/dev/null
    rm -rf "$WORK"
}
trap cleanup EXIT

echo "== calibration signals =="
ffmpeg -y -v error -f lavfi -i \
  "aevalsrc=sin(2*PI*220*t)|cos(2*PI*220*t):s=48000:d=90" "$WORK/circle.wav"
ffmpeg -y -v error -f lavfi -i \
  "aevalsrc=sin(2*PI*220*t)|0:s=48000:d=90" "$WORK/left.wav"
ffmpeg -y -v error -f lavfi -i \
  "aevalsrc=sin(2*PI*220*t)|sin(2*PI*220*t):s=48000:d=90" "$WORK/mono.wav"

echo "== front door (no window may appear) =="
"$BIN" --help >"$WORK/help.txt" 2>&1
[ $? -eq 0 ] && grep -q "usage:" "$WORK/help.txt" \
    && ok "--help prints help, exit 0" || bad "--help behavior"
"$BIN" --bogus-flag >/dev/null 2>&1
[ $? -eq 3 ] && ok "unknown flag exits 3" || bad "unknown flag exit code"
"$BIN" kit validate /nonexistent-a.phoskit /nonexistent-b.phoskit \
    >"$WORK/kit.txt" 2>&1
[ "$(grep -c '"valid":false' "$WORK/kit.txt")" = 2 ] \
    && ok "kit validate reports EVERY file" || bad "kit multi-file"

echo "== private instance up =="
"${PENV[@]}" "$BIN" >"$WORK/gui.log" 2>&1 &
GUI_PID=$!
for _ in $(seq 40); do
    [ -S "$RT/phosphor/ctl.sock" ] && break
    sleep 0.25
done
[ -S "$RT/phosphor/ctl.sock" ] && ok "control socket up" \
    || { bad "no control socket"; exit 1; }

pctl() { "${PENV[@]}" "$BIN" ctl "$@" >/dev/null 2>&1; }
pprobe() { "${PENV[@]}" "$BIN" probe --json 2>/dev/null; }

# median bbox aspect (w/h) over settled tap frames
tap_aspect() {
    "${PENV[@]}" timeout 8 "$BIN" tap 2>/dev/null \
      | grep '"event":"frame"' | head -14 | tail -8 \
      | jq -r 'select(.bbox) | (.bbox[2]-.bbox[0])/(.bbox[3]-.bbox[1]+0.0001)' \
      | sort -n | awk '{a[NR]=$1} END{if (NR) printf "%.3f", a[int((NR+1)/2)]}'
}
tap_extent() { # prints "w h" medians
    "${PENV[@]}" timeout 8 "$BIN" tap 2>/dev/null \
      | grep '"event":"frame"' | head -14 | tail -8 \
      | jq -r 'select(.bbox) | "\(.bbox[2]-.bbox[0]) \(.bbox[3]-.bbox[1])"' \
      | sort -n | awk '{w[NR]=$1; h[NR]=$2} END{if (NR) printf "%.1f %.1f", w[int((NR+1)/2)], h[int((NR+1)/2)]}'
}

echo "== PLAYER path geometry =="
pctl mode xy
pctl open "$WORK/circle.wav"; sleep 2
A=$(tap_aspect)
within "${A:-0}" 0.97 1.03 && ok "xy circle round via player (aspect $A)" \
    || bad "xy circle via player squished (aspect ${A:-none})"
pctl mode xy45; sleep 1
A=$(tap_aspect)
within "${A:-0}" 0.97 1.03 && ok "xy45 circle round via player (aspect $A)" \
    || bad "xy45 circle via player squished (aspect ${A:-none})"
pctl mode xy
pctl open "$WORK/left.wav"; sleep 2
read -r W H <<<"$(tap_extent)"
within "${H:-999}" 0 8 && within "${W:-0}" 100 99999 \
    && ok "L-only is a horizontal line (w=$W h=$H)" \
    || bad "L-only geometry off (w=${W:-?} h=${H:-?})"
pctl mode xy45
pctl open "$WORK/mono.wav"; sleep 2
read -r W H <<<"$(tap_extent)"
within "${W:-999}" 0 8 && within "${H:-0}" 100 99999 \
    && ok "mono is the M axis in xy45 (w=$W h=$H)" \
    || bad "mono xy45 geometry off (w=${W:-?} h=${H:-?})"

echo "== CAPTURE path geometry (private null sink) =="
MODULE_ID=$(pactl load-module module-null-sink sink_name=phosw1 \
    sink_properties=device.description=phosw1 2>/dev/null)
sleep 1
paplay --device=phosw1 "$WORK/circle.wav" &
TONE_PID=$!
pctl mode xy
pctl target "device:phosw1.monitor"; sleep 2
STATE=$(pprobe)
echo "$STATE" | jq -e '.source.kind == "capture"' >/dev/null \
    && ok "probe source.kind=capture after pick" \
    || bad "probe source after target pick: $(echo "$STATE" | jq -c .source)"
echo "$STATE" | jq -e '.player.paused == true' >/dev/null \
    && ok "track paused by target pick (the old dead-state)" \
    || bad "track not paused after pick"
A=$(tap_aspect)
within "${A:-0}" 0.97 1.03 && ok "xy circle round via CAPTURE (aspect $A)" \
    || bad "xy circle via CAPTURE squished (aspect ${A:-none}) ← the bug"
pctl mode xy45; sleep 1
A=$(tap_aspect)
within "${A:-0}" 0.97 1.03 && ok "xy45 circle round via CAPTURE (aspect $A)" \
    || bad "xy45 circle via CAPTURE squished (aspect ${A:-none}) ← the bug"
kill "$TONE_PID" 2>/dev/null; TONE_PID=""

echo "== resume law =="
pctl toggle; sleep 1
STATE=$(pprobe)
echo "$STATE" | jq -e '.source.kind == "player" and .capture.on == false' \
    >/dev/null \
    && ok "resume takes the beam back (capture off, player on)" \
    || bad "resume law: $(echo "$STATE" | jq -c '{source, capture}')"

echo "== single instance =="
WINDOWS_BEFORE=$(xdotool search --classname phosphor 2>/dev/null | wc -l)
"${PENV[@]}" "$BIN" >"$WORK/second.log" 2>&1
CODE=$?
WINDOWS_AFTER=$(xdotool search --classname phosphor 2>/dev/null | wc -l)
[ "$CODE" -eq 0 ] && grep -q "already running" "$WORK/second.log" \
    && [ "$WINDOWS_AFTER" -eq "$WINDOWS_BEFORE" ] \
    && ok "second launch focused, no new window (exit 0)" \
    || bad "single instance (exit $CODE, windows $WINDOWS_BEFORE→$WINDOWS_AFTER)"
"${PENV[@]}" "$BIN" "$WORK/circle.wav" >/dev/null 2>&1
sleep 1
pprobe | jq -e '.player.track | test("circle.wav")' >/dev/null \
    && ok "file forwarded to running instance" \
    || bad "file forward"

pctl quit
GUI_PID=""

echo
echo "receipts: $PASS ok, $FAIL failed"
[ "$FAIL" -eq 0 ]
