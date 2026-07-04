# Phosphor v4 — the full-Rust rewrite ("one engine")

Ben-approved July 4, 2026. This is the executable map; HANDOFF.md points here.

## Context

v3 tops out below 100 fps on a 165 Hz monitor with an RX 6750 XT nearly idle,
because Python drives every frame: segment marshaling → ~30 ctypes GL calls →
GTK3 frame clock → GSK composite, all under the GIL. The GPU path also looks
*softer* than CPU (linear-filtered RG16F supersampling + tonemap). And v3 pays
a permanent tax: two signal engines (Python + Rust core) kept in exact parity.

**Targets (Ben-approved):** ≥2× FPS on CPU and GPU renderers *individually*,
GPU sharpness ≥ CPU, zero Python, no fallback paths, all backlog features ride
along. Decisions locked: **egui + wgpu** (iced as structural inspiration),
**native PipeWire**, **in-place migration** (v3 stays runnable until a parity
checklist passes, then one commit deletes it), **GPLv3** throughout.

Hardware verified: Zen 2 (AVX2+FMA+F16C, no AVX-512), RADV reports
`shaderFloat16=true`. Toolchain ready: Rust 1.96, libpipewire-0.3-dev 1.0.5,
scdoc, wgpu v28 era.

## Architecture

Cargo workspace in-repo (existing `core/` folds in). One `phosphor` binary,
subcommand-first; GUI is the default command.

```
crates/
  phosphor-proto       .phos (FIXED 256-byte header, fit-trim rules), .phoskit,
                       scene/timeline JSON, tap/probe types, shipped JSON schemas
  phosphor-dsp         samples→segments: ALL 11 v3 modes (xy, xy45, xy_swirl, xy_dots,
                       xyz_takens, helix, waveform, ring, spectrum, spectrum_radial,
                       tunnel) + new views; kit chain; polyphase sinc oversampling;
                       autocorrelation τ; rustfft; compose resampler
  phosphor-beam        THE beam model (sigma, energy deposit, tonemap) + themes as
                       data files + 3D orbit camera — both renderers consume it
  phosphor-render-gpu  wgpu: decay/beam/composite in WGSL + bloom chain; f16 energy
                       when shaderFloat16; offscreen mode for headless
  phosphor-render-cpu  rayon tiles + 8-wide SIMD (AVX2/FMA via runtime dispatch,
                       `wide` crate) — same beam model, parity by construction
  phosphor-audio       pipewire-rs capture (monitor/app/mic) + playback; symphonia
                       decode (ffmpeg remains ONLY as mp4 mux pipe for render);
                       vacuum routing; multi-app mixer; gapless preload; cover art
  phosphor-studio      scene compiler + timeline tier (build, beat grid, morphs,
                       wireframe3d, Hershey vector font, camera automation)
  phosphor-app         egui shell: scope canvas as a raw wgpu pass in the same
                       surface, transport, playlist, kit editor, studio panel,
                       mini/glass modes, MPRIS via zbus, control socket
applet/                GJS rewrite — ZERO bundled engine; draws segments streamed
                       from `phosphor feed` over a Unix socket
```

**CLI surface (all agent-grade: `--output json`, exit codes 0/2/3/4):**
`phosphor` (GUI) · `render` · `bench` · `probe` · `tap` · `ctl` · `feed` ·
`kit validate|inspect` · `studio render|validate|inspect|preview|build` ·
`--mini` · `--screensaver` · `--visitor`. `phosphor-studio` stays as a compat
alias.

**Contracts that do not change:** .phos bytes, .phoskit JSON, scene JSON,
settings at `~/.config/phosphor/settings.json` (v3 keys read/migrated), CLI
flags above, .deb via `packaging/build-deb.sh`.

## Hard-won v3 constraints that port verbatim (from HANDOFF)

- Kit math: f64 phase accumulators advanced per chunk, `rem_euclid`/`% TAU`,
  f64 trig cast to f32 BEFORE f32 sample math, integer-sample channel delays,
  state zeroed on reset/configure.
- Precompute playback stays clock-synced at any Max FPS (fixed-step was tried
  and reverted).
- Vacuum: reader paces itself (rolling deadline, re-anchor >0.25 s behind);
  never `-re`; restore is sacred AND every launch sweeps stale vacuum
  artifacts; check return codes not stdout.
- Energy-buffer allocation failure handling (shed supersample, never draw into
  a broken FBO) — port the logic to wgpu error scopes.

## Waves

### Wave 1 — The engine (proof of the 2×)
1. **Baseline first:** measure v3 fps (GPU + CPU, max settings, uncapped where
   possible) and record numbers in `BENCH.md`. Capture **golden fixtures from
   live v3**: a dump script runs the Python engine to record segments for
   every mode × audio fixture, kit-chain outputs, .phos round-trips →
   `tests/golden/`. This happens before any Rust code, while v3 is pristine.
2. Workspace scaffold; `core/` absorbed into `crates/`.
3. `phosphor-proto` + `phosphor-dsp`: port all 11 modes + kits + oversampling.
   Gate: golden-fixture parity (documented tolerance).
4. `phosphor-beam` + both renderers. GPU: WGSL ports of the three passes,
   mailbox present, f16 energy path. CPU: rayon+SIMD rasterizer replacing
   Cairo stamping. Sharpness fix: sRGB-correct output, proper supersample
   downfilter.
