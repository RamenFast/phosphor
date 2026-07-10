# Layout-Aware Downmix & Mix-Alignment Design (audit §4.5–§4.7)

Status: design only, no code. Anchors verified at HEAD fcd3a12.
Scope: phosphor-audio capture/mix/playback paths. Target format everywhere is the
scope's stereo (FL, FR) @ f32.

---

## 1. Problem recap (from audit + verification)

- **§4.5 Mix without timestamps** — `fold_mix_into_ring`
  (`crates/phosphor-audio/src/engine.rs:403-426`) sums mix members by raw vector
  index: zero-pads to the longest member (engine.rs:411-413), unity-gain
  `*slot += *sample` (engine.rs:414-416), no per-source gain, no limiter, no
  graph-time alignment. Skew bounded only by drain period (doc comment
  engine.rs:398-402) plus per-member overflow drops (engine.rs:1336-1340).
- **§4.6 First-two-channels downmix** — `playback.rs:285-303`: >2ch content
  takes `frame[0]`/`frame[1]` only. Center, surrounds, LFE discarded.
  Symphonia's channel map is used only for `count()` (playback.rs:283).
- **§4.7 Missing channel positions on capture** — capture format pod
  (engine.rs:1351-1360) sets F32LE/rate/channels=2 but never `set_position`;
  playback does (engine.rs:1473-1480). No `param_changed` listener on capture
  streams (engine.rs:1290-1345), so the negotiated layout is never observed.

---

## 2. Channel-position handling on capture streams

### 2.1 Request positions (fix §4.7, cheap)

In the capture `AudioInfoRaw` construction (**engine.rs:1351-1360**), add:

```text
info.set_position([SPA_AUDIO_CHANNEL_FL, SPA_AUDIO_CHANNEL_FR, ...zeros])
```

mirroring the playback pod at **engine.rs:1473-1480**. This makes the request
explicit: PipeWire's channelmix converter then owns the N→2 fold for capture,
using its own layout-aware matrix. This is the *primary* fix for capture:
prefer delegating downmix to PipeWire rather than reimplementing it on the
capture side. Phosphor's own matrix (§3) is then needed only for the file
playback path (§4.6) and as a capture fallback if we ever negotiate >2ch.

### 2.2 Receive/observe the negotiated layout

Add a `param_changed` listener to the capture stream listener chain
(**engine.rs:1290-1345**, alongside `state_changed`/`process`):

- Match `ParamType::Format`, parse the pod back into `AudioInfoRaw`
  (libspa `format_utils` or manual pod parse), and record
  `{channels, rate, position[]}` into the per-target capture state
  (near `MemberBuffer`, engine.rs:1327-1340).
- Surface it in `phosphor-ctl probe` output for debuggability.
- If positions come back as `UNKNOWN`/`AUX*` or the count ≠ requested,
  log once and apply the fallback policy (§3.4).

---

## 3. Downmix matrix policy (fix §4.6)

Applies in the file player (`playback.rs:285-303`) where Symphonia gives us a
real channel map, and generically anywhere phosphor receives >2ch f32 frames.

### 3.1 Gain convention

Use the ITU-R BS.775 coefficients, then **normalize the matrix rows so the
max possible row sum ≤ 1.0**. Note this is **not** ffmpeg's default `-ac 2`
behavior: as measured in Appendix A.2 (ffmpeg 6.1.1), libswresample's default
downmix applies the raw ITU taps with **no normalization** (FL contributes at
1.0, center/surrounds at −3 dB = 0.7071, LFE dropped), so its 5.1 row sum is
1 + 2c ≈ 2.414 and can exceed full scale. Whether phosphor keeps this
row-sum normalization (and patches the ffmpeg decode paths to match, option
(a)) or drops it to match ffmpeg's unnormalized default (option (b)) is an
**open design decision** — see Appendix A.4. As written, this section
describes option (a), "unity-preserving after normalization":

- Raw coefficients: center and surrounds at −3 dB (`c = 1/√2 ≈ 0.7071`),
  LFE at −∞ (dropped) by default (configurable to −3 dB or −6 dB later).
- These −3 dB taps are the standard *amplitude-preserving-per-source /
  energy-preserving-for-uncorrelated* compromise. Full energy preservation
  per-layout is not attempted; the row normalization below is what actually
  prevents clipping.
- Normalization: `k = 1 / max(sum(row_L), sum(row_R))`, multiply the whole
  matrix by `k`. Deterministic, layout-dependent, computed once at stream open.

### 3.2 Matrices (pre-normalization, c = 0.7071)

