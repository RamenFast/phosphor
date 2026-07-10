# DECAY-DT-SPEC — the single authoritative dt-decay specification

Status: FINAL ruling of node `decay-spec-unify` (2026-07-10). Merges and supersedes the
`decay-repair-design` and `decay-refdt-reconcile` artifacts wherever they conflict, and folds in
`decay-deposit-dtnorm`, `decay-dt-callsite-spec`, and `decay-other-perframe-consumers`.
The repair/implementation node implements THIS document only.

---

## Ruling 1 — REF_DT anchor: **1/60 s** (reconcile ADOPTED, design's 1/164.83 REJECTED)

`pub const REF_DT: f32 = 1.0 / 60.0;` in phosphor-beam.

Receipts:
- The literal keeps FLASH_KEEP=0.50 (phosphor-beam/src/lib.rs:33) and glow_keep(p)
  (lib.rs:50-52, default 0.82) are what every golden, checksum, and doc-law is anchored to,
  with "one advance == one frame" semantics throughout: keep_laws_match_v3
  (phosphor-beam/src/lib.rs:448-451), render-cpu inline tests (lib.rs:254/:264, :308/:310/:315,
  :324/:329), bench checksums (phosphor-app/src/bench.rs:205,:208,:212,:224), export outputs
  (exports.rs:118,:180). All exports/bench are media-time at EXPORT_FPS=60 (render.rs:24).
- Design's 1/164.83 rests only on BENCH.md:15 (Ben's monitor refresh) and an inference that
  constants were "tuned live at ~165 Hz". A monitor refresh is not a physics anchor; adopting it
  would re-baseline every multi-advance golden (effective 60 Hz glow keep 0.82 → 0.82^2.747 ≈ 0.580,
  flash 0.50 → 0.149) and visibly shorten trails ~3x in every 60 fps export.
- Under REF_DT=1/60 with the bit-exact short-circuit (Ruling 2), `advance(segments, REF_DT)` is
  bit-identical to today's `advance(segments)`: NOTHING re-baselines.
- Design's actual goal (preserve the live 165 Hz look) is still satisfied: at dt=1/165 the powf
  path gives glow keep 0.82^(60/165) ≈ 0.9304, i.e. the live high-refresh look is the
  frame-rate-independent look by construction.

Open item for Ben (informational only, does not block implementation): the live look at 165 Hz
will subtly change relative to today (today it over-decays at 165 Hz; after this fix it matches
the 60 fps export look). This is the intended bug fix, not a regression.

## Ruling 2 — keep formula: **powf with bit-exact literal short-circuit** (reconcile adopted)

```
fn decay_step(dt: f32, persistence: f32) -> DecayStep
if dt == REF_DT (exact f32 ==):
    flash_keep = FLASH_KEEP (0.50 literal)
    glow_keep  = glow_keep(persistence)  // existing fn, lib.rs:50-52
    floors     = [ENERGY_FLOOR, ENERGY_FLOOR]  // 0.0004 literal
else:
    n          = dt / REF_DT
    flash_keep = FLASH_KEEP.powf(n)
    glow_keep  = glow_keep(persistence).powf(n)
    floors     = per Ruling 3
```

`keep_ref.powf(dt/REF_DT)` and `exp(-dt/tau)` with `tau = -REF_DT/ln(keep_ref)` are the same
function mathematically; powf is chosen because it needs no stored tau constants and expresses
the anchor directly. The dt==REF_DT special case is MANDATORY: f32 ln/exp/powf round-trips are
not bit-identical to the literals, and bit-exactness at REF_DT is what keeps all goldens green.
Do NOT re-baseline expectations.

Keeps compose exactly (k^a · k^b = k^(a+b)), so no subdivision loop is ever needed.
Document this in the decay_step doc comment so nobody re-adds one.

## Ruling 3 — ENERGY_FLOOR scaling: **geometric-series closed form** (design adopted, reconcile's linear form REJECTED)

```
keep_total = keep_ref.powf(n)                       // n = dt/REF_DT
floor_total = ENERGY_FLOOR * (1 - keep_total) / (1 - keep_ref)
E' = max(E * keep_total - floor_total, 0)
```

This is the EXACT closed form of sub-stepping the current per-step (keep_ref, floor) process n
times: `E·k^n − f·(1+k+…+k^(n−1))`. Reconcile's linear `ENERGY_FLOOR·n` is wrong for n>1 because
each step's floor subtraction is itself decayed by subsequent keeps. Divergence is material:

