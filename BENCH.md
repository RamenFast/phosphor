# Phosphor v3 baseline (the numbers v4 must double)

Measured on the pristine v3 tree at `5d3e18a` (v4 Wave 1, step 1 — before
any Rust). V4PLAN targets: **≥2× fps on the GPU and CPU renderers
individually**, GPU sharpness ≥ CPU. The workloads are deterministic and
SHA-pinned so the identical test runs against v4: rerun everything with
`python3 tests/bench/run_bench.py` on the desktop session.

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

## Workloads (tests/bench/signals.py — v4 must trace these exact bytes)

| signal | what it is | why |
|---|---|---|
| sweep | the parity test's sweep + detuned right | the light/basic reference |
| chaos | 8-osc FM stack, closed-form phases, ~90 % deflection | dense evolving figure |
| noise | seeded uniform ±0.85, every sample jumps anywhere | segment ≈ screen diagonal: the fill-rate worst case |
| scene | scenes/stress-knot.scene.json (31:29 lissajous, 8192 pts, rotating, breathing) through the real studio compiler | complex *drawn* geometry, the AFTERGLOW-shaped load |
| music1 | Attack Vector.wav @ 60 s (Ben's scope-music masters, 96 k/24) | real drawn geometry; cut lives in scratch, never the repo |
| tp192 | 192k Test Pattern.wav, loop-tiled | calibration pattern, high-rate content |

Synthetic signals regenerate bit-identically (closed-form math / pinned
PCG64 stream / deterministic studio compile); every run's results record
the signal SHA-256.

## Live scope, fullscreen 2560×1440, frame cap off, mode xy

Max settings = 384 kHz detail; GL supersample 2 / Cairo resolution 1.0.
`busy`/`sclk` from amdgpu (2 Hz). **Boost clock for this card is
~2600 MHz — note the sclk column never leaves the 500 MHz floor.**

| config | signal | fps median | min–max | GPU busy | sclk MHz |
|---|---|---|---|---|---|
| GL max | sweep | 163 ◾ceiling | 159–166 | 70 % | 500–705 |
| GL max | chaos | 163.5 ◾ceiling | 157–166 | 67 % | 500–645 |
| GL max | noise | 163 ◾ceiling | 160–166 | 64 % | 500–635 |
| GL max | scene | 163 ◾ceiling | 158–166 | 68 % | 500–705 |
| GL max | music1 | 163 ◾ceiling | 159–166 | 63 % | 500–610 |
| GL max | tp192 | 163 ◾ceiling | 152–166 | 51 % | 500–520 |
| GL ss1 384k | noise | 164 ◾ceiling | 122–166 | 56 % | 500–560 |
| GL ss1 96k | sweep | 165 ◾ceiling | 161–166 | 51 % | 500–520 |
| GL ss2 96k, xyz_takens | sweep | 164 ◾ceiling | 159–165 | 64 % | — |
| Cairo max | sweep | 7 | 5–9 | 10 % | 500 flat |
| Cairo max | chaos | 8 | 8–8 | 16 % | 500 flat |
| Cairo max | noise | **3** | 3–3 | 11 % | 500 flat |
| Cairo max | scene | 8 | 7–8 | 12 % | 500 flat |
| Cairo max | music1 | **5** | **1–9** | 6 % | 500 flat |
| Cairo 96k | sweep | 43–45.5 ◾declining | 20–57 | 25 % | — |

`ms py` reads 0.1–0.2 ms on every GL run (main-thread python is cheap —
the rust core computes segments); Cairo computes in a worker thread, so
its fps is the honest number.

### The cold-machine sag (real, reproduced once, mechanism open)

The very first (cold) round measured the *same* GL configs sagging:
`gl-default-96k` steady-state ~100 fps (97–105), `gl-384k-ss1` locked
~163 for 24 s then 90–104. Warm rounds never sag. The clock sampler
rules OUT the obvious story — sclk sits at the 500 MHz floor whether
locked at 165 or sagging — so the mechanism is elsewhere (CPU governor,
compositor scheduling, PipeWire delivery cadence…). Both states are
recorded in `results/v3-baseline.json` history (first round) and this
table (warm). The v4 requirement is state-independence: **hold ≥157 fps
at every load level from any starting state.** This bistability is
Ben's lived "sub-100 fps with the GPU nearly idle."

### Vsync-off experiment: v3 cannot exceed the display rate. Period.

Mesa `vblank_mode=0` on the unredirected fullscreen window
(`*-novsync` runs): fps stays pinned at 165 (max 166) for noise,
music1, and the 96 k config alike. GL swap is not the limiter — the
**GTK3 frame clock is**; ticks are paced by the compositor's frame
cycle and no setting or environment frees them. A >165 Hz live number
is architecturally impossible for v3. This, plus the 500 MHz sclk
column, is the whole GPU case for v4: the card spends the entire
baseline in its lowest power state while the pipeline around it
decides the frame rate.

## Offline render throughput (no frame clock, 720×720, mode xy)

| detail rate | signal | fps-equivalent | × realtime |
|---|---|---|---|
| 96 kHz | sweep | 85.6 | 1.43× |
| 96 kHz | chaos | 114.3 | 1.90× |
| 384 kHz | sweep | 39.3 | 0.65× |
| 384 kHz | chaos | 40.2 | 0.67× |
| 384 kHz | noise | **9.8** | **0.16×** |

## What v4 must beat (the gate, concretely)

v4's `phosphor bench` runs BOTH renderers uncapped offscreen on the
same signals (same SHA-256s). Gates, each ≥2× v3:

| metric | v3 | v4 gate |
|---|---|---|
| Live CPU, 384 k max, noise | 3 fps | ≥ 6 (aim far higher) |
| Live CPU, 384 k max, music1 | 5 fps (min 1) | ≥ 10, min ≥ 5 |
| Live CPU, 384 k max, sweep/chaos/scene | 7–8 fps | ≥ 16 |
| Live CPU, 96 k | 43 fps declining | ≥ 91, no decline |
| Live GPU, any signal, any state | 163 ◾ceiling / cold-sag to ~100 | holds ≥157 from any state; ≥2× uncapped offscreen vs the 163 ceiling (≥326) |
| Offline render 96 k sweep / chaos | 85.6 / 114.3 | ≥ 171 / ≥ 229 |
| Offline render 384 k sweep / noise | 39.3 / 9.8 | ≥ 79 / ≥ 20 |
| GPU sharpness | GL softer than CPU | GPU ≥ CPU (screenshot compare) |

The GPU uncapped gate is measured by v4's own offscreen bench because
v3 architecturally has no uncapped GPU mode (see the vsync section) —
its 163 @ 500 MHz floor clock stands as the number to double, with
~5× clock headroom and 30–50 % busy headroom untouched.

## Reading the numbers

1. **The GPU never woke up.** Every v3 workload, including
   full-deflection noise at max supersample, runs at the 6750 XT's
   500 MHz floor state. The frame pipeline (GTK frame clock + GSK +
   per-frame python/ctypes orchestration) sets the pace; the silicon is
   a spectator. v4's wgpu mailbox loop is where that headroom cashes in.
2. **The CPU path defines the emergency**, and *noise* (not pretty
   lissajous) defines the CPU path: 3 fps live, 9.8 fps-eq offline.
   Budget phosphor-render-cpu against screen-diagonal segments.
3. **Real music is harsher than synthetics for Cairo** (5 fps median,
   dips to 1 on Attack Vector — bursty geometry), and indistinguishable
   from synthetics for GL (everything ceilings). Both renderers get the
   music workloads in every future round.
4. **Distributions, not medians** — the cold-sag and the min columns
   are the story; v4's bench reports percentiles.
