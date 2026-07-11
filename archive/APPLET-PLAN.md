# APPLET-PLAN.md — fix issue #3: the applet loses its bundled engine

Written for the next session (target executor: Opus 4.8). Everything
you need is in this file plus the named sources. Read HANDOFF.md's top
section first for house law; this plan repeats the parts that bite.

## Context (why)

The Cinnamon panel applet `phosphor-scope@phosphor` (installed at
`~/.local/share/cinnamon/applets/phosphor-scope@phosphor/`, source in
`applet/` in this repo) is a GJS applet that draws a live vectorscope
in the panel. Today it works ONLY because it bundles its own copy of
the retired v3 Python engine (`phosphor_audio.py`, `phosphor_core.py`,
`phosphor_signal.py`, `libphosphor_core.so`, driven by
`phosphor_applet_feed.py`). v3 was removed from the system on July 4,
2026; the applet is the last v3 code running anywhere. GitHub issue
**#3** tracks this; V4PLAN step 12 planned the fix for wave 3.

**Goal:** the applet draws from `phosphor feed` (a new subcommand of
the installed v4 binary) and bundles ZERO engine code. The
version-skew bug class dies. The applet's look, settings, and behavior
otherwise DO NOT CHANGE.

## Non-goals (do not build these here)

- `ctl` / `tap` / `probe` and the Unix control socket — that is issue
  #4, a separate wave-3 slice. Leave them as exit-2 stubs in main.rs.
- Any applet UI redesign, new settings, or St/Clutter rework.
- Spices store submission.
- Do NOT touch the offline render pipeline, the golden tests, or
  phosphor-dsp internals. Your change surface is: one new file
  `crates/phosphor-app/src/feed.rs`, a 3-line change in `main.rs`, a
  small diff in `applet/applet.js`, deletions in `applet/`, and
  `applet/install.sh`.

## Locked decision: transport stays stdio NDJSON

V4PLAN step 10 sketched "feed = compact binary over a Unix socket".
DECISION (July 4, recorded here): the applet transport stays exactly
what the applet already speaks — a child process writing one JSON
line per frame on stdout, reading commands on stdin. Rationale: the
applet owns the process lifecycle (no socket files, no autospawn
races, no stale-socket sweeps; up to 4 panel instances each just
spawn their own feed), the bandwidth at 30 fps × ≤500 segments is
trivial, and the applet-side diff shrinks to one spawn line. The
binary-socket feed can still arrive with #4 later; nothing here
precludes it.

## The protocol (v3-verbatim — port it exactly)

Reference implementation: `phosphor_applet_feed.py` (193 lines, in
`applet/` and in the installed applet dir). Its constants and
behavior port VERBATIM:

- stdout: one JSON object per line.
  `{"s": [x0, y0, x1, y1, i, x0, y0, x1, y1, i, ...]}` — a flat run
  of 5-int segments. Coordinates are integers in a 0..1000 box
  (`COORDINATE_BOX = 1000.0`, compute at 1000×1000 then
  `round()`); intensity is `clamp(round(intensity * 255), 0, 255)`.
  On a fatal start problem emit `{"error": "<short text>"}` and exit.
- stdin: one command per line —
  `mode <m>` where m ∈ {xy, xy45, xy_swirl, xy_dots, waveform, ring,
  spectrum, spectrum_radial, tunnel} (NOT the 3D modes — the panel
  has no camera; silently ignore invalid modes; a mode CHANGE resets
  the computer), `fps <n>` (clamp 5..480), `quit` (exit 0).
  stdin EOF (applet died / closed the pipe) = exit 0.
  stdout write failing with EPIPE = exit 0.
- Frame pacing: default 30 fps (`--fps N` arg, clamp 5..480); sleep
  `max(0, interval − elapsed)` per frame; ALWAYS emit a frame each
  tick, even when silent (`{"s": []}` is fine — the applet's decay
  handles the fade; do not add a quiet-sleep law here).
- Capture: the DEFAULT OUTPUT MONITOR at 16000 Hz
  (`CAPTURE_SAMPLE_RATE = 16000` — a panel scope needs no more, and
  it keeps frames small). If no monitor source exists:
  `{"error": "no output monitor source found"}` and exit.
- Segment cap: if a frame computes more than
  `MAX_SEGMENTS_PER_FRAME = 500`, DOWNSAMPLE BY STRIDE (pick every
  len/500-th segment), do not truncate — v3 law, keeps the whole
  shape at lower density.
- Auto-gain (the panel autosize; constants verbatim):
  `AGC_TARGET_FILL = 0.9`, `AGC_NOISE_FLOOR = 0.005`,
  `AGC_MAX_GAIN = 40.0`, `AGC_RELEASE = 0.92`. Per frame:
  `frame_peak = max(abs(sample))`;
  `tracked_peak = frame_peak if frame_peak > tracked_peak else
  tracked_peak * 0.92 + frame_peak * 0.08`;
  if `tracked_peak > 0.005`:
  `computer.gain = clamp(0.9 / tracked_peak, 1.0, 40.0)`
  else `computer.gain = computer.gain * 0.9 + 0.1` (ease to unity —
  never amplify silence into jitter).