| Layout | L' | R' |
|---|---|---|
| **mono** (FC or single) | 1.0·M | 1.0·M |
| **stereo** (FL FR) | FL | FR |
| **2.1** (FL FR LFE) | FL (+0·LFE) | FR (+0·LFE) |
| **5.1** (FL FR FC LFE SL SR) | FL + c·FC + c·SL | FR + c·FC + c·SR |
| **7.1** (5.1 + RL RR) | FL + c·FC + c·SL + c·RL | FR + c·FC + c·SR + c·RR |
| quad (FL FR RL RR) | FL + c·RL | FR + c·RR |
| 3.0 (FL FR FC) | FL + c·FC | FR + c·FC |

After normalization, e.g. 5.1: row sum = 1 + 2c ≈ 2.414 → k ≈ 0.414, so
FL contributes ≈ 0.414 and each −3 dB tap ≈ 0.293. Output cannot exceed ±1.0
for full-scale correlated input.

Note mono currently duplicates at unity (playback.rs:290-295), which
matches row 1 of this design's table, but the ffmpeg offline paths use
0.7071 per side (A.2), so the player and render/bench disagree by 3 dB on
mono files (see A.6). Whether to keep 1.0 or adopt 0.7071 is an **open
question** tied to the A.4 (a)/(b) decision; do not treat this convention
as settled.

### 3.3 Layout detection order (player path)

1. Symphonia `spec.channels` bitmask → map SPA/ffmpeg-style positions.
2. If the mask is empty/degenerate, infer from count: 1→mono, 2→stereo,
   3→2.1 or 3.0 (prefer FL FR LFE if mask says so, else 3.0), 4→quad,
   6→5.1, 8→7.1.
3. Otherwise fallback (§3.4).

### 3.4 Fallback when positions are unknown

- count == 1: duplicate to both channels.
- count == 2: assume FL FR (this is also today's implicit capture assumption;
  after §2.1 it becomes explicit).
- count > 2, unknown layout: **keep today's behavior — take ch0/ch1** as FL/FR
  (they are FL/FR in every common convention), but scale by
  `1/√(count/2)` capped at 1.0? No — do NOT scale: ch0/ch1 are already
  discrete channels, so passthrough at unity is correct. Just log the unknown
  layout once. This keeps the fallback identical in level to current behavior.

### 3.5 Where it slots in (file:line anchors)

- **`playback.rs:283`** — where `channels = spec.channels.count()` is taken:
  build a `DownmixMatrix::from_symphonia(spec.channels, channels)` here,
  once per decoded stream.
- **`playback.rs:285-303`** — replace the `frame[0]/frame[1]` fold with
  `matrix.fold(frame) -> (l, r)`. Runs on the player thread (spawned at
  engine.rs:262), off the audio graph thread, so the per-frame dot product
  (≤8 mul-adds per output channel) is trivially cheap.
- New module suggestion: `crates/phosphor-audio/src/downmix.rs` holding
  `DownmixMatrix { l: [f32; 8], r: [f32; 8], n: usize }` + layout tables +
  unit tests (full-scale correlated input never exceeds ±1.0; stereo/mono
  identity; 5.1 coefficient goldens vs ffmpeg values).
- Bench/render/export paths (`bench.rs`, `render.rs`, `exports.rs`) decode
  via ffmpeg's default `-ac 2` (unnormalized, see Appendix A) — their fate
  depends on the option (a)/(b) decision in Appendix A.4.
- Capture path (**engine.rs:1327-1340**): no matrix needed while we negotiate
  2ch with explicit FL/FR (§2.1); PipeWire's channelmix does the fold.

---

## 4. Mix policy fixes (§4.5)

### 4.1 Gain and clipping

In `fold_mix_into_ring` (**engine.rs:403-426**):

- Per-source gain: default `1/N` is too quiet for typical uncorrelated app
  audio; use `1/√N` (energy-preserving for uncorrelated sources) as default,
  with an optional per-member gain knob later.
- Add a cheap soft clipper on the summed output (`tanh`-ish or hard clamp to
  ±1.0). Since the fold runs lazily on the shell thread
  (engine.rs:381-384), CPU cost is a non-issue.

### 4.2 Timestamp-aligned mixing: feasibility verdict

**Feasible, but only via raw FFI, and worth doing as a follow-up rather than
in this change.** With pinned `pipewire 0.9.2` (per va-pw-crate-caps):

- The safe wrapper lacks `stream.time()` (`stream/mod.rs:277` is a TODO), but
  `pw_sys::pw_stream_get_time_n` (bindings.rs:3852) and
  `pw_stream_get_nsec` are callable via `Stream::as_raw_ptr()`.
- Better: register `io_changed` (safe API, `stream/mod.rs:533`), match
  `id == SPA_IO_Position (7)`, keep the `*mut spa_io_position`
  (libspa-sys bindings.rs:2006), and read `clock.position` (per-cycle sample
  counter) + `clock.nsec` inside each capture `process` callback. Since all
  capture streams share the PipeWire graph clock, `clock.position` is a
  common timeline: tag each `MemberBuffer` write with the graph position and
  align members by position in the fold, instead of by vector index.
