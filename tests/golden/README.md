# Golden fixtures — the v3 engine, pinned to the byte

Captured from the pristine v3 Python engine (V4PLAN wave 1, step 1) so the
v4 Rust rewrite has ground truth for every display mode, kit chain, and
`.phos` round-trip. **These bytes are the parity contract.** Regenerating
them is a deliberate act:

    python3 tests/capture_golden.py --record       # regenerate everything
    python3 tests/test_golden_replay.py            # verify (exact bytes)

The replay test must stay green for as long as the v3 engine lives; it is
also the proof that the capture harness itself is sound. Every case was
captured twice with fresh objects and byte-compared before being written.

## Comparison contract for v4

- **This repo's Python engine**: exact byte equality (the replay test).
- **A ported engine (v4 Rust)**: match every recorded call with
  - coordinates within **0.05 px** (v3's own Python↔Rust parity tolerance,
    tests/test_native_parity.py),
  - intensity within **5e-3**,
  - segment counts **exactly equal**, call by call.
- Inputs are stored as the exact little-endian f32 bytes the engine
  consumed (generated in f64, rounded through f32 once). Feed those bytes,
  not re-generated signals.

## Layout

```
manifest.json            capture environment + tolerance contract + case list
inputs/<signal>-<rate>.f32   interleaved stereo f32le inputs, shared by cases
cases/<name>.json        full case description (see below)
cases/<name>.segments.bin    recorded calls' segments, f32le, 5 floats/row
kits/<name>.json + .out.f32  raw KitChain.process() output audio (f32le)
phos/<name>.phos + .json     .phos files + parsed-header/payload records
native-v3/               same case schema, engine "rust-v3" (see below)
```

## Case schema (`cases/<name>.json`)

Engine parameters: `mode`, `sample_rate`, `oversample`, `gain`,
`beam_energy`, `frame_glow_keep`, `width`/`height`, optional `camera`
(yaw/pitch/dolly, applied before any audio) and `kit` (canonical stages,
`[[op, [p0..p3]], …]`, installed after `set_sample_rate`).

Streaming: `chunks` is the exact sequence of f32 counts (interleaved
stereo) fed to `compute()`, one call each, from the head of the input
file (`input.floats` total). State must evolve through **all** calls;
only the calls listed in `recorded_calls` (cold-start head + mature tail)
have their segments stored — `segment_counts_recorded` and the `.bin`
rows are those calls only, concatenated in order. A segment row is
`(x0, y0, x1, y1, intensity)` at the case viewport.

The waveform/ring/tunnel/spectrum family computes rows in Python floats
(f64) and this capture stores them as f32 — storage precision, far inside
the 0.05 px tolerance. The xy/takens/helix family is f32 end to end.

## What is covered

- All 11 modes × deterministic signals (`sweep`, `sine`, `chord`,
  `burst` — silence→wideband hit→quiet tail) × 48 kHz & 96 kHz.
- Python full-rate truth at 192 kHz & 384 kHz for xy / xyz_takens /
  waveform (what the native sinc oversampler approximates).
- One square-viewport case (720×720), the export/postcard aspect.
- The 7 parity kit chains over xy and spectrum, plus raw kit audio for
  those chains and the three starter kits (loader agreement included).
- `.phos` round-trips: plain, unicode (multibyte survives), and overlong
  metadata (the 256-byte header fit-trim ladder lands at 24 chars);
  payload contract `f32 = s16le / 32767.0`.

## native-v3/

A second, clearly-labeled set captured from the v3 Rust core (API v2):
the six `MODE_IDS` modes at oversample 1, **plus the production
windowed-sinc oversampling plans** (pipe 96 kHz × os 2 and × 4 — the
192k/384k detail settings). v3's Python path never oversamples, so the
sinc behavior exists only here; the v4 `phosphor-dsp` oversampler is a
port of that Rust code and gates against this set (same tolerances).
The replay test exercises it only where the core is loadable.

## Notes for the v4 implementer

- `xyz_takens` before tau locks uses the default `tau =
  0.004 × sample_rate`; the first calls in those cases pin that warmup,
  including the `history − 2τ` emission ramp.
- `helix` at 48 kHz never reaches its 0.35 s span — the waveform history
  cap (8192 frames at 48 kHz) truncates it first. That is v3 truth.
- `tunnel`/`spectrum` cold-start calls legitimately record 0 segments
  (FFT not warm, levels under the draw threshold).
- Silence is processed, not skipped: the engine emits segments for
  all-zero audio (the app's quiet-sleep gate lives above the engine).
- Kit phase accumulators, f64→f32 cast order, and integer-sample delays
  follow the contract documented at `phosphor_signal.KitChain`.