## Part 1 — `phosphor feed` (Rust)

### main.rs (3 lines)

Remove `"feed"` from the `PENDING` array; add the match arm
`Some("feed") => feed::run(&arguments[1..]),` and `mod feed;`.
Update the module docstring's pending list (the honesty law: the
docstring must not claim feed is pending once it isn't).

### crates/phosphor-app/src/feed.rs — detailed stub

```rust
// SPDX-License-Identifier: GPL-3.0-or-later
//! `phosphor feed` — the headless panel feed for the Cinnamon applet.
//! One JSON line of beam segments per frame on stdout; mode/fps/quit
//! commands on stdin. Protocol and constants are VERBATIM from v3's
//! phosphor_applet_feed.py (the applet's paint code is unchanged) —
//! see APPLET-PLAN.md. The applet owns this process: stdin EOF or a
//! broken stdout pipe are both a clean exit.

use std::io::{BufRead, Write};
use std::sync::mpsc;
use std::time::{Duration, Instant};

const COORDINATE_BOX: f32 = 1000.0;
const DEFAULT_FPS: u32 = 30;
const CAPTURE_SAMPLE_RATE: u32 = 16_000;
const MAX_SEGMENTS_PER_FRAME: usize = 500;
const AGC_TARGET_FILL: f32 = 0.9;
const AGC_NOISE_FLOOR: f32 = 0.005;
const AGC_MAX_GAIN: f32 = 40.0;
const AGC_RELEASE: f32 = 0.92;

const VALID_MODES: &[&str] = &["xy", "xy45", "xy_swirl", "xy_dots",
    "waveform", "ring", "spectrum", "spectrum_radial", "tunnel"];

enum Command { Mode(String), Fps(u32), Quit }

/// stdin reader thread → channel. Sends Quit on EOF.
fn spawn_stdin_reader() -> mpsc::Receiver<Command> { /* … */ }

/// Flatten segments to the compact int run; stride-downsample past
/// the cap (v3 law: thin the whole shape, never cut its tail).
fn encode_segments(segments: &[[f32; 5]]) -> Vec<i64> { /* … */ }

fn write_line(payload: &serde_json::Value) -> bool { /* stdout,
    write + flush, false on any error (EPIPE = reader gone) */ }

pub fn run(arguments: &[String]) -> i32 {
    // --fps N (clamp 5..480)
    // AudioEngine::spawn(CAPTURE_SAMPLE_RATE, events)?  → on Err:
    //   write_line({"error": …}); return 3;
    // engine.default_monitor_target_id() → None: the verbatim
    //   "no output monitor source found" error line; return 3;
    // engine.start_capture(&combo_id);
    // let mut computer = Computer at CAPTURE_SAMPLE_RATE, mode xy;
    // loop:
    //   drain command channel (Mode → parse::<Mode>() + reset only
    //     if changed AND name ∈ VALID_MODES; Fps → new interval;
    //     Quit → return 0);
    //   drain engine events non-blocking (ignore; StreamEnded →
    //     try start_capture again once per second, meanwhile keep
    //     emitting empty frames — the applet shows a dark scope);
    //   samples = engine.take_stereo_samples();
    //   AGC as specified above; computer.gain = …;
    //   segments = computer.compute(&samples, 1000.0, 1000.0);
    //   if !write_line({"s": encode_segments(segments)}) return 0;
    //   sleep the remainder of the interval;
}
```

Reuse, don't rebuild: `phosphor_audio::AudioEngine` (spawn/
start_capture/take_stereo_samples/default_monitor_target_id — all
public, see `crates/phosphor-audio/src/engine.rs`),
`phosphor_dsp::Computer` + `Mode::from_str` (all nine 2-D mode names
above parse; `set_sample_rate(rate, 1)` after construction — copy the
construction pattern from `crates/phosphor-app/src/render.rs`
`build_computer`, minus kit/theme). `serde_json` is already a
workspace dep of phosphor-app.

Unit tests to include in feed.rs (pure parts): `encode_segments`
(rounding, intensity clamp, stride downsample: 1000 segments → 500
picked by stride, first segment kept), command parsing (mode
validity, fps clamp, quit), and the AGC step as a free function
(attack is instant, release is 0.92-geometric, silence eases to 1.0).

## Part 2 — the applet diff

Work on `applet/` in the repo, then reinstall to the user dir via
`applet/install.sh` (rewrite it — see below).

1. `applet.js` (529 lines): the ONLY functional change is the spawn.
   Today (~line 282): `_helperPath()` resolves the bundled
   `phosphor_applet_feed.py` and spawns
   `["python3", helperPath, "--fps", String(this.fps)]`.
   Replace with `["phosphor", "feed", "--fps", String(this.fps)]`,
   delete `_helperPath()`, and on spawn failure (GLib error —
   phosphor not installed) reuse the existing `{"error": …}` display
   path with a directive message ("install phosphor ≥ 4.0"). The
   stdin command writes (`mode …\n`, `fps …\n`, `quit\n`) and the
   stdout line reader ALREADY match the protocol — do not touch the
   paint, popup, menu, or settings code.