- `pw_buffer.time` (bindings.rs:3522) also carries a queue timestamp, but it
  requires system PipeWire ≥1.0.5 at build time (bindgen from system headers),
  so it is a portability risk. Prefer `spa_io_position`.
- Design shape: `MemberBuffer` becomes `{start_position: u64, samples: Vec}`;
  fold aligns members to `max(start_position)` and trims leaders, using
  `clock.position` gaps to detect drops/xruns (also `clock.xrun` accumulator).

### 4.3 Non-goals for now

- No resampling / rate_diff correction between members (they share one graph
  clock, so drift is not expected; `rate_diff` is clock-vs-monotonic, not
  member-vs-member).
- No RT_PROCESS change on capture (engine.rs:1373-1377); alignment metadata
  read in the existing engine-thread callback is fine.

---

## 5. Risks

1. **Pod round-trip on `param_changed`**: libspa 0.9.2's pod parsing helpers
   for Format params are thin; may need manual pod walking. Contained risk,
   read-only.
2. **`unsafe` FFI for `spa_io_position`**: dangling-pointer hazard — the io
   area pointer is only valid while the stream lives and can be revoked by a
   later `io_changed(ptr=null)`. Must handle the null case.
3. **Bindgen/system-header coupling**: `pw_buffer.time` and `pw_time.size`
   exist only when build-machine PipeWire ≥1.0.5. Any design relying on them
   breaks on older distros. Mitigation: use `spa_io_position` (stable since
   0.3.x) and gate anything newer.
4. **Behavioral change in loudness**: switching mix gain to `1/√N` and adding
   5.1 downmix changes scope brightness/levels vs current builds. Should be
   noted in release notes; scope auto-gain (if any) may mask or amplify this.
5. **PipeWire channelmix trust**: §2.1 assumes PipeWire's converter does a
   sane layout-aware fold when we declare FL/FR. Believed true (it is the
   standard path for 2ch sinks), but should be verified live with a 5.1
   source before shipping.
6. **Fallback ambiguity at 3ch/4ch/6ch without masks**: heuristics can guess
   wrong (e.g. 6ch hexagonal vs 5.1). Impact is mild (wrong fold, no crash).
7. **Alignment fold complexity**: position-tagged `MemberBuffer` changes the
   producer/consumer contract between the engine thread and the shell-thread
   fold; needs careful locking review (current cap/drop logic at
   engine.rs:1336-1340 must interact correctly with trimming).

---

## Appendix A. Audit of ffmpeg decode paths (va-gap-ffmpeg-decode-paths, 2026-07-10)

Static read of the three ffmpeg decode invocations, plus an empirical probe of
ffmpeg 6.1.1's default `-ac 2` behavior.

### A.1 The decode paths

All three pass raw `-ac 2` with no `pan=`/`aresample` filter, so all inherit
libswresample's default rematrix:

- `crates/phosphor-app/src/bench.rs:142-148` — `decode_signal`: `-f f32le -ac 2 -ar <rate>`.
- `crates/phosphor-app/src/render.rs:164-172` — `spawn_decoder` (render command): `-f f32le -ac 2 -ar <rate>`. (`spawn_encoder` at render.rs:176-190 re-muxes the *original* audio via `-map 1:a` + aac; any >2ch source keeps its channel count in the exported video's audio track, no downmix there.)
- `crates/phosphor-app/src/exports.rs:211-215` — `export_postcard`: `-f s16le -ac 2 -ar 48000`.
- `crates/phosphor-app/src/exports.rs:159-171` — `save_clip` encoder only; input wav is already stereo history, no downmix concern.

### A.2 Measured ffmpeg 6.1.1 default `-ac 2` rematrix (aevalsrc solo-channel probe, f32le readback)

Method: for each layout, generate `aevalsrc` with a 1 kHz sine soloed on one
channel (all others zero), decode with plain `-ac 2 -f f32le`, and read back
the peak of the L and R interleaved streams. Measured 2026-07-10 on ffmpeg
6.1.1-3ubuntu5. All six layouts requested by va-gap-ffmpeg-matrix-more-layouts:

| Layout | Solo channel | L out | R out |
|---|---|---|---|
| mono | FC | 0.7071 | 0.7071 |
| quad | FL / FR | 1.0 / 0 | 0 / 1.0 |
| quad | BL / BR | 0.7071 / 0 | 0 / 0.7071 |
| 5.1(side) | FL / FR | 1.0 / 0 | 0 / 1.0 |
| 5.1(side) | FC | 0.7071 | 0.7071 |
| 5.1(side) | LFE | 0.0 | 0.0 |
| 5.1(side) | SL / SR | 0.7071 / 0 | 0 / 0.7071 |
| 5.1 (back) | FL/FR/FC/LFE | identical to 5.1(side) | — |
| 5.1 (back) | BL / BR | 0.7071 / 0 | 0 / 0.7071 |
| 6.1 | FL/FR/FC/LFE | identical to 5.1 | — |
| 6.1 | **BC** | **0.5** | **0.5** |
| 6.1 | SL / SR | 0.7071 / 0 | 0 / 0.7071 |
| 7.1 | FL/FR/FC/LFE | identical to 5.1 | — |
| 7.1 | SL / SR | 0.7071 / 0 | 0 / 0.7071 |
| 7.1 | BL / BR | 0.7071 / 0 | 0 / 0.7071 |

Observations:

- Taps are **layout-invariant**: FL/FR stay 1.0, FC and every side/back
  surround stay 0.7071, LFE stays 0, regardless of channel count. In 7.1
  the SL/SR **and** BL/BR taps are all 0.7071 simultaneously (max row sum
  1 + 3·0.7071 ≈ 3.12), confirming **no normalization kicks in for wider
  layouts** — libswresample's default never renormalizes here (for
  **float** output; integer output like s16le *is* normalized, see A.5).
