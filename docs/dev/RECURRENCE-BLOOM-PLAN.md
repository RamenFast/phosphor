# 🎯 RECURRENCE BLOOM — implementation plan (the 12th mode + the 13th room)

Authored 2026-07-10 under the fable-to-opus contract, from `docs/dev/BLOOM-SPEC.md`
(the arc's deliverable-2 input). Every anchor below verified against HEAD at
authoring time (baseline `c6f7867`, all suites green, clippy silent).

**Status: PLAN ONLY — nothing below is implemented.**

**Symbol legend** (stable): ▸ task · 📁 file · ✅ verify · ↩ rollback ·
⛔ constraint · ⚠ gotcha · ❓ decision.

---

## Context — the load-bearing discoveries

The spec asks for "a vectorscope that remembers the music": a chroma-driven polar
flower (the bloom) over the familiar Lissajous carrier, with visually similar
moments from the recent past returning as fading ghosts. The codebase already
carries most of the physics; the plan is mostly *analysis + geometry*, not engine
surgery:

1. **Modes are enum variants, not plugins.** `Mode` +
   `Mode::ALL: [Mode; 11]` + `name()` + `FromStr` live in
   📁 `crates/phosphor-dsp/src/lib.rs:66-115`. Dispatch is a match in
   `Computer::dispatch` (lib.rs:335-368). Mode state lives as fields on
   `Computer` (lib.rs:130-179). Adding a variant automatically extends the ctl
   `mode` verb (shell.rs:1396-1411 parses via FromStr, error `fix` lists
   `Mode::ALL`) and the agent schema enums (agent.rs:590 maps `Mode::ALL`).
2. **The UI mode list is a DUPLICATED representation** —
   `DISPLAY_MODES: [(&str, &str); 11]` at 📁 `crates/phosphor-app/src/chrome.rs:36`
   feeds both the settings-panel combo (chrome.rs:222-230) and the context menu
   (chrome.rs:1872). **No test pins it against `Mode::ALL`** (verified by grep).
   This is audit §6.3's exact drift class; Phase 4 adds the missing pin test.
3. **The spec's "CRT model" already exists.** Dual reservoirs = phosphor-beam's
   FLASH/GLOW machinery; modes only emit `Segment = [x0,y0,x1,y1,intensity]`.
   Bloom implements **zero decay code**. Ghost fading is *in-mode geometry*
   (brightness as a function of age in seconds — time-based by construction).
   The underlying beam decay becomes FPS-honest when `DECAY-DT-SPEC.md` ships;
   spec validation #6 is receipted **after** that repair (see Verification).
4. **Time = samples, never wall clock** (lib.rs doctrine, line 23-24: "swirl
   phase advances by sample count"). Bloom's precession/novelty clocks advance by
   frames consumed ÷ rate. This makes validation #7 (bitwise-deterministic
   geometry) hold by construction, including headless `phosphor render`.
5. **The spectrum family is the analysis template**: mono fold from the
   `waveform_history` tail, FFT every other frame, fast-attack/slow-fall levels
   (📁 `crates/phosphor-dsp/src/modes/spectrum.rs:15-42`). But its hz/bin
   (46.9 Hz at 48 k/1024) is far too coarse for pitch classes — chroma uses a
   **Goertzel bank at exact pitch frequencies** instead (~0.1 ms/hop, inside the
   spec's 1.5 ms budget). Flux/centroid reuse the existing `Fft`.
6. **Audio reaches modes post-kit** (lib.rs:315-333: kit chain runs before
   dispatch) and **outside any RT callback** — `compute()` runs on the render
   side, samples arrive via ring drain. The spec's "no allocation or locking in
   the audio callback" is satisfied structurally; bloom just preallocates its own
   rings so the *frame* path doesn't churn either.
7. **Settings are flat keys with clamps** in
   📁 `crates/phosphor-proto/src/settings.rs` (struct ~line 20+, `SETTINGS_KEYS`
   list ~196-236, `take!` decoding ~304, clamp precedent at ~324). Knobs flow to
   the engine per frame in 📁 `crates/phosphor-app/src/render.rs:205-209`
   (`computer.mode`/`computer.gain` from settings).
8. **Themes**: beam-color presets = `THEME_NAMES: [&str; 10]` (chrome.rs:50) —
   the spec's "Phosphor: P7/amber/ice/custom" control **already exists there,
   don't duplicate it**. Chrome rooms = `PALETTES: [Palette; 12]`
   (📁 `crates/phosphor-app/src/theme.rs:56`, tests pin count/ids at
   theme.rs:476-520, `ui_style` verb at shell.rs:1429). The companion theme is
   the 13th room.
9. **Probe** JSON is assembled in shell.rs (~1601, `beam_cycle` precedent:
   present when animating, null/absent otherwise) and mirrored by the schema in
   agent.rs (~614) — **the snapshot-vs-schema test from commit `1ec2f6c` fails
   if you update one half only**. That test is the honesty guard; use it.
10. **Compose loops force xy** (📁 `crates/phosphor-app/src/compose.rs:82-85`),
    so bloom is automatically excluded from compose recordings. Nothing to do.

## Resolved decisions (planner-owned — don't re-ask)

- **Mode id `recurrence_bloom`**, enum `Mode::RecurrenceBloom`, UI label
  **"Bloom · recurrence"** (fits the family-prefix idiom: "Spectrum · tunnel").
- **Chroma = Goertzel bank**, 12 classes × 5 octaves (C2..B6, MIDI 36-95), exact
  frequencies, Hann-windowed 4096-sample window @48 k (scaled by feed ratio),
  computed once per 1024-frame hop. FFT rejected for chroma: 46.9 Hz/bin cannot
  resolve semitones below ~C4; Goertzel is deterministic and ~250 k MACs/hop.
- **Feature vector z = 25 dims**: chroma 12 + tonnetz 6 + band-flux 4 + width 1 +
  coherence 1 + centroid 1, L2-normalized. Ring capacity 1536 hops ≈ 32 s.
  Recurrence = cosine affinity, non-local exclusion ~1 s, threshold 0.80, top-K.
- **Ghosts re-render stored envelope parameters** (chroma + warp/width/coherence
  per hop, 16 f32 ≈ 98 KB ring), not stored segments — cheap, and ghosts rotate/
  shrink/fade as functions of age in *seconds*.
- **New-mode numeric doctrine**: compute f64, cast f32 at emission; scale by feed
  ratio alone (the python-lineage law, lib.rs:11-18) — bloom never oversamples.
- **Segment budget**: envelope 480 + carrier ≤1200 + ghosts ≤6×240 = ≤3120 <
  `MAX_POINTS_PER_FRAME` 4000. Degrade order per spec: ghosts, curve samples.
- **Tempo Lock is cut from v1** (needs a tempogram; lowest load-bearing control).
  Phase 6 notes it as FUTURE with the concrete design (onset-envelope
  autocorrelation, existing takens-autocorr precedent). Ship without it.
- **Presets**: three shippable personalities (Instrument / Night Garden / Deep
  Listening) as a one-shot knob-writer combo. "Windows 2001" and "Broadcast
  Ghost" need scene-energy/scanline machinery phosphor doesn't have — FUTURE,
  honestly ledgered.
- **Novelty flash is beam-current only, never full-field**, soft attack ≥80 ms,
  hard-capped ≥333 ms between events (≤3/s). Content-driven brightness at that
  rate is not a strobe; the existing photosensitivity prompt (which guards
  sub-1 s *color cycling*) is NOT re-armed by bloom. State this in the MANUAL.
- **Companion theme: "Night Garden"** (id `night_garden`), 13th room, dark,
  `accent_follows_beam: true` — the garden is lit by whatever the bloom glows.
- ❓ (Ben, at plan review — defaults chosen, execution proceeds on them):
  label wording, Night Garden hexes, preset trio membership.

## 🤖 Execution model

Serial phases, one executor session (or one session per phase — each phase is
independently shippable with all suites green). No parallel fan-out: phases 1→4
share `Computer`/`Settings` state and would conflict in one tree. The executor
runs every ✅ itself. Rollback is git: every phase is ONE commit; ↩ = revert it.

### ⛔ Global do-not-touch

- ⛔ `tests/golden/` fixtures and the goldens law (docs/dev/GOLDEN.md). Bloom
  adds NO golden fixtures — it has no v3 reference. Its regression pins are new
  deterministic unit tests in phosphor-dsp.
- ⛔ The applet feed protocol (frozen v3; feed.rs) — bloom changes nothing there.
- ⛔ The kit parity contract (lib.rs doctrine block) and `.phos` 256-byte header.
- ⛔ `keyboard.rs:174` `is_3d` mode list — bloom is 2D; do not add it (camera
  keys must not capture input in bloom).
- ⛔ Existing mode arms in `dispatch` and their constants — bloom is additive.
- ⛔ Ben's live scope's default ctl.sock. Receipts run isolated (ARC-BRIEF §law 4:
  own XDG_RUNTIME_DIR, PIPEWIRE_RUNTIME_DIR handed back, or
  PHOSPHOR_NO_SINGLE_INSTANCE=1 / --background).
- ⛔ No release, no tag, no version bump (Ben decides; ARC-BRIEF law 7).

### ⚠ Verified gotchas

- ⚠ `cargo clippy --workspace --all-targets -- -D warnings` is a hard gate. An
  inert module whose functions are only used by tests trips `dead_code` in
  non-test builds — that's why Phase 1 wires the enum variant + dispatch arm
  immediately (agent-reachable, UI-invisible) instead of landing a dark module.
- ⚠ `set_sample_rate` calls `reset()` (lib.rs:273); reset() must drop/zero bloom
  state or a rate switch renders stale ghosts at wrong frequencies.
- ⚠ `fft.magnitudes` allocates two size-N vecs per call (fft.rs:41-42, existing
  behavior, audit §4.2). Bloom calls it once per hop for flux/centroid — same
  cost class as spectrum mode. Don't "fix" it here; that's the audit §4.2 repair.
- ⚠ The snapshot-vs-schema test (`1ec2f6c`) fails if probe JSON and agent.rs
  schema diverge — update both in the SAME task (Phase 3 ▸3.4).
- ⚠ MANUAL.md:67 says "## Eleven modes" — a count that must move to twelve, or
  the docs lie (audit §5.6 class).
- ⚠ Silence: match spectrum's cold-start law (spectrum.rs:13-14) — below-epsilon
  levels draw NOTHING (no idle circle), so the resting-beam dot (v4.3.0 chrome
  feature) keeps owning true silence.
- ⚠ Mode switch away/back must not resurrect stale ghosts: `UiAction::ModeChanged`
  path ends in `render.rs:205-208` setting `computer.mode`; bloom state keys its
  ring validity on consumed-sample continuity (simplest: drop BloomState when a
  dispatch arrives with `mode != RecurrenceBloom` — see ▸1.6).

---

## Phase 0 — baseline receipts (no code)

▸ 0.1 Pin the baseline.
✅ `git rev-parse --short HEAD` → record; `cargo test --workspace` → 20/20
suites; `cargo clippy --workspace --all-targets -- -D warnings` → silent.
✅ `cargo run --release -- bench` (or `phosphor bench` if installed) → note the
four numbers as the before-figures for the Phase 6 bench guard.
↩ nothing to roll back.

## Phase 1 — the bloom engine: analysis + envelope (agent-reachable, UI-invisible)

One commit: `feat(dsp): recurrence bloom mode — chroma bloom + carrier (engine only)`.

▸ 1.1 📁 `crates/phosphor-dsp/src/lib.rs` — add the variant end-of-enum:
`RecurrenceBloom` in `Mode`, in `Mode::ALL` (now `[Mode; 12]`), and
`Mode::RecurrenceBloom => "recurrence_bloom"` in `name()`.
✅ `cargo test -p phosphor-dsp` still green (FromStr picks it up; nothing
dispatches yet until ▸1.5).

▸ 1.2 📁 `crates/phosphor-dsp/src/lib.rs` — constants next to the mode families:

```rust
pub(crate) const BLOOM_HOP: usize = 1024;            // frames @48k, scaled by feed
pub(crate) const BLOOM_ANALYSIS_WINDOW: usize = 4096; // frames @48k, scaled by feed
pub(crate) const BLOOM_THETA_SAMPLES: usize = 480;
pub(crate) const BLOOM_CARRIER_MAX_POINTS: usize = 1200;
pub(crate) const BLOOM_GHOST_THETA: usize = 240;
pub(crate) const BLOOM_HISTORY_HOPS: usize = 1536;    // ≈ 32 s of hops @48k
pub(crate) const BLOOM_FEATURE_DIMS: usize = 25;
pub(crate) const BLOOM_EXCLUSION_HOPS: usize = 47;    // ≈ 1 s non-local window
pub(crate) const BLOOM_SIMILARITY_FLOOR: f64 = 0.80;
```

And the `Computer` field (one line, near `takens_*`):
`pub(crate) bloom: Option<Box<crate::modes::bloom::BloomState>>,` — `None` in
`new()`, `self.bloom = None;` in `reset()`.

▸ 1.3 📁 `crates/phosphor-dsp/src/modes/mod.rs` — `pub(crate) mod bloom;`

▸ 1.4 📁 `crates/phosphor-dsp/src/modes/bloom.rs` — NEW. The core, complete:

```rust
// SPDX-License-Identifier: GPL-3.0-or-later
//! Recurrence Bloom — the vectorscope that remembers the music.
//! Chroma (Goertzel bank at exact pitch frequencies) shapes a closed polar
//! envelope; a feature ring brings similar past moments back as ghosts.
//! All math f64, cast f32 at emission (python-lineage doctrine). Time is
//! samples consumed / rate — never wall clock. Decay is NOT computed here:
//! segments carry intensity, phosphor-beam owns persistence; ghost fading
//! is geometry (a function of age in seconds).

use std::f64::consts::{PI, TAU};

use crate::{age_weight64, Computer, BLOOM_CARRIER_MAX_POINTS, BLOOM_EXCLUSION_HOPS,
            BLOOM_FEATURE_DIMS, BLOOM_GHOST_THETA, BLOOM_HISTORY_HOPS,
            BLOOM_SIMILARITY_FLOOR, BLOOM_THETA_SAMPLES, SQRT_HALF};

/// Chromatic-index petal harmonics: fifths-adjacent classes share a family,
/// so related notes make visibly related petals (the spec's deliberate
/// aliasing). Derived from h = 3 + fifths_position/3.
const PETAL_HARMONIC: [f64; 12] = [3.0, 5.0, 3.0, 6.0, 4.0, 6.0, 5.0, 3.0, 5.0, 4.0, 6.0, 4.0];
/// Stable per-class phase offsets: golden-angle spacing, seeded, never random.
const GOLDEN_ANGLE: f64 = 2.399_963_229_728_653;
/// Slow per-class precession (rad/s of media time), alternating sense.
fn petal_omega(k: usize) -> f64 {
    let rate = 0.02 + 0.012 * ((k % 3) as f64);
    if k % 2 == 0 { rate } else { -rate }
}
/// Octave weights folding C2..B6 into 12 chroma bins (bass slightly soft,
/// treble rolls off — keeps pads and leads from monopolizing petals).
const OCTAVE_WEIGHT: [f64; 5] = [0.8, 1.0, 1.0, 0.9, 0.7];

pub(crate) struct GoertzelBank {
    /// (coeff, cos, sin, hann-normalization) per pitch C2..B6, chromatic-major
    /// order: pitch = octave*12 + class.
    coefficients: Vec<(f64, f64, f64)>,
    window: usize,
}

impl GoertzelBank {
    pub(crate) fn new(sample_rate: f64, window: usize) -> GoertzelBank {
        let coefficients = (0..60)
            .map(|pitch| {
                let midi = 36 + pitch; // C2..B6
                let hz = 440.0 * 2f64.powf((midi as f64 - 69.0) / 12.0);
                let omega = TAU * hz / sample_rate;
                (2.0 * omega.cos(), omega.cos(), omega.sin())
            })
            .collect();
        GoertzelBank { coefficients, window }
    }

    /// Hann-windowed Goertzel magnitudes for the 60-pitch bank over the last
    /// `window` mono samples. Output normalized by window/4 (Hann coherent gain).
    pub(crate) fn magnitudes(&self, mono: &[f64], out: &mut [f64; 60]) {
        let n = self.window.min(mono.len());
        let tail = &mono[mono.len() - n..];
        let norm = n as f64 / 4.0;
        for (pitch, &(coeff, cosine, sine)) in self.coefficients.iter().enumerate() {
            let (mut s0, mut s1, mut s2) = (0.0f64, 0.0, 0.0);
            for (i, &sample) in tail.iter().enumerate() {
                let hann = 0.5 - 0.5 * (TAU * i as f64 / (n as f64 - 1.0)).cos();
                s0 = sample * hann + coeff * s1 - s2;
                s2 = s1;
                s1 = s0;
            }
            let real = s1 - s2 * cosine;
            let imaginary = s2 * sine;
            out[pitch] = (real * real + imaginary * imaginary).sqrt() / norm;
        }
    }
}

/// One analysis hop's stored face — enough to re-render its envelope as a ghost.
#[derive(Clone, Copy)]
pub(crate) struct HopFrame {
    pub chroma: [f64; 12],
    pub warp: f64,
    pub width: f64,
    pub coherence: f64,
    /// media time of this hop, seconds (samples consumed / rate)
    pub t: f64,
}

pub(crate) struct BloomState {
    pub goertzel: GoertzelBank,
    pub mono: Vec<f64>,          // rolling mono fold, capacity 2×window
    pub frames_since_hop: usize,
    pub samples_consumed: u64,   // frames (stereo pairs) — the media clock
    pub chroma_smooth: [f64; 12],
    pub flux_bands_prev: [f64; 4],
    pub features: Vec<[f64; BLOOM_FEATURE_DIMS]>, // ring, newest last
    pub hops: Vec<HopFrame>,                       // parallel ring
    pub novelty: f64,
    pub flash_level: f64,        // 0..1, decays by media time
    pub last_flash_t: f64,
    pub current: HopFrame,
    pub ghost_indices: Vec<(usize, f64)>, // (ring index, similarity) newest search
    pub magnitude_scratch: Vec<f32>,
    pub window: usize,
    pub hop: usize,
}
```

The remainder of bloom.rs (same file, same commit) implements, in order — each
with the exact signature so the executor writes no interfaces of their own:

- `impl BloomState { pub(crate) fn new(sample_rate: f64, window: usize, hop: usize) -> Box<BloomState> }`
  — preallocates every ring at capacity (`Vec::with_capacity`, then never grows
  past it: push+drain-front pattern like `extend_waveform_history`).
- `fn analyze_hop(&mut self, fft: &mut crate::Fft, gain: f64)` — Goertzel →
  fold 60 pitches into 12 chroma with `OCTAVE_WEIGHT` → per-bin fast-attack
  (max) / slow-fall (×0.90) smoothing into `chroma_smooth` (the spectrum-family
  law) → tonnetz 6-projection (standard fifths/major-third/minor-third circles:
  `Σ c_k·cos/sin(k·7·π/6)`, `(k·π/3)`, `(k·π/2)` pairs, energy-normalized) →
  4-band spectral flux from `fft.magnitudes` over the same mono tail (bands
  split at 200/800/3200 Hz; flux = half-wave-rectified magnitude delta vs
  `flux_bands_prev`) → stereo width & phase coherence from the hop's L/R
  (side/mid energy ratio; normalized cross-correlation at lag 0) → spectral
  centroid (Hz, log-mapped to 0..1 over 35..18000) → assemble+L2-normalize
  `z ∈ R²⁵` → push into `features`/`hops` rings → recurrence search:
  cosine affinity vs all ring entries older than `BLOOM_EXCLUSION_HOPS`,
  collect top-6 ≥ `BLOOM_SIMILARITY_FLOOR` into `ghost_indices`; novelty =
  `1 − best_similarity` (1.0 when the ring is younger than the exclusion).
- `impl Computer { pub(crate) fn recurrence_bloom(&mut self, samples: &[f32], width: f32, height: f32) }`
  — the dispatch target: lazily create `BloomState` (window/hop scaled by
  `distance_scale_feed`), fold interleaved stereo into the mono ring, advance
  `samples_consumed`, run `analyze_hop` every `hop` frames, then emit:
  1. **the envelope**: `r(θ,t) = r0 + Σ_k [a0 + a1·c_k]·cos(h_k·θ + φ_k + ω_k·t)`
     with `r0 = 0.30`, `a0 = 0.015`, `a1 = 0.22·bloom_amount`, `φ_k = k·GOLDEN_ANGLE`,
     `t = samples_consumed / rate`; projected
     `x = r·cos(θ+warp)·(1 + 0.35·width_feature)`,
     `y = r·sin(θ+warp)·(1 + 0.25·coherence)` — θ over `BLOOM_THETA_SAMPLES`,
     closed (first point repeated last), scaled/centered like tunnel
     (spectrum.rs:81-83: `center ± value·min(width,height)`); segment intensity
     `0.30 + 0.55·chroma_total + flash_level·flash_gain`, `age_weight64` graded;
  2. **the carrier**: last ≤`BLOOM_CARRIER_MAX_POINTS` sample pairs as
     `x_c=(L−R)·SQRT_HALF`, `y_c=(L+R)·SQRT_HALF` (the xy45 rotation, xy.rs is
     the reference), intensity ×`bloom_carrier`, drawn over the envelope;
  3. **the ghosts**: for the top `bloom_ghosts` entries of `ghost_indices`,
     re-evaluate the envelope from the stored `HopFrame` at `BLOOM_GHOST_THETA`
     samples, rotated `+0.06·age_s`, radius ×`0.97^age_s`, intensity
     `similarity · 0.35 · exp(−age_s/(memory_seconds/3))` — skip below 0.02;
  4. **silence law**: when `chroma_total < 0.01` and the carrier RMS < 1e-4,
     emit nothing (the chrome resting-dot owns silence).
- Knob plumbing note: `bloom_amount`, `bloom_carrier`, `bloom_ghosts`,
  `memory_seconds`, `flash_gain` are `Computer` pub fields in Phase 3; Phase 1
  hard-codes the defaults (0.7 / 0.6 / 3 / 16.0 / restrained-0.25) as consts so
  the engine ships self-contained.
- Novelty flash: on `novelty > 0.65` and `t − last_flash_t ≥ 0.333`,
  set `flash_level = novelty`, `last_flash_t = t`; every emit,
  `flash_level *= exp(−hop_dt/0.35)` — beam-current boost only. ⛔ never scale
  the whole frame's clear color or full-field anything.

▸ 1.5 📁 `crates/phosphor-dsp/src/lib.rs` dispatch arm (in the outer match, a
sibling of `Mode::XySwirl`): `Mode::RecurrenceBloom => self.recurrence_bloom(samples, width, height),`

▸ 1.6 Mode-switch hygiene: at the top of `dispatch`, before the match:
`if self.mode != Mode::RecurrenceBloom { self.bloom = None; }` — one line,
stale ghosts cannot survive a mode round-trip. (reset() already drops it, ▸1.2.)

▸ 1.7 Tests, same file (`#[cfg(test)] mod tests` in bloom.rs) — the spec's
validation list as code:
- `bloom_is_deterministic`: seeded synth (three-chord progression, 8 s, sine
  stacks at exact pitch freqs, f32) → two fresh Computers → `compute` chunked
  at 800 frames → byte-compare every segment slice. (validation #7)
- `mono_collapses_symmetric`: L=R noise-free chord → carrier |x| < 1e-3 of
  width; envelope x-symmetry within 1 %. (validation #1)
- `polarity_flip_mirrors_carrier`: R→−R flips carrier axis. (validation #2)
- `static_pair_is_stable`: one sine pair, 4 s in → envelope point set at t and
  t+1 s differ only by the precession rotation (compare radii sorted). (#3)
- `chord_change_moves_petals_not_scenes`: C-major→A-minor → chroma_smooth
  moves, segment COUNT stays within ±20 % (no scene cut). (validation #4)
- `repeats_bring_ghosts`: A(4 s) B(4 s) A(4 s) pattern → during the second A,
  `ghost_indices` non-empty and best similarity > 0.85. (validation #5)
- `silence_emits_nothing`: 2 s of zeros after warmup → empty slice. (#6-shape)
- `goertzel_matches_reference`: bank magnitude of a pure 440 Hz sine at the A4
  bin ≈ 1.0 (±2 %), neighbors < 0.1 — pins the normalization forever.
✅ `cargo test -p phosphor-dsp bloom` → all above green.
✅ `cargo clippy --workspace --all-targets -- -D warnings` → silent.
✅ Agent-reachable receipt: `cargo test -p phosphor-dsp` green, then
`printf '{"verb":"mode","args":{"name":"recurrence_bloom"}}' | ...` — covered
instead by the FromStr unit path: `"recurrence_bloom".parse::<Mode>().is_ok()`
(the ctl walk is Phase 6's rig; no GUI needed here).
↩ `git revert <phase-1 commit>`.

## Phase 2 — perf guard + memory hygiene (small, separate commit)

▸ 2.1 `bloom_hop_under_budget` test: 32 s of ring at capacity, one
`analyze_hop` timed over 100 reps → assert mean < 5 ms (CI-generous; spec
target 1.5 ms — print the mean so the receipt carries the real figure).
▸ 2.2 `rings_never_reallocate` test: capture `features.capacity()` after new(),
run 3000 hops, assert capacity unchanged (the preallocation law).
✅ `cargo test -p phosphor-dsp bloom_ -- --nocapture` → timings printed, green.
↩ revert the commit.

## Phase 3 — settings, knob flow, probe (agent-complete before UI exists)

One commit: `feat(scope): bloom knobs persist + probe carries the garden's state`.

▸ 3.1 📁 `crates/phosphor-proto/src/settings.rs` — five fields on `Settings`
(struct + `Default` + `SETTINGS_KEYS` + `take!` rows, clamped on decode like
`beam_cycle_seconds`): `bloom_memory_seconds: f64` (4..=32, default 16.0) ·
`bloom_amount: f32` (0..=1, 0.7) · `bloom_carrier: f32` (0..=1, 0.6) ·
`bloom_ghosts: i64` (0..=6, 3) · `bloom_novelty_flash: String`
(off/restrained/theatrical, "restrained", unknown → default).
✅ `cargo test -p phosphor-proto` — extend the existing settings round-trip
test with the five keys (out-of-range in → clamped out).

▸ 3.2 📁 `crates/phosphor-dsp/src/lib.rs` — promote the Phase-1 consts to pub
fields on `Computer` (`bloom_amount`, `bloom_carrier`, `bloom_ghosts`,
`bloom_memory_seconds`, `bloom_flash_gain`), defaults preserved;
`bloom_memory_seconds` maps to the recurrence ring's *active length* (the ring
stays at max capacity; search just bounds age).

▸ 3.3 📁 `crates/phosphor-app/src/render.rs:205-209` — thread the five settings
into the computer beside `computer.gain = settings.gain;`
(`flash_gain`: off→0.0, restrained→0.25, theatrical→0.6).
✅ `cargo test --workspace` green.

▸ 3.4 Probe + schema, SAME task (⚠ the snapshot-vs-schema honesty test):
📁 `crates/phosphor-app/src/shell.rs` (~1601, beside `beam_cycle`): when
`display_mode == "recurrence_bloom"`, a `bloom` object: `{memory_seconds,
amount, carrier, ghosts, novelty_flash, novelty: f64, active_ghosts: int,
chroma: [f64;12]}` (live values read back from the Computer each probe — add a
small `Computer::bloom_probe()` accessor); absent in other modes (the
beam_cycle-null precedent). 📁 `crates/phosphor-app/src/agent.rs` (~614):
mirror it in the schema.
✅ `cargo test -p phosphor-app` — the schema snapshot test green (it FAILS if
either half is forgotten — that's the point).
↩ revert the commit.

## Phase 4 — UI activation (the smallest seam) + the drift-killer test

One commit: `feat(ui): the bloom opens — mode row, garden controls, twelve modes`.

▸ 4.1 📁 `crates/phosphor-app/src/chrome.rs:36` — `DISPLAY_MODES` grows row 12:
`("recurrence_bloom", "Bloom · recurrence"),` (array type → `; 12]`). Combo
(chrome.rs:222) and context menu (chrome.rs:1872) follow automatically —
verified: both iterate this one list.
▸ 4.2 NEW pin test (in chrome.rs tests or app tests): every `DISPLAY_MODES` id
parses via `phosphor_dsp::Mode::from_str` AND every `Mode::ALL` name appears in
`DISPLAY_MODES` exactly once — the §6.3 drift class dies here.
▸ 4.3 Bloom controls section in the settings panel (pattern: the Custom-theme
conditional at chrome.rs:542): visible only when
`settings.display_mode == "recurrence_bloom"` — Memory slider (4-32 s,
real-unit readout "16 s" per the v4 slider law), Bloom / Carrier sliders
(percent), Ghosts (0-6 stepper), Novelty flash combo (Off / Restrained /
Theatrical), and a **Personality** combo (— / Instrument / Night Garden / Deep
Listening) that one-shot-writes the knobs:
Instrument = carrier 1.0 / bloom 0.25 / ghosts 1 / flash off ·
Night Garden = carrier 0.45 / bloom 0.9 / ghosts 4 / memory 24 / restrained ·
Deep Listening = carrier 0.5 / bloom 0.6 / ghosts 6 / memory 32 / flash off.
Every change pushes `UiAction::SaveSettings` (the cycle_fps crash-proof law).
▸ 4.4 📁 `docs/MANUAL.md:67` — "Eleven modes" → "Twelve modes", add the bloom
paragraph (what the petals, ghosts, and flashes mean; novelty flash ≤3/s and
why that isn't the strobe the epilepsy prompt guards).
▸ 4.5 📁 `tests/golden/README.md:60` — note: "recurrence_bloom (v4.7+) is
post-v3 and has no golden fixtures; its regression pins are the deterministic
unit tests in phosphor-dsp/src/modes/bloom.rs."
✅ `cargo test --workspace` green · clippy silent.
✅ UI receipt at 2560×1440 (standing law), isolated rig: combo lists 12 modes
fully expanded (screenshot), pick Bloom → controls section appears (screenshot),
Personality → Night Garden → sliders visibly move (screenshot).
↩ revert the commit.

## Phase 5 — Night Garden, the 13th room

One commit: `feat(theme): night garden — the room the bloom lives in`.

▸ 5.1 📁 `crates/phosphor-app/src/theme.rs` — 13th `Palette` after Fable
(array → `; 13]`):

```rust
// ── Night Garden — the bloom's companion room: a garden after
//    midnight. Deep blue-violet loam, moonlight ink, and an accent
//    that FOLLOWS THE BEAM — the garden is lit by whatever the
//    flower glows. Moon-orchid violet when the beam rests. 🌙🌸 ──
Palette {
    id: "night_garden", label: "Night Garden", dark: true,
    plane: rgb(0x0b, 0x0f, 0x17), surface: rgb(0x11, 0x17, 0x22),
    surface_2: rgb(0x16, 0x1e, 0x2c),
    ink: rgb(0xe8, 0xed, 0xf7), ink_2: rgb(0xa8, 0xb4, 0xc9),
    muted: rgb(0x6b, 0x76, 0x8c),
    line: rgba(176, 196, 255, 32), line_strong: rgba(176, 196, 255, 70),
    accent: rgb(0xc4, 0x9a, 0xf0), on_accent: rgb(0x12, 0x0d, 0x1c),
    stone: rgb(0x18, 0x20, 0x30), stone_hi: rgb(0x2a, 0x35, 0x4a),
    stone_lo: rgb(0x0a, 0x0d, 0x14),
    accent_follows_beam: true,
},
```

▸ 5.2 Palette tests (theme.rs:476-520): len 12→13, `PALETTES[12].id ==
"night_garden"`, dark, `accent_follows_beam` asserted true; the existing
range-≤-stonework and unique-ids sweeps cover the rest automatically.
✅ `cargo test -p phosphor-app theme` green.
✅ Live receipt on the rig: `ctl ui night_garden` → screenshot; with bloom mode
and a warm beam preset the margins take the beam's hue (afterglow law).
↩ revert the commit.

## Phase 6 — receipts rig, bench guard, docs bank

One commit: `docs+test(bloom): the garden's receipts`.

▸ 6.1 NEW 📁 `tests/receipts/bloom-probe.sh` — self-contained isolated rig
(w1-geometry.sh is the template; ⛔ never the default ctl.sock): Xvfb at
2560×1440, own XDG_RUNTIME_DIR (short path, SUN_LEN), PIPEWIRE_RUNTIME_DIR
handed back, `--background`; play a generated three-chord WAV (chord A 4 s /
B 4 s / A 4 s — LONGER than the test), then: `ctl mode recurrence_bloom` →
probe walk: chroma follows the chord (dominant bins change A→B), novelty
spikes at the A→B boundary, `active_ghosts ≥ 1` during the second A;
`ctl snapshot` twice 2 s apart → pixel-diff confirms motion; mode round-trip
xy→bloom→xy → probe bloom field present→absent→present-and-ghost-free.
✅ `bash tests/receipts/bloom-probe.sh` → PASS lines, exit 0, twice.
▸ 6.2 Bench guard: re-run the Phase-0 bench command under comparable load —
existing four numbers within the environmental-law band (bloom is additive;
regression here means a dispatch mistake).
▸ 6.3 Determinism receipt across the export path: `phosphor render` a 4 s clip
of the seeded WAV in bloom mode twice → byte-identical frames
(`sha256sum` the outputs). (validation #7 end-to-end)
▸ 6.4 Docs bank: HANDOFF.md entry (what shipped, receipts); docs/AGENTS.md —
`recurrence_bloom` in the modes note + the probe `bloom` field;
docs/dev/FUTURE.md — Tempo Lock (tempogram design), Windows 2001 / Broadcast
Ghost personalities (need scene-energy / scanlines), offline full-track memory
(the spec's optional cache). ⚠ validation #6 (wall-clock decay at every FPS
cap) is receipted in the DECAY-DT-SPEC repair, not here — say so in HANDOFF.
▸ 6.5 Skill touch: `~/.claude/skills/phosphor/SKILL.md` + jcode mirror — one
line each: the 12th mode id + the probe `bloom` field.
↩ revert the commit (receipts + docs only).

---

## 📁 Critical files

| File | Role | Phase |
|---|---|---|
| `crates/phosphor-dsp/src/lib.rs` | Mode enum/ALL/name, constants, Computer field, dispatch arm, knob fields | 1, 3 |
| `crates/phosphor-dsp/src/modes/bloom.rs` | NEW — the whole engine + its tests | 1, 2 |
| `crates/phosphor-dsp/src/modes/mod.rs` | one-line module registration | 1 |
| `crates/phosphor-proto/src/settings.rs` | five bloom keys, clamps, round-trip test | 3 |
| `crates/phosphor-app/src/render.rs:205` | settings → Computer knob flow | 3 |
| `crates/phosphor-app/src/shell.rs:~1601` | probe `bloom` object | 3 |
| `crates/phosphor-app/src/agent.rs:~614` | schema mirror (honesty test guards) | 3 |
| `crates/phosphor-app/src/chrome.rs:36,222,542,1872` | DISPLAY_MODES row, controls section, pin test | 4 |
| `crates/phosphor-app/src/theme.rs:56,233,476` | Night Garden palette + tests | 5 |
| `docs/MANUAL.md:67`, `tests/golden/README.md:60` | twelve modes, no-goldens note | 4 |
| `tests/receipts/bloom-probe.sh` | NEW — the isolated live rig | 6 |

## Verification summary

Every phase: `cargo test --workspace` (20 suites + bloom's new ones) and
`cargo clippy --workspace --all-targets -- -D warnings`, then the phase's own
✅ receipts. The spec's validation list lands as: #1-#5, #7 → Phase 1 unit
tests; #7 end-to-end → Phase 6 render determinism; #8 → Phase 3 probe+schema
(honesty test); #6 → **deferred to the DECAY-DT-SPEC repair by design** — bloom's
own ghost fading is second-based from birth, but frame-decay FPS-honesty is that
repair's exit criterion, not this feature's. Recommended sequencing:
**DECAY-DT-SPEC implementation first, Bloom second** — the garden should open on
honest phosphor.

No release, no tag: Ben decides when the garden opens. Release-worthiness note
for that day: this is a minor-version feature (4.7.0-shaped), demo-hungry —
`phosphor render` a bloom clip for the README the way cycle-demo.gif was born.