| plane | n=dt/REF_DT | geometric | linear | linear over-subtracts |
|---|---|---|---|---|
| glow p=0.7 (k=0.82) | 2 | 0.000728 | 0.000800 | 1.10x |
| glow p=0.7 | 6 (dt=0.1s) | 0.001547 | 0.002400 | 1.55x |
| glow p=0.7 | 30 (dt=0.5s) | 0.002216 | 0.012000 | 5.41x |
| flash (k=0.50) | 6 | 0.000788 | 0.002400 | 3.05x |
| flash | 30 | 0.000800 | 0.012000 | 15.0x |

The geometric floor converges to `f/(1−k)` (flash 0.0008, glow(0.7) ≈ 0.00222); the linear form
grows without bound and would visibly stomp faint trails after any stall. For n<1 (high refresh)
the geometric form correctly gives a sub-reference floor as well.

Floors are per-plane (keep_ref differs per plane): `DecayStep { flash_keep, glow_keep, floor: [f32;2] }`.
At dt==REF_DT both floors short-circuit to the literal 0.0004.

GPU plumbing: DecayUniforms currently has `keep: vec2f, _pad: vec2f` (shaders.wgsl:9-12, 16-byte
buffer at render-gpu/src/lib.rs:445). The two floors go into `_pad` → `floor: vec2f`, and the
hardcoded `0.0004` literal at shaders.wgsl:28 is replaced by the uniform (also fixes the law-9
one-fact-one-representation violation). Shader math shape unchanged, no per-pixel exp/powf.

## Ruling 4 — dt clamp: **dt.clamp(0.0, 0.25)**; no lower clamp

One value replaces all three proposals (0.5 design / 0.1 callsite-spec / [1/240,1/24] reconcile):

- **Upper 0.25 s.** With geometric floors, a 0.25 s step leaves glow(p=1.0, k=0.98) at
  keep_total = 0.98^15 ≈ 0.74 minus its floor: long stalls decay plenty without integrating a
  window-drag or debugger pause into a full flash-out. Design's 0.5 s and 0.25 s are visually
  near-equivalent for p≤0.9; 0.25 halves the worst-case post-stall dimming step. Callsite-spec's
  0.1 s is too tight (a 7 fps struggling frame is real elapsed time and should decay as such).