- 6.1's back-center is the one composed coefficient: BC → 0.5 per side
  (0.7071 into BL/BR, then 0.7071 into L/R).
- mono → stereo is 0.7071 into each output, not 1.0 — a mono file decoded
  via these ffmpeg paths comes back 3 dB down per channel.
- **Polarity: all taps are positive.** The table above was measured by peak
  magnitude only (sine probe), which is sign-blind. Re-probed 2026-07-10
  (va-gap-tap-polarity-probe) with a **DC offset** (`aevalsrc=0.5` soloed per
  channel, others zero, `-ac 2 -f f32le`, mean of each output stream): for
  5.1 (FL FR FC LFE BL BR) and 7.1 (FL FR FC LFE BL BR SL SR) every non-zero
  output mean was positive (+0.5 fronts, +0.3536 ≈ 0.5·0.7071 for
  FC/BL/BR/SL/SR, 0 for LFE). No coefficient in libswresample's default
  stereo downmix is phase-inverted, so the matrix table stands as written.

So ffmpeg's default matches this design's raw (un-normalized) taps exactly
across all measured layouts: ITU −3 dB (0.7071) for center/surrounds, LFE
dropped (`lfe_mix_level` default 0), fronts at unity.

### A.3 Divergence: normalization claim in §3.1 is wrong

§3.1 originally asserted that row-sum normalization (`max row sum ≤ 1.0`) is
"ffmpeg's default `-ac 2` behavior". It is not (§3.1 has since been amended,
2026-07-10, to state the measured default and mark (a)/(b) as open). libswresample's default (`rematrix_maxval`
= INT_MAX effectively, no renorm applied here) leaves FL at 1.0 with row sum
1 + 2c ≈ 2.414. Consequences:

1. **Loudness mismatch**: the proposed DownmixMatrix (k ≈ 0.414 for 5.1) will
   be ≈ 7.7 dB quieter than the ffmpeg decode paths for the same 5.1 source.
   The player (Symphonia path) and the render/bench (ffmpeg f32le paths)
   would disagree audibly and in beam amplitude. (Postcard's s16le path is
   normalized, see A.5, so it matches the proposed matrix, not the f32le
   paths.)
2. **Clipping risk in ffmpeg paths**: ~~`export_postcard` decodes to **s16le**,
   which hard-clips; a hot correlated 5.1 source can exceed full scale after
   ffmpeg's unnormalized downmix.~~ **Refuted empirically — see A.5.** The
   s16le path is normalized by swresample and does not clip. The f32le paths
   (bench, render decode) do see >±1.0 samples (measured 2.39 peak, A.5).

### A.4 Recommended amendment

Pick one and make it consistent everywhere:

- **(a)** Keep row-sum normalization in DownmixMatrix (safe, no clip) and add
  an explicit `pan=`/`aresample` matrix or `-af "pan=stereo|..."` (or
  `aresample` with normalized matrix) to the three ffmpeg invocations so they
  match; or
- **(b)** Drop the normalization and match ffmpeg's default (FL 1.0, taps
  0.7071, LFE 0), accepting potential >FS peaks and adding a limiter/clamp.

§3.1's wording has been fixed (2026-07-10): it now states that ffmpeg's
default is *not* row-sum normalized and references this appendix for the
open (a)/(b) decision.

### A.5 Empirical clip probe of the export_postcard command shape (va-gap-postcard-clip-repro, 2026-07-10)

