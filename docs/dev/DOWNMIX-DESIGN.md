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

Note mono currently duplicates at unity (playback.rs:290-295), which is
correct and matches row 1. Keep it.

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
  layouts** — libswresample's default never renormalizes here.
- 6.1's back-center is the one composed coefficient: BC → 0.5 per side
  (0.7071 into BL/BR, then 0.7071 into L/R).
- mono → stereo is 0.7071 into each output, not 1.0 — a mono file decoded
  via these ffmpeg paths comes back 3 dB down per channel.

So ffmpeg's default matches this design's raw (un-normalized) taps exactly
across all measured layouts: ITU −3 dB (0.7071) for center/surrounds, LFE
dropped (`lfe_mix_level` default 0), fronts at unity.

### A.3 Divergence: normalization claim in §3.1 is wrong

§3.1 asserts row-sum normalization (`max row sum ≤ 1.0`) is "ffmpeg's default
`-ac 2` behavior". It is not. libswresample's default (`rematrix_maxval`
= INT_MAX effectively, no renorm applied here) leaves FL at 1.0 with row sum
1 + 2c ≈ 2.414. Consequences:

1. **Loudness mismatch**: the proposed DownmixMatrix (k ≈ 0.414 for 5.1) will
   be ≈ 7.7 dB quieter than the ffmpeg decode paths for the same 5.1 source.
   The player (Symphonia path) and the render/bench/postcard (ffmpeg paths)
   would disagree audibly and in beam amplitude.
2. **Clipping risk in ffmpeg paths**: `export_postcard` decodes to **s16le**,
   which hard-clips; a hot correlated 5.1 source can exceed full scale after
   ffmpeg's unnormalized downmix. The f32le paths (bench, render decode) don't
   clip at decode, but downstream beam math sees >±1.0 samples.

### A.4 Recommended amendment

Pick one and make it consistent everywhere:

- **(a)** Keep row-sum normalization in DownmixMatrix (safe, no clip) and add
  an explicit `pan=`/`aresample` matrix or `-af "pan=stereo|..."` (or
  `aresample` with normalized matrix) to the three ffmpeg invocations so they
  match; or
- **(b)** Drop the normalization and match ffmpeg's default (FL 1.0, taps
  0.7071, LFE 0), accepting potential >FS peaks and adding a limiter/clamp.

Either way, fix §3.1's wording: ffmpeg's default is *not* row-sum normalized.

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
