#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Gate A — the vacuum invariants, exercised against the REAL PipeWire
# server with real processes (V4PLAN testing regime):
#
#   route → verified move → kill -9 → the sink module LINGERS and the
#   app keeps playing into the void (v3 crash semantics) → the next
#   launch's sweep unloads it and the server rescues the stream →
#   graceful route/release restores the ORIGINAL sink exactly.
#
# The sink lifecycle is pactl module load/unload (the pre-authorized
# hatch; receipt: native node destroy kills pulse-shim streams on
# PW 1.0.5 — "Connection terminated"). Routing/verify/restore are
# native metadata + link-watch. pactl below is only the independent
# WITNESS for assertions. Run from the repo root:
#   bash tests/vacuum/gate.sh
set -u
cd "$(dirname "$0")/../.."

PASS=0; FAIL=0
check() { # name ok detail
  if [ "$2" = 1 ]; then echo "PASS $1: $3"; PASS=$((PASS+1));
  else echo "FAIL $1: $3"; FAIL=$((FAIL+1)); fi
}

app_sink_index() { # sink index the gate-tone stream currently plays into
  pactl list sink-inputs | awk '
    /^Sink Input #/ { sink="" }
    /^[[:space:]]*Sink:/ { sink=$2 }
    /application\.name = "gate-tone"/ { print sink; exit }'
}
sink_name_by_index() {
  pactl list short sinks | awk -v idx="$1" '$1==idx{print $2}'
}
app_sink_name() { sink_name_by_index "$(app_sink_index)"; }
vacuum_sink_exists() {
  pactl list short sinks | grep -q "phosphor_vacuum"
}

cargo build -q -p phosphor-audio --example vacuum_ctl || exit 1
CTL=target/debug/examples/vacuum_ctl
WORK=$(mktemp -d)
trap 'kill $TONE_PID 2>/dev/null; rm -rf "$WORK"' EXIT

ffmpeg -v error -y -f lavfi -i "sine=frequency=330:sample_rate=48000:duration=120" \
  -ac 2 "$WORK/tone.wav"
paplay --client-name=gate-tone --volume=16000 "$WORK/tone.wav" 2>"$WORK/tone.err" &
TONE_PID=$!
sleep 1.2

ORIG_NAME=$(app_sink_name)
check "baseline: tone playing on a real sink" \
  "$([ -n "$ORIG_NAME" ] && echo 1 || echo 0)" "on: ${ORIG_NAME:-<none>}"

# ---- 1. route, verified move ------------------------------------------------
"$CTL" route gate-tone forever > "$WORK/route.log" 2>&1 &
CTL_PID=$!
for _ in $(seq 1 50); do grep -q ROUTED "$WORK/route.log" && break; sleep 0.1; done
check "route confirmed by link" \
  "$(grep -q ROUTED "$WORK/route.log" && echo 1 || echo 0)" \
  "$(head -1 "$WORK/route.log")"
sleep 0.4
check "vacuum sink exists" "$(vacuum_sink_exists && echo 1 || echo 0)" \
  "$(pactl list short sinks | grep phosphor_vacuum | cut -f1-2 | head -1)"
MOVED_NAME=$(app_sink_name)
check "app moved into the vacuum" \
  "$([ "$MOVED_NAME" = "phosphor_vacuum" ] && echo 1 || echo 0)" \
  "now on: ${MOVED_NAME:-<none>}"

# ---- 2. kill -9: module lingers, app keeps playing into the void -------------
kill -9 "$CTL_PID"
sleep 1.5
check "kill -9: sink module lingers (v3 crash semantics)" \
  "$(vacuum_sink_exists && echo 1 || echo 0)" \
  "the void persists until the next launch sweeps"
LIMBO_NAME=$(app_sink_name)
check "kill -9: app still alive, silent in the void" \
  "$([ "$LIMBO_NAME" = "phosphor_vacuum" ] && echo 1 || echo 0)" \
  "on: ${LIMBO_NAME:-<gone>}"

# ---- 3. the next launch sweeps; the server rescues the stream ----------------
SWEEP_OUT=$("$CTL" sweep)
sleep 0.8
SWEPT_COUNT=${SWEEP_OUT#swept }
check "sweep unloaded the stale module" \
  "$([ "${SWEPT_COUNT:-0}" -ge 1 ] && ! vacuum_sink_exists && echo 1 || echo 0)" \
  "$SWEEP_OUT; sinks clean"
check "no zombie module rows" \
  "$([ "$(pactl list short modules | grep -c phosphor_vacuum)" = 0 ] && echo 1 || echo 0)" \
  "pactl module table clean"
RESCUED_NAME=$(app_sink_name)
check "sweep rescued the stream to a real sink" \
  "$([ -n "$RESCUED_NAME" ] && [ "$RESCUED_NAME" != "phosphor_vacuum" ] && echo 1 || echo 0)" \
  "now on: ${RESCUED_NAME:-<gone>}"

# ---- 4. graceful route + release restores the original sink ------------------
"$CTL" route gate-tone 2 > "$WORK/route2.log" 2>&1
check "graceful release ran" \
  "$(grep -q RELEASED "$WORK/route2.log" && echo 1 || echo 0)" \
  "$(tail -1 "$WORK/route2.log")"
FINAL_NAME=$(app_sink_name)
check "restore is sacred: back on the original sink" \
  "$([ -n "$FINAL_NAME" ] && [ "$FINAL_NAME" = "$ORIG_NAME" ] && echo 1 || echo 0)" \
  "${FINAL_NAME:-<gone>} (was $ORIG_NAME)"
check "no vacuum left behind" "$(vacuum_sink_exists && echo 0 || echo 1)" "sinks clean"

echo "----"
echo "gate A: $PASS passed, $FAIL failed"
exit "$([ "$FAIL" -eq 0 ] && echo 0 || echo 1)"