Reproduction attempt of the A.3.2 clip claim, on ffmpeg 6.1.1-3ubuntu5
(same host that runs the app). Hot legal 5.1 source: correlated full-scale
FL+FC+SL, `aevalsrc=0.99·sin(440)` on channels FL/FC/SL, zero elsewhere,
pcm_f32le 5.1 wav @48k. Ran the exact exports.rs:211-215 shape:
`ffmpeg -v error -i src -f s16le -ac 2 -ar 48000 -`.

**Result: refuted for the s16le path.** Output L peak = ±32440 (0.990 FS),
zero samples at i16 bounds, no clip. Solo-channel probes through the same
s16le shape measure FL→L **0.4142**, FC→L/R **0.2929**, SL→L **0.2929**,
i.e. row-sum-normalized ITU coefficients (0.4142+0.2929+0.2929 ≈ 1.000).
The unnormalized 0.7071 taps in A.2 were measured with **f32le** output;
re-running the hot file with `-f f32le -ac 2` gives L peak **2.390**
(= 0.99·(1+2·0.7071)), confirming the split: **libswresample normalizes the
rematrix when the output sample format is integer (s16), and leaves it
unnormalized for float output.** So:

- `export_postcard` (s16le, exports.rs:211-215): normalized, cannot clip
  from downmix. A.3.2's clip claim is wrong for this path.
- `bench.rs` / `render.rs` (f32le): unnormalized, >±1.0 samples confirmed
  (2.39 peak measured), as A.3.2 said.

Clamp-vs-wrap: forcing >FS content into the s16 conversion (a 1.5·FS f32
stereo wav → `-f s16le`) yields a saturated plateau at ±32767/−32768 with
no sign-flip wrap artifacts. **swresample clamps (saturates), never wraps.**

**Mechanism (grounded in FFmpeg source, va-gap-swr-int-float-mechanism,
2026-07-10):** the split is intentional, sample-format-keyed clipping
protection in libswresample's auto-matrix builder. In
`libswresample/rematrix.c`, `auto_matrix()` picks the normalization
ceiling `maxval`: if `rematrix_maxval` is unset (>0 not given), then
`maxval = 1.0` when either the **output sample format or the internal
processing format is integer** (`av_get_packed_sample_fmt(...) <
AV_SAMPLE_FMT_FLT`), else `maxval = INT_MAX` (i.e. effectively no cap)
for float. `swr_build_matrix2()` then computes `maxcoef = max row sum of
|coeffs|` and divides the whole matrix by `maxcoef/maxval` only when
`maxcoef > maxval`. So for s16 output the 5.1→stereo row sum 1+2·0.7071 ≈
2.414 exceeds 1.0 and the matrix is scaled by 1/2.414 (FL 1.0→0.4142, taps
0.7071→0.2929, exactly the measured values); for f32 output 2.414 < INT_MAX
and nothing is scaled. Rationale: integer paths saturate at full scale, so
the renorm is clip protection; float can carry >±1.0 losslessly, so
headroom is preserved. Confirmed on this host via `ffmpeg -v debug`: the
`Matrix coefficients` dump shows FL: 0.414214/FC: 0.292893 for `-f s16le
-ac 2` and FL: 1.000000/FC: 0.707107 for `-f f32le -ac 2` on the same 5.1
input. `-ac 2` inserts an `auto_aresample` filter either way; the only
per-format difference is swr's own `maxval` choice, not CLI plumbing.

**Version stability:** this exact `maxval` selection (with the same
integer-vs-float condition) has been in `rematrix.c` since at least
FFmpeg n1.0 (2012, then as an inline `maxcoef > 1.0` check gated on the
same `< AV_SAMPLE_FMT_FLT` test; refactored to the `maxval` form by
n2.8) and is unchanged through n8.0 (checked tags n1.0, n1.2, n2.0,
n2.8, n3.0, n6.1.1, n8.0). It is stable, documented-by-code behavior,
not version-fragile. It can be overridden explicitly with
`-rematrix_maxval 1.0` (forces normalization even for float output),
which is the low-risk lever if option (a) of A.4 is chosen.

### A.6 Downstream fate of >±1.0 f32 samples (va-gap-f32-overrange-downstream, 2026-07-10)

Traced where the two f32le decode paths (bench.rs `decode_signal`, render.rs
`spawn_decoder`) deliver samples, and what each consumer assumes about range.
Both feed `Computer::compute` (`crates/phosphor-dsp/src/lib.rs:315-334`) —
bench via `execute()` (bench.rs:199-203), render via the frame loop
(render.rs:449-452). No stage between the ffmpeg pipe read and `compute`
normalizes, clamps, or checks range.

Per-mode behavior with >FS input (code read):