5. Headless `phosphor render` (symphonia decode → offscreen wgpu or CPU raster
   → ffmpeg mux pipe), renders .phos too.
6. `phosphor bench`: uncapped fixture render, JSON output, becomes the
   permanent regression gate.
7. **Spikes (front-loaded, X11/Muffin):** winit transparent window (glass
   mode), file drag-and-drop, egui+wgpu hello at 165 Hz.

**Exit:** bench ≥2× v3 on both paths; screenshot compare shows GPU ≥ CPU
sharpness; golden tests green.

### Wave 2 — The shell (daily-drivable)
8. `phosphor-audio`: pipewire-rs capture/playback; vacuum port with the
   sacred-restore + startup-sweep invariants (**escape hatch:** if PW node
   routing misbehaves, hybrid mode keeps pactl subprocess for module
   load/unload only); per-app capture + **multi-app mixing**; **gapless**
   preload; **cover art** from symphonia metadata.
9. egui app to parity: scope canvas, capture selector, transport, playlist
   (DnD, shuffle/repeat, per-stream volume), now-playing overlay, MPRIS both
   directions (zbus), compose/draw mode, snapshot/clip/pin/grid/fps/fullscreen,
   mini mode (magnetism + Align), glass toggle, 3D orbit + idle drift, kit
   editor, .phos play/export with credit fade, visitor mode, full keyboard
   map, settings migration. Scope themes + UI styles become **data files**
   (ship v3's 8+8, add new ones — AMOLED-black first-class).

**Exit:** v4 is Ben's daily driver; parity checklist green except applet +
timeline.

### Wave 3 — Agents & the panel
10. Control socket (Unix, NDJSON): `ctl` (play/pause/mode/theme/…), `tap`
    (per-frame JSON: t, mode, segment count, bbox, centroid, dominant pitch,
    downsampled polyline), `feed` (compact binary for the applet).
    `probe --at T file --output json` for offline "what's on screen as data."
11. `kit validate|inspect` family; shipped JSON schemas for .phoskit + scenes;
    scdoc manpages. Error text short and directive — a 7B model repairs its
    kit in one round-trip (Ben's bedtime note).
12. **Applet GJS rewrite:** panel scope + popup with ALL scope views, themes,
    power toggle, autostart — no bundled engine, auto-spawns `phosphor feed`.
    Version-skew class deleted.
13. **Icon:** 4-panel SVG, one shape per panel (xy knot / Takens torus /
    waveform / radial spectrum) + desktop file.
14. Docs rewrite: MANUAL.md v4 covering everything (it's 4 releases stale),
    README, "pantomime" gets its promised home.

**Exit:** an agent can drive the scope end-to-end without pixels; applet live
on Ben's panel via LookingGlass check.

### Wave 4 — The show
15. Studio timeline tier: `timeline.json` + `studio build` → one flac; beat
    grid (pure-Rust onset detection, aubio CLI as fallback dep); morphs;
    wireframe3d through the shared camera; vector font; multi-stroke retrace
    blanking; camera automation keyframes.
16. GUI studio panel + inotify hot-reload; `preview --watch` as the working
    mode.
17. Bloom polish pass; **screensaver** (`--screensaver`: fullscreen, no
    chrome, cursor hidden, exits on input, music else generative scenes in
    vacuum, xautolock/DPMS-respecting).
18. **`AFTERGLOW-SESSION.md`** — the creative-session handoff: timeline
    workflow, Lyria 3 via OpenRouter ($0.08/song) for music beds, structure
    sketch, greets (Fenderson, brakence, woscope, Ben, Nexus), Windowlicker
    mode-switch trick.
19. **Parity checklist final pass → one commit deletes the Python tree** →
    packaging/build-deb.sh reworked for a compiled amd64 deb → v4.0.0 tag +
    deb + HANDOFF update.

**Exit:** v4.0.0 tagged, v3 gone, creative session ready to run.

## Testing regime (more than "once")

- **Golden fixtures** captured from live v3 (wave 1, step 1) — segments per
  mode, kit outputs, .phos round-trips; failures name the mode + fixture.
- **Property tests** (proptest): kit phase wraparound, oversampling
  invariants, .phos header fit-trim, scene traversal constant-speed.
- **Snapshot render tests:** both renderers vs the shared beam model
  (perceptual-hash with tolerance); GPU-vs-CPU sharpness comparison automated.
- **Studio golden hashes** ported (`--record` re-pins deliberately).
- **Bench baseline** in-repo; CI-style local gate: a bench regression fails
  the wave.
- **Vacuum integration script:** null-sink loopback exercising route →
  kill -9 → sweep → restore.
- No fallback paths anywhere. An error surfaces; nothing silently degrades.

## Risks (named now, spiked early)

- pipewire-rs vacuum routing (riskiest port) — hybrid pactl escape hatch
  preserved.
- winit transparency/DnD on Muffin — wave 1 spike before we're pot-committed.
- 384 kHz scope rate × 165 Hz on CPU path — measured in bench, budgeted, not
  assumed.
- egui look ≠ GTK-native — mitigated by owning themes completely (that's a
  feature).

## Verification

Wave-by-wave: bench JSON vs recorded v3 baseline; automated screenshot parity;
golden/property/snapshot suites via `cargo test`; live daily-drive on Ben's
box (capture, vacuum, MPRIS media keys, glass, mini); applet checked in
LookingGlass; final `phosphor render` of turtle.scene.json must still produce
the turtle.