- **No lower clamp** (reconcile's 1/240 floor rejected): dt below 1/240 is legitimate at high
  refresh or coalesced frames; powf handles n<1 exactly. Negative dt (clock anomaly) clamps to 0
  = no-op decay (keep_total=1, floor_total=0 — decay_step must return exactly that at dt=0).
- Clamp lives inside `decay_step` so every consumer inherits it.

Wake/first-frame rule (restated from design §4/§6): the live sim clock is
`last_advance_enqueue: Option<Instant>` on the Shell. On first frame, after quiet-law wake
(shell.rs:2807-2809), and after pause/target switch, the anchor is None → use dt = REF_DT
(one reference step, no catch-up flash-out). Sleep never accumulates dt (anchor reset to None).

raster_worker dt-folding (restated from callsite-spec, adopted as specified there):
`RasterJob` gains `dt: f32` (raster_worker.rs:20-39), stamped at enqueue against
`last_advance_enqueue` (shell.rs:2910 submit site); restyle submits (shell.rs:2955, advance=false)
neither read nor bump the anchor and carry dt=0. Mailbox folding: in `RasterWorker::submit`, if
the overwritten job has `old.advance`, fold `old.dt` into the new job; if the new job is
advance=false, the worker stashes it in a `carry_dt` added to the next advancing job. Folded dt
is clamped to 0.25 s at consumption (inside decay_step), so drop storms cannot overshoot.
GPU live path (shell.rs:2902) has no queue: its own `last_gpu_advance` anchor, same clamp.

## Ruling 5 — exports.rs final partial chunk: **dt = samples/rate**; render.rs full chunks only (REF_DT)

Only exports.rs produces short final chunks: `chunks()` yields a short trailing chunk
(exports.rs:110-119 snapshot path, :175-184 clip path). Rule there: offline dt is ALWAYS media
time, `dt = (chunk.len()/2) as f32 / rate`, which equals REF_DT exactly for full chunks
(per_frame = rate/EXPORT_FPS·2) and the correct shorter dt for the final partial chunk.

render.rs is different: its offline loop reads fixed-size frames with `read_exact` and treats a
read_exact error as end-of-stream ("read_exact error = trailing partial frame: done",
render.rs:~483-484). The trailing partial chunk is DROPPED and never rendered — render.rs today
has no short-dt final frame. Therefore render.rs passes exactly REF_DT for every frame it
renders (full chunks only).

Drop-vs-render is an explicit implementation choice for the repair node:
- **Keep dropping the trailing partial frame** (default): zero behavior change; render.rs never
  needs a non-REF_DT dt.
- **Start rendering it** with `dt = samples/rate`: a deliberate output change (one extra final
  frame in exports whose sample count is not a multiple of rate/60) that MUST be called out in
  the commit note per law 5.

Determinism impact: fully deterministic — dt is a pure function of input length and rate, so
byte-identical inputs give byte-identical outputs. In exports.rs, all frames except at most the
last one hit the dt==REF_DT bit-exact path, so existing export goldens change at most in the
final frame, and only for inputs whose sample count is not a multiple of rate/60. Reconcile's
"unconditional REF_DT" would silently treat a partial frame as a full one, which is precisely
the class of bug this migration exists to kill. Never use wall clock offline; never use
frame_index live.

bench.rs (:205,:212) is media-time with exact full chunks: passes REF_DT, checksums unchanged.
`render()` no-dt convenience wrapper (render-cpu lib.rs:239-242) stays, defaulting to REF_DT,
keeping timing.rs:37,41 and cross_snapshot.rs:119,208 byte-identical. Inline render-cpu tests
(:264,:310,:315,:329) pass REF_DT.

## Ruling 6 — deposit-side normalization and other per-frame consumers (integrated)

From `decay-deposit-dtnorm` (adopted in full, with REF_DT=1/60 confirming its assumption):

- **Streaming modes** (xy/xy45/xy_dots/swirl at modes/xy.rs:98-105, takens): energy-per-sample,
  already fps-invariant on the deposit side; dt-decay fixes their steady-state fps-dependence
  automatically. NO deposit change.
- **Restamped modes** MUST scale their per-frame stamp: `intensity *= dt / REF_DT`, dt being the
  same value passed to advance for that frame (so at 60 fps and offline this is exactly 1.0 and
  nothing changes). Applies to:
  - compose preview (compose.rs:22 PREVIEW_INTENSITY=0.25, restamped at :243-263)
  - waveform (modes/waveform.rs:52), spectrum/spectrum_radial (modes/spectrum.rs:94),
    ring, helix, tunnel (waveform-history dispatch, dsp/lib.rs:349-361)
  - visitor segments (shell.rs:2870-2873)
  **Application point (RULED, no discretion):** the scaling is applied in **phosphor-app only**,
  as a single post-compute pass over the assembled segment `Vec`, in exactly the two live paths:
  1. **shell.rs frame()** — immediately after the visitor extend (shell.rs:2870-2877), before
     `let segments = &segments[..]` (shell.rs ~2878), i.e. before the tap broadcast, before
     `gpu.advance(segments)` (shell.rs:2944) and before the `RasterJob` submit (shell.rs:2951).
     One pass covers all three restamped sources in that vec: compose preview segments,
     restamped-mode Computer output, and visitor segments. The dt used is the SAME clamped dt
     value computed for that frame's advance (Ruling 4 anchor), captured once into a local
     before both consumers.
  2. **compose preview offline export path is N/A** (export_compose_drawing synthesizes audio,
     not segments) — no other live segment source exists.

  **phosphor-dsp is NOT touched.** `Computer::compute()` keeps no dt parameter; modes/*.rs emit
  exactly what they emit today, so `crates/phosphor-dsp/tests/golden_replay.rs` (byte-for-byte
  segment-row comparison) is provably unchanged — the scaling never executes inside the crate
  under golden test.

  **Mode classification API:** add to phosphor-dsp `impl Mode` (lib.rs:81, beside `name()`):
  ```rust
  /// True for modes whose segments restamp a held picture every frame
  /// (intensity is per-frame, not per-sample); their deposit must be
  /// dt-normalized by the caller. Streaming modes deposit energy per
  /// sample and are already fps-invariant.
  pub fn is_restamped(self) -> bool {
      matches!(self, Mode::Waveform | Mode::Ring | Mode::Helix | Mode::Tunnel
                   | Mode::Spectrum | Mode::SpectrumRadial)
  }
  ```
  This is a pure classification method, adds no behavior to dsp, changes no goldens. shell.rs
  applies the factor when `self.compose_drawing || self.computer.mode.is_restamped()` to the
  Computer/compose portion of the vec, and unconditionally to the visitor-appended tail
  (visitor segments are always restamped). Simplest correct shape: scale the compose/Computer
  portion first (gated), then extend with visitor segments pre-scaled, OR record the vec length
  before the visitor extend and scale the two ranges — implementer's choice, both are one pass.

  **Bit-exactness offline and at 60 fps:** the multiply is short-circuited, mirroring Ruling 2:
  ```rust
  if dt != phosphor_beam::REF_DT {           // exact f32 ==
      let factor = dt / phosphor_beam::REF_DT;
      for seg in scaled_range { seg[4] *= factor; }
  }
  ```
  When dt == REF_DT (exact f32 equality) NO arithmetic touches the intensities — not even a
  `*= 1.0` — so offline paths (render.rs full frames, bench.rs, exports.rs full chunks) and
  60 fps live frames are bit-identical to today with zero float perturbation. exports.rs's
  final partial chunk (Ruling 5) legitimately gets factor < 1.0; that is the intended behavior,
  not a golden break (only the last frame of a non-multiple-length export changes, already
  accepted under Ruling 5). render.rs/bench.rs pass REF_DT for every frame, so the branch never
  fires there.

  **exports.rs offline restamp:** exports.rs runs Computer directly (exports.rs:117,:207) with
  media-time dt; apply the identical short-circuited pass there right after compute(), gated on
  `computer.mode.is_restamped()`, so live/offline parity (T5) holds for restamped modes too.
  Factor it as one small helper in phosphor-app (e.g. `pub(crate) fn restamp_scale(segments:
  &mut [Segment], dt: f32)` in signals.rs or a new decay.rs) called from shell.rs, exports.rs —
  ONE implementation, no copies.

  Use the clamped dt, so a stall cannot produce a >2.5x bright flash (dt≤0.25 → factor ≤15;
  acceptable since it is one frame of a "held picture" that decays immediately — if visually
  objectionable, clamp the deposit factor separately to ≤4, decided at implementation with eyes on).
- age_weight (dsp/lib.rs:398-413): re-derive the per-chunk grading keep each frame as
  `glow_keep(p).powf(dt/REF_DT)` so intra-chunk grading matches buffer decay (second-order polish;
  may land in a follow-up commit).
- dsp `frame_glow_keep=0.82` (dsp/lib.rs:135,204): intra-frame per-wave brightness fade, NOT
  buffer-time decay. Stays per-frame, out of scope, annotated in code as deliberately excluded
  from law 8. dsp goldens unchanged.

From `decay-other-perframe-consumers` (adopted):

- **FADE_OUT_FRAMES=90 → wall-clock deadline** (shell.rs:34,:1670,:2371,:2712-2714): replace the
  frame counter with `fade_out_until: Option<Instant>` set to now + FADE_OUT_SECONDS (1.5 s,
  = 90/60, preserving today's 60 fps wall time). The render loop stays awake until the deadline
  passes AND planes are zero. begin_visitor's 240 fps frame math (shell.rs:1670) converts to
  seconds the same way.
- Feed AGC 0.92 (feed.rs:28) and shell auto-gain 0.999/0.05 (shell.rs:2282-2288): control-loop
  smoothing, deliberately per-frame, NOT part of this migration. Optional later polish.
- chrome.rs ThemeXfade, cycle_song_fade: already Instant-based, no change.

## API surface (single source of truth, law 9)

In phosphor-beam/src/lib.rs:
```rust
pub const REF_DT: f32 = 1.0 / 60.0;
pub const FLASH_KEEP: f32 = 0.50;        // existing, unchanged
pub const ENERGY_FLOOR: f32 = 0.0004;    // existing, unchanged
pub fn glow_keep(persistence: f32) -> f32;  // existing, unchanged
pub struct DecayStep { pub flash_keep: f32, pub glow_keep: f32, pub floor: [f32; 2] }
pub fn decay_step(dt: f32, persistence: f32) -> DecayStep;  // clamps dt, short-circuits dt==REF_DT
```
Both renderers consume ONLY DecayStep. `advance(&mut self, segments, dt: f32)` on CpuRenderer
(render-cpu lib.rs:105) and GpuRenderer (render-gpu lib.rs:514); FrameSink::advance (render.rs:48-53)
gains dt and passes through. settings.persistence keeps its type, range, default, ctl key, and UI
slider unchanged; no migration.

## Implementation order (for the repair node)

1. phosphor-beam: REF_DT, DecayStep, decay_step (clamp + short-circuit + geometric floors) + tests T2/T3/T4.
2. render-cpu: advance(segments, dt) consuming DecayStep; keep `render()` REF_DT wrapper; tests T1/T5.
3. render-gpu: advance(dt), upload keep+floor vec2s into DecayUniforms `_pad`, replace wgsl 0.0004
   literal with uniform (shaders.wgsl:28).
4. shell.rs: `last_advance_enqueue` sim clock, RasterJob.dt + submit folding + worker carry_dt,
   GPU-path anchor, FADE_OUT_FRAMES → fade_out_until deadline, visitor seconds math.
5. Restamp dt-normalization: `Mode::is_restamped()` in phosphor-dsp (classification only, no
   golden change), plus one shared `restamp_scale(&mut [Segment], dt)` helper in phosphor-app
   applied post-compute in shell.rs frame() (before tap/gpu.advance/RasterJob) and in exports.rs
   after compute(); short-circuits dt==REF_DT (exact ==) so offline/60fps paths are bit-identical.
   phosphor-dsp Computer takes NO dt parameter (Ruling 6).
6. exports.rs: media-time dt = samples/rate incl. partial final chunk + test T6; render.rs
   passes REF_DT (full chunks only; decide drop-vs-render for the trailing partial frame per
   Ruling 5); bench.rs passes REF_DT explicitly.
7. age_weight dt-correction (optional polish commit).
8. Golden check: with the bit-exact short-circuit NO goldens should change except final-partial-
   chunk export frames; if any other checksum moves, that is a bug in the implementation, not a
   re-baseline. Commit note per law 5 regardless: "physics parameterized by dt, REF_DT=1/60
   bit-exact to previous per-frame behavior".

## Acceptance tests (final, updated to these rulings)

- **T1 cross-FPS invariance**: fresh CpuRenderer, one impulse deposit, advance to t=0.5 s in steps
  of 1/30, 1/60, 1/144, 1/240; each rate's plane energy at the first frame boundary ≥ 50 ms
  matches the analytic `E0·keep_total − floor_total` within rel 0.5% + abs 1e-6.
- **T2 stall equivalence**: one 0.25 s step vs 15 steps of 1/60 from the same seeded plane:
  max abs per-pixel diff ≤ 1e-5 (geometric closed form is the exact sub-step limit).
- **T3 clamp**: dt=10 s bitwise-equals dt=0.25 s; dt=-1 and dt=0 are exact no-ops
  (keep=1, floor=0).
- **T4 bit-exact anchor**: `decay_step(REF_DT, p).glow_keep == glow_keep(p)` (exact ==) for
  p ∈ {0,0.25,0.5,0.7,0.9,1.0}; `.flash_keep == FLASH_KEEP`; both floors `== ENERGY_FLOOR`.
  Plus monotonicity: keep(1/165) > keep(1/60) > keep(1/30) per plane.
- **T5 live/offline parity**: same segment stream + same dt sequence via exports offline_pipeline
  and via direct advance: frames identical (max abs diff 0). CPU-vs-GPU decay parity at varied
  dt ∈ {1/165, 1/60, 1/30, 0.25} via existing cross_snapshot harness tolerance.
- **T6 partial final chunk (exports.rs)**: input with sample count not a multiple of rate/60:
  the exports.rs chunk→dt computation gives last-frame dt = remaining samples/rate (unit-test
  it), and a full-multiple input is byte-identical to the pre-migration output. render.rs is
  covered by this test ONLY if the implementer chooses (Ruling 5) to start rendering the
  trailing partial frame; if it keeps dropping it, assert instead that render.rs's frame count
  equals floor(samples / (rate/60·2)) and every rendered frame uses REF_DT.
- **T7 restamp invariance**: restamped mode (waveform) steady-state plane energy after 1 s at
  1/30 vs 1/144 stepping agrees within rel 2% (deposit dt-normalization × dt-decay cancel).
  Exercised through the shared `restamp_scale` helper + CpuRenderer advance directly (unit
  test in phosphor-app), not through dsp. Plus: (a) `restamp_scale(segs, REF_DT)` leaves the
  slice bitwise unchanged (exact byte compare, proving the short-circuit); (b)
  `Mode::is_restamped()` returns true exactly for {waveform, ring, helix, tunnel, spectrum,
  spectrum_radial} and false for {xy, xy45, xy_swirl, xy_dots, xyz_takens}.
- **T8 existing suites green unchanged**: keep_laws_match_v3, render-cpu inline tests, bench
  checksums, timing.rs, cross_snapshot.rs — zero expected-value edits.

## Ruling 7 — f16 energy-buffer quantization under small dt (node `decay-gpu-f16-quantization`)

The GPU energy buffer is rg16float (probe at render-gpu/src/lib.rs:222-229, fallback
rgba16float — same 10-bit mantissa either way), while decay_fs (shaders.wgsl:24-29) computes in
f32 (textureLoad yields f32, math is f32) and rounds to f16 only on the render-target write
(round-to-nearest-even). Stall condition for an "undead trail": the per-frame decrement
`E·(1−keep_total) + floor_total` falls below half-ulp(E), making round(E·keep−floor) == E.

Quantified (worst plane = glow at persistence 1.0, keep_ref 0.98; flash keep 0.50 is ~35x safer):

- Worst relative half-ulp of f16 in the normal range [2^-14, 65504]: **2^-11 ≈ 4.883e-4**
  (E just above a power of two; near the top of a binade it improves to 2^-12).
- Relative keep-decrement for small n: `1 − 0.98^(60·dt) ≈ 1.2122·dt`
  (60·|ln 0.98| = 1.2122). Per-fps: 60 fps → 2.02e-2 (41x margin), 165 fps → 7.34e-3 (15x),
  1000 fps → 1.211e-3 (**2.48x margin**), stall onset at dt < 4.03e-4 s ≈ **2483 fps**.
- The floor term `floor_total ≈ 0.02424·dt` (geometric form, small n) is absolute, so it alone
  guarantees decay whenever `E < 0.02424·dt / 4.883e-4 ≈ 49.6·dt` (at 1000 fps: all E < 0.0497),
  independent of the keep term. And once E ≤ floor_total the shader's max() clamps to exact 0.0.
- True-zero guarantee holds: f16 subnormals bottom out at ulp 2^-24 ≈ 6e-8, three orders of
  magnitude below floor_total even at 1000 fps, so the tail cannot dither above zero — it is
  subtracted through the subnormal range and clamped to 0.0 exactly.
- Deposit side: the beam pipeline is pure additive One/One/Add on color AND alpha with no
  saturation (render-gpu/src/lib.rs beam_pipeline blend; float render targets do not clamp),
  so the CPU steady-state analysis carries over. Small-deposit absorption (deposit < half-ulp
  of accumulated E) has the same ~2.4 kfps onset because dt-normalized deposits and half-ulp of
  the dt-invariant steady state both scale together until that bound.

**Ruling**: no shader or format change for the supported regime. fps 60..1000 (the observed
uncapped ceiling on this rig per the shell Mailbox receipt) is safe with ≥2.48x margin at the
worst persistence. Mitigation for beyond-spec frame rates is **dt accumulation, not a minimum
decrement** (a decrement floor would break fps invariance): the GPU live path's decay anchor
MUST fold dt forward and skip the decay pass for a frame whose accumulated dt < **DECAY_MIN_DT
= 1/1000 s**, running one decay with the accumulated dt on the next frame that crosses the
threshold. Keeps compose exactly (k^a·k^b = k^(a+b), Ruling 2) and the geometric floor is the
exact sub-step limit (Ruling 3), so folding is bit-honest and restores the ≥2.48x margin at any
fps. CPU f32 planes need none of this (f32 half-ulp 2^-25 relative; stall would need ~10^7 fps).
Implementation note: this is the same carry mechanism as raster_worker carry_dt (Ruling 4); the
GPU path reuses its `last_gpu_advance` anchor.

### Acceptance test T9 — GPU decay-to-zero at small dt

Offscreen GpuRenderer (rg16float path), seed one bright impulse (post-deposit peak energy
≥ 8.0 so E sits in a coarse binade), then advance with dt = 1/1000 at persistence 1.0 until
2.5 s of simulated time, reading back the energy plane every 0.25 s:

1. Peak energy is strictly monotonically decreasing at every sample (no stall plateau), and
2. the entire plane is exactly 0.0 by t = 2.5 s (analytic: keep_total(2.5 s) = 0.98^150 ≈ 0.048,
   then floored to zero well before that), and
3. the dt-accumulation fold: 1000 steps of dt=5e-4 (folded pairwise by DECAY_MIN_DT) matches
   500 steps of dt=1e-3 within 1 f16 ulp per pixel.

Skip (with a logged reason) on hosts with no adapter, per the existing GPU test convention.