- **xy / xy45 / xy_dots / swirl** (`modes/xy.rs:60-70`): position is
  `center ± sample * gain * radius` — a 2.39 sample maps ~2.39× outside the
  nominal scope circle, i.e. **off-screen** (measured x range
  [−234, 1314] on a 1080-wide frame; see probe below). No NaN, no panic:
  segment intensity is `min(1.0)`-capped (xy.rs:99), and the CPU
  rasterizer culls/clips quads to the buffer
  (`phosphor-render-cpu/src/raster.rs:39-53`) while the compositor clamps
  each channel to [0,1] (`phosphor-render-cpu/src/lib.rs:227,233`); the GPU
  shader clamps alpha too (`shaders.wgsl:208`). So the failure mode is
  purely **visual**: the trace slams past the graticule and mostly leaves
  the frame (looks like extreme over-gain), not blowout/NaN.
- **waveform / ring / helix / tunnel** (`modes/waveform.rs:38-56` etc.):
  amplitude scales `height * 0.21 * gain * sample`; a 2.39 sample overshoots
  its lane and can cross the other channel's trace. Benign otherwise.
  Trigger search (waveform.rs:18-30) only compares signs, unaffected.
- **spectrum / spectrum_radial** (`modes/spectrum.rs:29-34`): level is
  `((peak/norm).sqrt() * gain).min(1.0)` — hot input just pins bars at 1.0.
- **kit chain** (`lib.rs:319-327`): stages process the raw over-range buffer;
  no stage asserts [-1,1] (not re-audited per-op — see caveat).

Empirical probes:

1. Unit probe: fed one 60 fps chunk of `2.39·sin(440 Hz)` into
   `Computer::compute` (mode xy, defaults). Result: 799 segments, **0
   non-finite values**, x range [−234.4, 1314.4] px on a 1080×720 frame,
   max intensity 0.988. Confirms off-screen excursion, no NaN.
2. End-to-end: `phosphor render` on the A.5 hot 5.1 wav (0.99 FL+FC+SL)
   completed normally (120 frames, cpu renderer), dump-frame output is a
   valid PPM with max channel 255 and no corruption.

Verdict: over-range f32 input is **numerically benign but visually wrong**
(off-screen beam / lane overshoot). There is no [-1,1] invariant enforced or
relied on for safety anywhere in the segment→raster→composite chain; the
only real defect is amplitude semantics.

Mono parity divergence (same root cause, opposite sign): ffmpeg mono→stereo
is 0.7071 per side (A.2) while the Symphonia player duplicates mono at 1.0
(`crates/phosphor-audio/src/playback.rs:289-294`), so mono files render/bench
**3 dB smaller** in beam deflection than they play live. Together with the
5.1 case (render hotter than player) the player and the offline paths
disagree in both directions today.

### A.7 Audio fate of >±1.0 samples in clip exports (va-gap-save-clip-overrange-audio, 2026-07-10)

A.6 covered the *visual* fate of over-range f32; this covers the *audio
export* fate. Chain (code read):

- `fold_mix_into_ring` (`crates/phosphor-audio/src/engine.rs:403-426`) sums
  mix members at **unity gain with no limiter** into the capture ring, so
  the ring can carry >±1.0 samples whenever a multi-app mix is hot (§4.1's
  soft clipper is a proposal, not implemented).
- `save_clip` (`crates/phosphor-app/src/shell.rs:1004,1032` →
  `crates/phosphor-app/src/exports.rs:140`) hands
  `engine.copy_history()` — the raw ring, untouched — to `write_wav`.
- `write_wav` (`exports.rs:71`) does `sample.clamp(-1.0, 1.0) * 32767.0`
  before the s16 cast: a **hard clamp**, then ffmpeg muxes the wav to AAC.

Empirical probe (unit test, run then removed): fed `write_wav`
`[2.0, -3.5, 0.5, 1.0000001]` and read back the PCM — got
`[32767, -32767, 16383, 32767]`. **Saturates, never wraps** (no sign-flip
artifacts), matching swresample's behavior in A.5.

Verdict: clip exports of a hot multi-app mix **hard-clip audibly today** —
flat-topped waveform, harsh odd-harmonic distortion — while the same
over-range samples were numerically benign visually (A.6). This is the
concrete audible consequence motivating the §4.1 recommendation: the
`1/√N` per-source gain plus a soft clipper in `fold_mix_into_ring` would
fix both the live scope feed and every export drawn from the ring, since
`write_wav`'s clamp would then be a no-op safety net rather than the
distortion stage.

Clamp requirement for A.4: **no hard clamp is required for safety under
either option** — nothing NaNs or panics. But:

- Option **(b)** (match ffmpeg unnormalized) accepts routine >FS peaks, so it
  **should** add a soft limiter or clamp before `compute` if the off-screen
  beam excursion is considered unacceptable; otherwise hot 5.1 content
  renders as a blown-out over-gain trace.