2. DELETE from `applet/` (and, via install, from the installed dir):
   `phosphor_applet_feed.py`, `phosphor_audio.py`, `phosphor_core.py`,
   `phosphor_signal.py`, `libphosphor_core.so` if present,
   `__pycache__/`. After install, the applet dir must contain ONLY
   `applet.js`, `metadata.json`, `settings-schema.json`,
   `stylesheet.css` (+ icon if one exists).
3. `metadata.json`: version → `2.0.0`; description gains "requires
   phosphor ≥ 4.0". Keep uuid, max-instances 4, cinnamon-version.
4. `applet/install.sh`: reduce to copying the four files into
   `~/.local/share/cinnamon/applets/phosphor-scope@phosphor/` and
   REMOVING the now-dead bundled files from an existing install
   (explicit `rm -f` list — upgrades must clean, not accrete).

## Part 3 — receipts (run every one; screenshots into the scratchpad)

1. `cargo test --workspace --release` green, `cargo clippy` silent
   (the permanent gates), plus the new feed.rs unit tests.
2. Protocol receipt, no applet involved:
   `paplay <a 30 s test tone> &` then
   `timeout 3 ./target/release/phosphor feed | head -3` —
   three JSON lines, `"s"` arrays non-empty, every coordinate in
   0..1000, every 5th value in 0..255. Then
   `printf 'mode waveform\n' | timeout 2 ./target/release/phosphor feed | tail -1`
   parses and differs in shape from xy (waveform segments span x).
   And `printf 'quit\n' | ./target/release/phosphor feed` exits 0
   fast.
3. Pipe-death receipt: `./target/release/phosphor feed | head -1`
   exits promptly after head closes (no zombie; check `pgrep`).
4. Reinstall the deb (`bash packaging/build-deb.sh` + `sudoplz sudo
   dpkg -i …`) so `/usr/bin/phosphor` has `feed`, then run
   `applet/install.sh` and reload Cinnamon (ask Ben, or
   `dbus-send --session --dest=org.Cinnamon --type=method_call
   /org/Cinnamon org.Cinnamon.Eval string:'global.reexec_self()'` —
   coordinate with Ben first; his session is live).
5. Live receipt: music playing (use a WAV from
   `~/Music/WAV versions/` — the house law is real songs, e.g.
   Attack Vector), panel scope traces it. Root-capture screenshot
   cropped to the panel (import -window root + crop; the panel is at
   the screen bottom). Popup receipt: hover/click opens the bigger
   scope, mode switch from its menu works (that exercises `mode`
   over stdin).
6. Skew-death receipt: `ls` the installed applet dir — zero `.py`,
   zero `.so`. `pgrep -f applet_feed` empty while the panel draws.
7. Kill-resilience: `pkill -f "phosphor feed"` (note: -f pattern must
   not appear in your own shell command — use `pkill -f 'phosphor fee[d]'`
   — the self-match suicide bit us twice) → applet's respawn/powered
   state behaves as it did with the python helper (read applet.js's
   subprocess wait handler to confirm expected behavior BEFORE
   testing).

## Part 4 — close-out

- PARITY.md: applet row moves to done-with-receipts; deferrals list
  shrinks to {timeline/studio (wave 4)} + whatever #4 still owes.
- HANDOFF.md: top section notes the applet is native-fed; the
  wave-3 remaining scope is #4 (control socket) + #6 (mixing UI) +
  #7 (docs) + #5 (raster worker).
- Close issue #3 with the receipts; reference the commit.
- Ben's flow: branch `v4-applet`, `--no-ff` merge, tag, push. Release
  notes law: `--notes-file`, never inline backticks. If a new deb
  ships, keep the honest-ledger pattern (the README "Not yet in v4"
  list loses its applet line; update it).

## Pitfalls that already bit this repo (do not relearn)

- `pkill -f X` where X appears in your own command line kills your
  own shell. Use `-x`, or bracket-escape the pattern.
- xdotool `key --window` needs the window FOCUSED for winit apps
  (activate first); wheel/click events go to whatever is on top
  (raise first); Ben's real mouse races synthetic input — keep
  receipt windows short.
- The applet runs INSIDE the Cinnamon process: a feed that blocks on
  stdout without the applet reading would fill the pipe — that's why
  every write checks for the reader and exits. Never log to stdout;
  diagnostics go to stderr (the applet spawns with STDERR_SILENCE).
- AudioEngine::spawn needs a live PipeWire session (fails cleanly
  with the error-line path when headless).
- gh release notes with backticks: `--notes-file` only.
- A running Phosphor GUI and a feed process are independent PipeWire
  clients — no coordination needed; do NOT try to share state.
