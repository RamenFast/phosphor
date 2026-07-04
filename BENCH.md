# Phosphor v3 baseline (the numbers v4 must double)

Measured on the pristine v3 tree at `5d3e18a` (v4 Wave 1, step 1 — before
any Rust). V4PLAN targets: **≥2× fps on the GPU and CPU renderers
individually**, GPU sharpness ≥ CPU. Where a live number is pinned to the
monitor's 165 Hz vsync ceiling, the honest comparison currency is the
frame-clock-free offline throughput and the sag-free-ness of the frame
distribution — both recorded below.

## Machine

| | |
|---|---|
| CPU | AMD Ryzen 5 3600 (6c/12t, Zen 2) |
| GPU | AMD Radeon RX 6750 XT — Mesa 25.2.8, radeonsi, LLVM 20.1.2 |
| Monitor | 2560×1440 @ 164.83 Hz |
| Kernel | 6.17.0-35-generic |
| Audio | PulseAudio 15 on PipeWire 1.0.5 |
| Stack | Python 3.12.3 · GTK 3.24.41 · numpy 1.26.4 · rust core API v2 |
| Measured | 2026-07-04, idle desktop, no other Phosphor running |

## Method

- `tests/bench/run_bench.py` orchestrates; `tests/bench/bench_probe.py`
  runs the real app in-process (`Gio.ApplicationFlags.NON_UNIQUE`,
  scratch `HOME`, fullscreen with WM-race retry) and samples the app's
  own FPS overlay once a second for 30 s after a 6 s warmup. The overlay
  is the shipped readout: `fps` = drawn frames, `ms py` = python +
  GL-CPU work per frame **on the main thread** (the Cairo renderer
  computes in a worker, so its `ms py` reads ~0 — fps is the honest
  number there), `max` = worst inter-frame gap.
- Audio: a deterministic stereo sweep (the parity test's signal,
  cycling every 8 s) played by `paplay` into a `module-null-sink`
  (silent by construction, tone outlasts every run, module unloaded on
  every exit path). The scope captures the null sink's monitor.
- Liveness is checked against the fps counter's window clock, not label
  text — a quiet scope fails the run instead of freezing the numbers.
- GPU utilization from amdgpu `gpu_busy_percent` at 2 Hz.
- Offline: `phosphor --render` (720×720, EXPORT_FPS 60, offline Cairo
  renderer + libx264) over a 15 s tone.
- Rerun everything: `python3 tests/bench/run_bench.py` (desktop session
  required). Partial: `BENCH_ONLY=<prefix>`, `BENCH_SKIP_OFFLINE=1`.
  Raw samples: `tests/bench/results/v3-baseline.json`.

## Live scope, fullscreen 2560×1440, uncapped (Max FPS = 0), mode xy

| config | detail | fps median | fps min–max | ms py | worst gap | proc CPU | GPU busy |
|---|---|---|---|---|---|---|---|
| GL, supersample 2 (max) | 384 kHz | **162** ◾vsync-locked | 157–166 | 0.2 | 18 ms | 0.35 core | 67 % |
| GL, supersample 1 | 384 kHz | 162.5 ◾bimodal, see below | 90–166 | 0.2–0.3 | 30 ms | 0.32 core | 55 % |
| GL, supersample 1 | 96 kHz | **102** ◾sagged steady state | 97–165 | 0.2 | 18 ms | 0.24 core | 50 % |
| GL, ss 2, mode xyz_takens | 96 kHz | 162 ◾vsync-locked | 159–165 | 0.2 | 12 ms | 0.33 core | 64 % |
| Cairo, resolution 1.0 (max) | 384 kHz | **7** | 5–9 | (worker) | 206 ms | 1.02 core | 10 % |
| Cairo, resolution 1.0 | 96 kHz | **45.5** ◾declining 57→22 | 20–57 | (worker) | 73 ms | 1.03 core | 25 % |

◾ **The GL sag — v3's real "sub-100 fps" pathology.** At supersample 2
the GPU has enough sustained work to hold the vsync ladder (162 fps for
the full 30 s). At supersample 1 the same scene runs locked ~163 for
24 s, then falls to 90–104 and stays there; at 96 kHz detail the sag
arrives within ~4 s and 97–105 *is* the steady state — with the GPU
under 60 % busy and 0.2 ms of python per frame. Lighter loads run
*slower*. Consistent hypothesis (unproven here): amdgpu power management
downclocks under partial load and the GTK3 frame clock + GSK composite
never claws back onto the vsync ladder. This matches the lived
complaint in HANDOFF ("sub-100 fps with the GPU nearly idle") better
than raw throughput does — v4's wgpu mailbox present + owned frame loop
must hold the ladder at *every* load, not just heavy ones.

◾ **The Cairo decline** at 96 kHz (57→22 over 30 s) is the skipped-frame
feedback loop: dropped frames leave samples queued, the next frame
traces more audio, gets slower, drops more. One core pegged throughout
= the GIL ceiling. At 384 kHz fullscreen it is 7 fps — for reference it
measured 8 fps windowed at 1600×924, so resolution barely matters: the
bottleneck is per-segment stamping cost on one core.

## Offline render throughput (no frame clock, 720×720, mode xy)

| detail rate | wall for 15 s of audio | fps-equivalent | × realtime |
|---|---|---|---|
| 96 kHz | 11.2 s | **80.6** | 1.34× |
| 384 kHz | 23.1 s | **39.0** | 0.65× — slower than realtime |

## What v4 must beat (the gate, concretely)

| metric | v3 baseline | v4 gate (≥2×) |
|---|---|---|
| Offline render, 96 kHz | 80.6 fps-eq | ≥ 161 fps-eq |
| Offline render, 384 kHz | 39.0 fps-eq | ≥ 78 fps-eq (≥1.3× realtime) |
| Live CPU renderer, 384 kHz max, fullscreen | 7 fps | ≥ 14 fps (aim much higher) |
| Live CPU renderer, 96 kHz, fullscreen | 45.5 fps declining | ≥ 91 fps, no decline |
| Live GPU renderer, any setting | sags to 90–104 | holds ≥ 157 at every load level |
| Live GPU renderer, uncapped throughput | vsync-hidden (162 @ 67 % busy) | `phosphor bench` offscreen ≥ 2× its own v3-equivalent measurement* |

\* v3 has no offscreen GPU path, so the uncapped GPU comparison is
established by v4's `phosphor bench` against this table's locked-state
+ GPU-busy numbers: at minimum, ≥324 fps-equivalent offscreen at the
gl-max workload, and GPU sharpness ≥ CPU per the screenshot compare.

## Reading the numbers

1. **GPU throughput is not v3's problem — frame pacing is.** 67 % busy
   at a locked 162 fps says the 6750 XT swallows the max workload; the
   sag at partial loads is where the lived slowness comes from.
2. **The CPU path is the emergency** and the clearest 2× win: one GIL
   core stamping segments vs v4's rayon + 8-wide SIMD tiles.
3. **`ms py` ≈ 0.2 everywhere on GL** — with the rust core computing
   segments, main-thread python math is already cheap; the cost v4
   deletes is the pipeline around it (ctypes GL calls, GTK frame clock,
   GSK composite), plus the two-engine parity tax.
4. **Distributions, not medians.** The ss1 run's median (162.5) hides a
   90 fps tail. v4's `phosphor bench` must report percentiles; this
   file's min–max columns are the precedent.