- Option **(a)** (normalize everywhere) removes the over-range source
  entirely; a clamp is then optional belt-and-braces only. Option (a) also
  naturally fixes the mono 0.7071-vs-1.0 parity if the ffmpeg invocations
  get an explicit pan matrix including `mono→stereo = 1.0` per side (or the
  player adopts 0.7071 — pick one convention).

Not checked here: individual kit ops' numeric behavior on >FS input
(waveshapers etc. could fold interestingly, still finite-math), GPU renderer
end-to-end with hot input (CPU path verified), and the live PipeWire capture
path (out of scope — PipeWire owns that fold).

### A.7 AC-3 / DTS codec-level downmix probe (va-gap-ac3-decoder-downmix, 2026-07-10)

Concern: lossy decoders (ac3, dca) can apply their own codec-level downmix
(dmix metadata / `request_channel_layout`) *before* swresample, potentially
changing the effective taps vs the pcm-based A.2 matrix. Probe on ffmpeg
6.1.1-3ubuntu5: encoded the A.2 solo-channel 5.1(side) signals and the A.5
hot correlated FL+FC+SL (0.99·sin 440) signal to **AC-3** (`-c:a ac3`) and
**DTS** (`-c:a dca -strict -2`), then ran both exact app decode shapes
(`-f f32le -ac 2 -ar 48000` and `-f s16le -ac 2 -ar 48000`).

Measured (both codecs identical to within lossy-codec error, ≈0.7001 vs
0.7071, i.e. <0.09 dB):

| Solo ch | f32 L / R | s16 L / R |
|---|---|---|
| FL | 0.990 / 0 | 13438 / 0 |
| FC | 0.700 / 0.700 | 9502 / 9502 |
| LFE | 0 / 0 | 0 / 0 |
| SL | 0.700 / 0 | 9502 / 0 |
| HOT FL+FC+SL | **2.390** / 0.700 | 32442 / 9502 (no clip) |

(DTS values within ±3 LSB of AC-3.)

**Conclusion: refuted.** The AC-3 and DTS decode paths behave identically to
the pcm-based A.2/A.5 results: unnormalized ITU taps (FL 1.0, FC/SL/SR
0.7071, LFE 0) for f32le output with the same 2.39 hot peak, and
row-sum-normalized taps for s16le output with no clipping (13438/32767 ≈
0.410 ≈ 0.99·0.4142). No codec-level downmix alters the effective matrix
here — with plain `-ac 2` the downmix still happens in libswresample after
full 5.1 decode. Note the ffmpeg *encoders* wrote no custom dmix metadata;
a broadcast AC-3 file carrying non-default cmixlev/surmixlev could still
differ (not checked).

## Verified SPA <-> Symphonia channel mapping (va-gap-spa-symphonia-mapping, 2026-07-10)

Sources read directly:
- Symphonia: `~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/symphonia-core-0.5.5/src/audio.rs` (bitflags `Channels: u32`, lines ~29-90). First 18 bits match Microsoft WAVEFORMATEXTENSIBLE.
- SPA: bindgen output for libspa-sys 0.9.2 at `target/release/build/libspa-sys-de6dc2e310eb1405/out/bindings.rs` lines 2586-2662. **SPA values are enum position indices (`spa_audio_channel: u32`), NOT bitflags.**

### Symphonia `Channels` bitflags (audio.rs:38-89)

| Flag | Bit value |
|---|---|
| FRONT_LEFT | 0x0000_0001 |
| FRONT_RIGHT | 0x0000_0002 |
| FRONT_CENTRE | 0x0000_0004 |
| LFE1 | 0x0000_0008 |
| REAR_LEFT | 0x0000_0010 |
| REAR_RIGHT | 0x0000_0020 |
| FRONT_LEFT_CENTRE | 0x0000_0040 |
| FRONT_RIGHT_CENTRE | 0x0000_0080 |
| REAR_CENTRE | 0x0000_0100 |
| SIDE_LEFT | 0x0000_0200 |
| SIDE_RIGHT | 0x0000_0400 |
| TOP_CENTRE | 0x0000_0800 |
| TOP_FRONT_LEFT | 0x0000_1000 |
| TOP_FRONT_CENTRE | 0x0000_2000 |
| TOP_FRONT_RIGHT | 0x0000_4000 |
| TOP_REAR_LEFT | 0x0000_8000 |
| TOP_REAR_CENTRE | 0x0001_0000 |
| TOP_REAR_RIGHT | 0x0002_0000 |
| REAR_LEFT_CENTRE | 0x0004_0000 |
| REAR_RIGHT_CENTRE | 0x0008_0000 |
| FRONT_LEFT_WIDE | 0x0010_0000 |
| FRONT_RIGHT_WIDE | 0x0020_0000 |
| FRONT_LEFT_HIGH | 0x0040_0000 |
| FRONT_CENTRE_HIGH | 0x0080_0000 |
| FRONT_RIGHT_HIGH | 0x0100_0000 |
| LFE2 | 0x0200_0000 |

Channel interleave order = ascending bit order (see `ChannelsIter`, audio.rs:~101, iterates by `trailing_zeros`).

**Empirically verified (va-gap-symphonia-interleave-verify, 2026-07-10):**
5.1 files with distinct solo tones per channel (ffmpeg aevalsrc: FL=300, FR=600,
FC=900, LFE=60, RL=1200, RR=1500 Hz) decoded via symphonia 0.5.5
`SampleBuffer::copy_interleaved_ref` (probe:
`crates/phosphor-audio/examples/symphonia_interleave_probe.rs`). For **FLAC**,
**WAV (pcm_s16le)**, and **Vorbis**, `spec.channels` came back as bits 0x3f
(FL|FR|FC|LFE1|RL|RR) and interleave slots 0..5 measured 300/600/900/60/1200/1500 Hz,
i.e. exactly FL,FR,FC,LFE,RL,RR ascending bit order. The Vorbis result is the
strong evidence: Vorbis's *native* 5.1 order is FL,FC,FR,RL,RR,LFE, so
Symphonia's decoder demonstrably remaps to bit order rather than passing
codec-native order through. Not tested: AAC, MP3 (mp3 is ≤2ch anyway), ALAC.

### SPA_AUDIO_CHANNEL_* enum values (bindings.rs:2586-2662)

UNKNOWN=0, NA=1, MONO=2, FL=3, FR=4, FC=5, LFE=6, SL=7, SR=8, FLC=9, FRC=10, RC=11, RL=12, RR=13, TC=14, TFL=15, TFC=16, TFR=17, TRL=18, TRC=19, TRR=20, RLC=21, RRC=22, FLW=23, FRW=24, LFE2=25, FLH=26, FCH=27, FRH=28, TFLC=29, TFRC=30, TSL=31, TSR=32, LLFE=33, RLFE=34, BC=35, BLC=36, BRC=37, START_Aux/AUX0=4096, START_Custom=65536.

### Mapping table (downmix.rs)

| Semantic | Symphonia flag (bit) | SPA constant (value) |
|---|---|---|
| FL | FRONT_LEFT (0x1) | SPA_AUDIO_CHANNEL_FL (3) |
| FR | FRONT_RIGHT (0x2) | SPA_AUDIO_CHANNEL_FR (4) |
| FC | FRONT_CENTRE (0x4) | SPA_AUDIO_CHANNEL_FC (5) |
| LFE | LFE1 (0x8) | SPA_AUDIO_CHANNEL_LFE (6) |
| RL | REAR_LEFT (0x10) | SPA_AUDIO_CHANNEL_RL (12) |
| RR | REAR_RIGHT (0x20) | SPA_AUDIO_CHANNEL_RR (13) |
| SL | SIDE_LEFT (0x200) | SPA_AUDIO_CHANNEL_SL (7) |
| SR | SIDE_RIGHT (0x400) | SPA_AUDIO_CHANNEL_SR (8) |
| FLC | FRONT_LEFT_CENTRE (0x40) | SPA_AUDIO_CHANNEL_FLC (9) |
| FRC | FRONT_RIGHT_CENTRE (0x80) | SPA_AUDIO_CHANNEL_FRC (10) |
| RC | REAR_CENTRE (0x100) | SPA_AUDIO_CHANNEL_RC (11) |
| LFE2 | LFE2 (0x0200_0000) | SPA_AUDIO_CHANNEL_LFE2 (25) |
| Mono | FRONT_LEFT (0x1, per Layout::Mono, audio.rs:147) | SPA_AUDIO_CHANNEL_MONO (2) |

### Gotchas
- SPA is positional (enum indices), Symphonia is a bitmask. Do not treat SPA values as bits.
- Ordering diverges: SPA puts SL/SR (7,8) BEFORE RL/RR (12,13); Symphonia bit order puts REAR (bits 4,5) before SIDE (bits 9,10). A 5.1 stream in Symphonia interleave order is FL,FR,FC,LFE,RL,RR — do not assume PipeWire's SL/SR-first convention.
- SPA distinguishes MONO (2) from FL; Symphonia represents mono as FRONT_LEFT (audio.rs:147).
- Current code (crates/phosphor-audio/src/playback.rs:~283-300) ignores masks entirely: mono duplicates, stereo passes through, >2ch takes frame[0]/frame[1] (front pair by Symphonia bit order).
