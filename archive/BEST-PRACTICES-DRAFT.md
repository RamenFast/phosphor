---
name: phosphor-development-best-practices
description: >
  Repository-specific engineering rules for safely modifying Phosphor, especially
  its real-time audio transport, DSP, CPU/GPU renderers, CRT persistence model,
  stateful UI, agent protocol, packaging, and experimental music visualizers.
  Use this skill before planning, implementing, reviewing, or debugging any
  non-trivial change to the Phosphor repository.
version: "1.0.0"
repository: "RamenFast/phosphor"
baseline_revision: "fcd3a1234d99e07d96fda24e9e62b658fcab4bcc"
baseline_release: "4.6.2"
last_reviewed: "2026-07-09"
---

# Phosphor development best practices

## Purpose

Phosphor is both an instrument and an application.

A successful change must preserve all of these at once:

- truthful audio-derived geometry,
- stable real-time behavior,
- CPU/GPU visual agreement,
- time-correct phosphor behavior,
- predictable human interaction,
- machine-operable semantic control,
- file and settings compatibility,
- platform honesty,
- reproducible exports,
- correct third-party attribution.

Do not optimize one layer by silently weakening another.

The central engineering lesson from the 4.6.2 audit is:

> Local correctness is not enough. Most serious failures occur where two
> individually reasonable subsystems disagree about time, ownership, state,
> identity, or protocol.

The preferred direction is **contract consolidation**, not broad rewrites.

---

# 1. Activation rules

Use this skill whenever a task touches one or more of the following:

- PipeWire, CPAL, capture, playback, resampling, mixing, or vacuum processing;
- scope geometry, display modes, transforms, oversampling, or `.phoskit`;
- phosphor decay, persistence, bloom, tonemapping, CRT emulation, or color;
- WGPU resources, adapters, surfaces, shaders, CPU rendering, or exports;
- `Shell`, `UiAction`, window modes, popups, mini mode, fullscreen, or focus;
- CLI commands, Unix sockets, JSON messages, schema, MPRIS, or agent control;
- settings, migration, persistence, compatibility keys, or defaults;
- new Windows Media Player / MilkDrop-style visualizers;
- embedded fonts, icons, presets, textures, samples, or package metadata;
- any benchmark, parity, “real CRT,” “headless,” or platform-support claim.

For a trivial spelling-only documentation change, the full workflow is unnecessary.
For everything else, follow it.

---

# 2. Repository mental model

## 2.1 Subsystem ownership

Treat the crates as ownership boundaries, not convenient places to put code.

| Area | Owner | Rule |
|---|---|---|
| Audio samples and clocks | `phosphor-audio` | Transport and timing only; never make UI or rendering policy here |
| Scope geometry | `phosphor-dsp` | PCM/features to renderer-neutral beam segments |
| Beam physics | `phosphor-beam` | One law for deposition, decay, color, and tonemap |
| GPU rasterization | `phosphor-render-gpu` | Implement the shared beam law; do not invent separate physics |
| CPU rasterization | `phosphor-render-cpu` | Match the shared beam law within documented tolerance |
| Formats and shared types | `phosphor-proto` or a dedicated protocol crate | One serialization truth |
| Application state and effects | `phosphor-app` | Adapt inputs to domain actions; do not duplicate domain rules |
| Future studio work | `phosphor-studio` | Experimental until it has a real user path and tests |

## 2.2 Data path

Keep the intended direction explicit:

```text
audio callback
  -> bounded timestamped transport
  -> non-RT analysis/history workers
  -> renderer-neutral feature and beam models
  -> CPU or GPU deposition
  -> time-domain phosphor integration
  -> presentation/export
```

Do not create hidden reverse dependencies such as:

- renderer code reading UI state directly,
- audio callbacks updating widgets,
- protocol handlers mutating renderer internals,
- visualization algorithms owning playback transport,
- settings parsing performing platform effects.

## 2.3 Control path

All control surfaces must converge on the same typed domain action:

```text
human widget ─┐
keyboard      ├─> DomainAction -> reducer -> effects -> receipt
MPRIS        ─┤
agent CLI    ─┘
```

Never implement an agent-only mutation that bypasses the human action path.
Never implement a UI-only mutation that cannot be represented semantically.

---

# 3. Non-negotiable invariants

A change must not merge if it knowingly violates any invariant below.

## 3.1 Time is measured in seconds or media timestamps

Presentation FPS is not simulation time.

Every decay, animation envelope, interpolation, timeout, transition, and export step
must state which clock it uses:

- audio graph timestamp,
- media timestamp,
- monotonic wall clock,
- fixed simulation tick,
- presentation time.

A per-frame constant is forbidden for physical persistence.

Correct pattern:

```text
keep(dt) = exp(-dt / tau)
```

For a stalled frame, either subdivide a clamped `dt` or advance a fixed simulation
tick multiple times. Do not apply one oversized unstable step.

## 3.2 The audio callback is hard real-time code

CPAL documents that modern backends call stream callbacks on a dedicated,
high-priority thread. Treat that as a deadline, not an ordinary worker thread.

After stream startup, callbacks must perform:

- no mutex acquisition,
- no allocation or capacity growth,
- no filesystem or logging I/O,
- no formatting,
- no front-draining vectors,
- no unbounded iteration,
- no graph discovery,
- no UI messaging that can block.

Allowed callback work should be bounded:

- convert or copy a known block,
- write/read a preallocated SPSC ring,
- attach sequence/timestamp metadata,
- update lock-free counters,
- output silence on underrun.

Drops are preferable to blocking. Drops must be counted and observable.

## 3.3 One fact has one authoritative representation

Forbidden duplication includes:

- CLI parser types separate from server parser types;
- handwritten JSON schema separate from runtime types;
- duplicated lists of settings fields;
- separate UI and agent action names;
- renderer-specific persistence formulas;
- copied mode lists in docs, menus, and protocol tables.

Generate secondary representations from primary types whenever practical.

## 3.4 CPU and GPU implement one visual model

A new beam or CRT effect must define:

1. its renderer-neutral mathematical model;
2. its CPU implementation;
3. its GPU implementation;
4. acceptable numerical/image tolerance;
5. a deterministic fixture.

Do not declare parity because both outputs “look close.”

## 3.5 Live and offline rendering agree at equal timestamps

Compare by media or simulation timestamp, not frame index.

Offline export may use a different presentation FPS, but it may not change:

- phosphor half-life,
- animation phase,
- motif memory duration,
- onset timing,
- visualizer seed,
- mix alignment.

## 3.6 Impossible application states must be unrepresentable

Do not encode mutually exclusive modes with unrelated booleans.

Prefer enums and explicit state machines:

```rust
enum WindowMode {
    Normal,
    Mini(MiniState),
    Fullscreen(FullscreenState),
}
```

A transition owns:

- preconditions,
- state update,
- effects,
- completion/error receipt,
- cancellation behavior.

## 3.7 Claims must match capabilities

Do not call X11-only behavior “Linux behavior.”
Do not call an Xvfb GUI “headless.”
Do not call a CRT-inspired palette a measured physical phosphor simulation without
measurement and sources.
Do not call a geometry tap equivalent to human visual inspection.

Expose limitations in UI, probe output, and documentation.

---

# 4. Required change workflow

## Step 1 — Read the failure history

Before editing code:

1. read `docs/dev/BUGLOG.md`;
2. search for prior changes to the same subsystem;
3. locate the nearest golden, property, integration, and benchmark fixtures;
4. identify whether the problem is another instance of a known ownership pattern.

If three bugs share a root cause, stop patching symptoms and propose a boundary change.

## Step 2 — Write the contract before the patch

State in the issue, plan, or PR:

- user-visible behavior;
- owning subsystem;
- input and output types;
- clock source;
- boundedness requirements;
- failure behavior;
- platform scope;
- agent-visible behavior;
- compatibility impact;
- acceptance tests.

For visual work, include a deterministic reference input and expected qualitative
and quantitative behavior.

## Step 3 — Add a failing end-to-end receipt

Prefer a receipt that crosses the real boundary:

- CLI text -> typed request -> socket -> server -> state receipt;
- audio fixture -> analysis -> geometry -> CPU/GPU image comparison;
- UI action -> reducer -> effect -> state transition;
- persistence setting -> elapsed time -> measured residual energy;
- package build -> installed notice files.

A component-only unit test is not sufficient for a boundary regression.

## Step 4 — Instrument before optimizing

Measure the relevant distribution, not only an average:

- callback duration and deadline misses;
- ring fill and drops;
- analysis queue latency;
- frame-time percentiles;
- GPU submission/present latency;
- allocations per frame;
- persistent texture bandwidth;
- action accepted/applied/completed latency;
- export/live timestamp drift.

Record hardware, backend, driver, sample rate, FPS, command, commit, and fixture.

## Step 5 — Implement at the owner boundary

Do not place a convenient patch in `Shell` if the invariant belongs in:

- the protocol type,
- the state reducer,
- the audio transport,
- the beam model,
- the platform capability adapter,
- the settings validator.

## Step 6 — Run the full verification matrix

Use the repository’s pinned toolchain and existing commands first. A typical gate is:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features
cargo deny check
```

Run targeted golden, replay, property, renderer, export, package, X11, and Wayland
tests relevant to the change.

Do not silently skip a failing test because a local device is unavailable. Mark the
test as not run and explain the missing capability.

## Step 7 — Leave a machine-readable handoff

Report:

- changed contracts;
- tests added and run;
- tests not run;
- measurements before/after;
- compatibility or migration notes;
- remaining risks;
- exact reproduction commands;
- source/license additions.

---

# 5. Real-time audio rules

## 5.1 Preferred transport

Use fixed-capacity, preallocated SPSC block rings.

Each block should carry enough metadata to diagnose alignment:

```rust
struct AudioBlock<const N: usize> {
    sequence: u64,
    graph_time_ns: u64,
    sample_rate_hz: u32,
    channels: u16,
    frames: u16,
    samples: [f32; N],
}
```

Exact layout may differ, but preserve these semantics:

- monotonically increasing sequence;
- a monotonic or graph timestamp;
- explicit channel/frame counts;
- bounded capacity;
- explicit overrun policy.

History, resampling, visualization analysis, and export copies happen off the
callback thread.

## 5.2 Drop policy

Every ring must document what happens when full:

- drop newest;
- overwrite/drop oldest;
- emit silence;
- backpressure outside the callback.

A callback must never wait for a consumer.

Expose at least:

- blocks received;
- blocks dropped;
- playback underruns;
- sequence gaps;
- maximum observed ring occupancy;
- maximum callback duration.

## 5.3 Mixing

Do not mix by “whatever vectors were drained this UI frame.”

Mix sources using timestamps and a defined quantum.

Specify:

- alignment tolerance;
- missing-source behavior;
- gain per source;
- headroom policy;
- limiter or normalization policy;
- latency accounting;
- resampling ownership.

Never sum arbitrary source counts at unity without headroom.

## 5.4 Channel layout

Channel count is not channel meaning.

For capture and file playback:

- request or validate channel positions where the backend supports them;
- downmix using a layout-aware matrix;
- test 1.0, 2.0, 2.1, 5.1, and 7.1 fixtures;
- document LFE treatment;
- do not assume channels 0 and 1 are always left and right.

## 5.5 Resampling

Avoid repeated `drain(0..n)`, `collect`, and front removal.

Prefer:

- fixed input/output work buffers;
- incremental indices;
- ring-backed queues;
- `process_into_buffer`-style APIs;
- explicit latency reporting.

The audible path and scope path should share one resampled timeline unless a
documented design requires otherwise.

---

# 6. DSP and scope geometry rules

## 6.1 Preserve renderer neutrality

A scope mode produces a renderer-neutral representation such as beam segments,
points, or a feature frame. It must not know whether WGPU or the CPU rasterizer is
active.

A mode specification must define:

- input channels and expected normalization;
- sample/time window;
- coordinate system;
- bounds;
- meaning of intensity, width, and color fields;
- deterministic seed behavior;
- behavior for silence, mono, polarity inversion, clipping, NaN, and discontinuity.

## 6.2 Numerical hygiene

At every public DSP boundary:

- reject or sanitize NaN and infinity;
- clamp only where physically or visually justified;
- document normalization;
- avoid sample-rate-dependent hidden constants;
- define behavior on an empty or short block;
- keep transforms deterministic for a fixed input and seed.

## 6.3 Oversampling

Separate two concepts:

- **physics/geometry sampling**: how accurately the curve is formed;
- **display supersampling**: how accurately it is rasterized.

Do not expose renderer implementation names as user-facing quality levels.
A quality preset should state what changes and its cost.

## 6.4 Golden tests

For every mode, keep compact deterministic fixtures including:

- silence;
- mono sine;
- quadrature sine pair;
- inverted channel;
- impulse;
- clipped input;
- swept frequency;
- seeded music-like multitone;
- sample-rate variants.

Golden changes require a migration note explaining whether the physical model,
normalization, or only raster precision changed.

---

# 7. Beam, persistence, and CRT emulation

## 7.1 Shared time-domain model

The beam model owns deposition and phosphor response.

A recommended two-reservoir CRT model is:

```text
fast' = fast * exp(-dt / tau_fast) + beam
slow' = slow * exp(-dt / tau_slow) + beam * slow_gain
energy = fast + slow
```

Use seconds. Recommended presets may vary, but no preset may encode retention as a
fixed per-frame multiplier.

## 7.2 Pass ordering

Use an explicit conceptual order:

```text
beam deposition
-> phosphor energy integration
-> optional diffusion/bloom
-> phosphor color response
-> tone mapping / output encoding
-> scanline and aperture presentation effects
-> glass/reflection overlay
```

Scanlines are not persistence.
A vignette is not tube curvature.
A blur is not phosphor integration.

## 7.3 CRT modes

Keep CRT personalities composable rather than branching the renderer:

- `clean`: minimal display texture;
- `studio`: controlled scanline/aperture, calibrated response;
- `consumer`: more bloom, curvature, and softness;
- `exhausted`: reduced focus, uneven response, restrained artifacts.

Each parameter must have a defined unit or normalized meaning.

## 7.4 Frame-rate invariants

Required test:

1. deposit the same impulse;
2. advance one second at 30, 60, 144, and 240 presentation FPS;
3. compare integrated residual energy;
4. fail if outside tolerance.

Also test a long frame stall and offline/live equal timestamps.

## 7.5 Color honesty

Keep scene-linear energy until the final output stage.

State whether output is:

- linear,
- sRGB encoded,
- extended sRGB,
- HDR/PQ/HLG.

Do not apply a gamma approximation multiple times.
Do not label a theme “real P7” unless backed by measured spectral/decay data.
Prefer “P7-inspired” or “P7-style” for artistic presets.

---

# 8. GPU renderer rules

## 8.1 Adapter, device, and surface identity

A WGPU adapter represents a physical graphics/compute device, and surface
capabilities are queried for a particular adapter.

Do not request a new adapter during a renderer rebuild while reusing an existing
device, queue, or surface assumptions.

Keep the selected adapter, device, queue, and surface configuration as one coherent
graphics context. Rebuild resources from that context.

## 8.2 Resource lifecycle

- Reuse buffers and textures when dimensions/formats permit.
- Grow capacity geometrically; do not resize every frame.
- Label GPU resources and passes.
- Handle `SurfaceError` explicitly.
- Keep pipeline creation and shader compilation off latency-sensitive interaction
  paths where possible.
- Do not block the UI thread waiting for GPU completion except in an explicit
  diagnostic/export path.
- Use validation in development and trace/capture facilities for hard GPU bugs.

## 8.3 Shader parity

The shader may optimize the model, but it may not redefine it.

Keep beam and CRT constants in shared generated data or clearly versioned shader
uniform structures. A change to WGSL must include:

- CPU equivalent or a documented GPU-only presentation effect;
- image/parity test;
- bounds and NaN behavior;
- backend coverage where available.

## 8.4 Color and surface capabilities

Intersect desired format/color space with advertised surface capabilities.
Keep an SDR fallback.
Treat unknown HDR metadata as unknown, not as proof of SDR.

---

# 9. CPU renderer rules

## 9.1 Reuse scratch memory

Do not allocate these per frame:

- prepared segment vectors;
- nested tile/bin vectors;
- full-plane temporary images;
- repeated conversion buffers.

Prefer flat reusable storage:

```text
bin_offsets: Vec<u32>
bin_indices: Vec<u32>
prepared_segments: Vec<PreparedSegment>
active_tiles: BitSet
```

A CSR-like bin layout is preferable to `Vec<Vec<...>>`.

## 9.2 Bound work

Use conservative spatial bins and active tiles.
Avoid scanning the full plane for sparse traces when the shared beam law permits a
bounded active region.

Do not sacrifice parity for speed without a named quality mode and tests.

## 9.3 Benchmark correctly

Report:

- frame-time percentiles;
- segment count;
- resolution;
- persistence/bloom settings;
- thread count;
- CPU model;
- build profile;
- commit;
- fixture.

A peak microbenchmark FPS number is not a product-performance claim.

---

# 10. Application state and UI rules

## 10.1 Keep `Shell` thin

`Shell` should adapt events, render views, and dispatch effects.
It should not remain the owner of every domain rule.

Prefer modules such as:

```text
app_state.rs
actions.rs
effects.rs
window_state.rs
source_state.rs
playback_state.rs
export_state.rs
agent_state.rs
view_model.rs
```

## 10.2 Reducers and effects

A reducer is deterministic:

```text
(state, action) -> (new_state, effect_requests)
```

Effects perform:

- filesystem work;
- device discovery;
- renderer rebuilds;
- exports;
- window operations;
- socket replies.

Effects return structured completion or failure actions.

## 10.3 Escape and destructive actions

Use one tested precedence table.

Recommended semantics:

1. close modal/dialog;
2. close popup/menu;
3. leave temporary interaction mode;
4. leave fullscreen;
5. otherwise do nothing.

Do not make bare Escape quit a normal window.
Use an explicit quit command such as Ctrl+Q and surface unsaved work.

## 10.4 Long operations

Exports, file scans, renderer rebuilds, kit compilation, and device discovery must
be cancellable or asynchronous where practical.

Expose progress and cancellation to both UI and agent protocol.

## 10.5 Settings persistence

Use one serde-derived settings type with:

- defaults;
- versioned migrations;
- validation;
- generated schema;
- deliberate unknown-key compatibility;
- atomic write.

Preferred save sequence:

1. write a temporary file;
2. flush and `sync_all` where appropriate;
3. atomically rename;
4. preserve/recover the last valid file on failure.

Never silently replace malformed settings with defaults without a visible warning.

---

# 11. Platform capability rules

Create a platform capability service for:

- window positioning;
- work area;
- transparency;
- popup/transient behavior;
- always-on-top;
- override-redirect;
- fullscreen;
- focus/raise;
- screenshot availability;
- true headless rendering.

The UI and agent probe should report capabilities and disabled reasons.

A feature must choose one:

- supported;
- emulated with documented limitations;
- disabled with reason;
- experimental.

Never shell out to X11-specific tools from generic application logic.

---

# 12. Agent and CLI rules

## 12.1 One typed protocol

Define shared request, response, action, target, error, and event types in one crate.

Derive as appropriate:

- `Serialize`;
- `Deserialize`;
- `JsonSchema`;
- CLI argument metadata/help;
- documentation tables.

Schemars is designed so generated schemas reflect Serde serialization attributes;
use that property rather than maintaining a third handwritten schema.

Pin the JSON Schema dialect used by the protocol instead of depending on an
implicit generator default.

## 12.2 Required handshake

The server—not the client—emits a handshake containing:

- protocol version;
- application version;
- instance ID;
- process ID;
- capability set;
- schema/dialect version;
- state revision;
- optional build commit.

Support explicit instance selection.

## 12.3 Action registry

An action descriptor should include:

```text
id
label
description
argument schema
current value
enabled
disabled reason
danger level
reversibility
UI semantic ID
keyboard binding
MPRIS mapping
agent visibility
```

UI, keyboard, MPRIS, and CLI invoke the same action ID.

## 12.4 Receipts

Every mutation should produce structured lifecycle events:

```text
accepted -> applied -> completed
                  \-> failed
                  \-> cancelled
```

Include:

- correlation ID;
- idempotency key where useful;
- starting and ending state revision;
- human-readable message;
- machine-readable error code;
- repair action expressed in syntax that is integration-tested.

## 12.5 Action Lens

The preferred semantic agent surface is:

```text
phosphor observe
phosphor invoke <action-id> [args]
phosphor watch
```

`observe` should expose:

- active window/view/dialog;
- semantic control tree;
- stable IDs;
- role, label, value;
- enabled state and disabled reason;
- logical and physical bounds;
- available actions;
- current source/player/beam state;
- platform/backend capabilities;
- optional thumbnail, screenshot, or frame hash.

`invoke` must dispatch the same domain action as the human widget.

`watch` should stream state revisions, action receipts, dialogs, errors, toasts,
operation progress, and optional visual revisions.

When an agent acts, the human should see:

- a restrained widget pulse;
- a toast describing the change;
- the same correlation ID;
- no focus theft unless requested.

Coordinate clicking may be offered only as a debugging fallback. It is never the
primary control mechanism.

## 12.6 End-to-end protocol tests

For every command:

1. parse actual CLI text;
2. serialize shared request;
3. send through the real socket framing;
4. deserialize on the server;
5. validate the action;
6. apply to a test state;
7. assert the receipt;
8. validate request/response against generated schemas;
9. execute any returned repair command.

Separate client and server unit tests do not prove compatibility.

---

# 13. Adding creative music visualizers

Phosphor may expand toward Windows Media Player, Winamp, MilkDrop, or projectM-like
expressiveness, but it should not become an arbitrary shader jukebox.

The Phosphor-native rule is:

> Every persistent visual behavior must be traceable to a named audio feature,
> deterministic state, or documented aesthetic modulation.

projectM demonstrates the power of PCM analysis, beat detection, FFT-derived
features, equations, and pixel shaders. Phosphor should borrow the expressive
freedom while preserving instrument truth and reproducibility.

## 13.1 Three-layer visualizer architecture

### Layer A — Analysis

Runs off the real-time callback.

Input:

- timestamped audio blocks.

Output:

```rust
struct FeatureFrame {
    media_time: Duration,
    rms: Bands,
    onset: Bands,
    chroma: [f32; 12],
    tonnetz: [f32; 6],
    tempogram_peaks: SmallVec<[TempoPeak; 4]>,
    stereo_width: f32,
    phase_coherence: Bands,
    spectral_centroid: f32,
    novelty: f32,
    recurrence_matches: SmallVec<[RecurrenceMatch; 8]>,
}
```

The exact representation may evolve, but it must be bounded, timestamped,
serializable for fixtures, and renderer-neutral.

### Layer B — Deterministic scene/geometry

Consumes feature frames and a seeded preset state.

Output:

- beam segments;
- particles with bounded lifetime;
- mesh/field parameters;
- renderer-neutral scene commands.

No wall-clock randomness.
Any randomness comes from an explicit seed and deterministic PRNG.

### Layer C — Rendering and CRT presentation

Maps the deterministic scene into the shared beam/CRT model.
Presentation-only effects may be GPU-specific only when clearly separated from
instrument geometry and disabled in parity tests.

## 13.2 Feature extraction guidance

Useful feature families include:

- constant-Q chroma for pitch-class energy;
- Tonnetz coordinates for harmonic movement;
- onset strength and bandwise spectral flux;
- local tempograms for rhythmic periodicity;
- stereo width and bandwise correlation;
- recurrence or nearest-neighbor affinity over recent feature history;
- spectral centroid, flatness, contrast, and reassigned ridges.

Do not add a heavy dependency blindly. Prototype offline, benchmark, then either:

- implement the required bounded subset;
- add an optional feature-gated crate;
- compute expensive features only for file/offline mode;
- run analysis at a lower hop rate;
- degrade gracefully.

## 13.3 Preset contract

A preset must declare:

```text
id and version
author and license
deterministic seed
required features
analysis hop/window
history duration
quality tiers
parameter schema
default values
GPU requirements
CPU fallback
estimated cost
safe ranges
migration behavior
```

Never execute untrusted arbitrary native code from a preset.

If an equation or shader language is added:

- sandbox or strictly validate it;
- bound loops and memory;
- define resource limits;
- version the language;
- include source attribution;
- reject unsupported capabilities before rendering.

## 13.4 Scene switching

Avoid random hard cuts disconnected from music.

Use one or more named triggers:

- structural novelty;
- section recurrence;
- bar boundary;
- sustained energy transition;
- user action;
- deterministic playlist schedule.

Crossfade state by timestamp.
A repeated run with the same input, seed, and settings must switch scenes at the
same media times.

## 13.5 Degradation order

When over budget, reduce quality predictably:

1. lower decorative particle count;
2. lower recurrence ghost count;
3. reduce curve samples;
4. reduce bloom taps;
5. lower analysis update rate;
6. reduce internal presentation resolution.

Do not first drop audio, change the time model, or block the callback.

Expose the active degradation level in diagnostics.

---

# 14. Recurrence Bloom integration profile

`Recurrence Bloom` is the recommended first memory-aware visualizer.

## 14.1 Meaning

- bright central carrier: immediate stereo Lissajous truth;
- harmonic petals: chroma-derived pitch-class structure;
- global orientation: Tonnetz harmonic position;
- breathing: tempogram/onset phase;
- horizontal opening: stereo width;
- filament symmetry: phase coherence;
- sparks: onset current;
- ghost blooms: nearest non-local recurrence matches;
- rupture/new petal: novelty.

## 14.2 Minimum viable implementation

Start with:

- existing stereo carrier;
- 12-bin chroma at a modest analysis rate;
- one seeded polar bloom;
- 8–12 seconds of bounded feature history;
- cosine nearest-neighbor recurrence;
- at most three ghosts;
- shared CRT persistence;
- CPU and GPU parity fixture.

Do not begin with a full scene language.

## 14.3 Data ownership

```text
audio transport
  -> analysis worker
      -> fixed-capacity FeatureFrame ring
          -> visualization state reducer
              -> beam segments
```

The renderer never searches history.
The callback never computes chroma.
The UI never owns recurrence buffers.

## 14.4 Acceptance tests

1. Silence decays completely and does not generate petals.
2. Mono collapses the carrier and preserves deterministic harmonic geometry.
3. Polarity inversion changes the expected carrier axis.
4. A static chord produces stable petals.
5. A chord change alters petals without random scene switching.
6. A repeated section restores recognizably similar ghost geometry.
7. The same fixture and seed produce identical geometry before rasterization.
8. 30/60/144/240 FPS produce equivalent phosphor decay.
9. CPU/GPU frames remain within the documented image tolerance.
10. Agent observation reports preset, feature revision, memory length, and controls.
11. Overload reduces decorative quality before losing audio blocks.
12. Offline and live results match at equal media timestamps.

## 14.5 User controls

Keep the primary surface small:

- Memory;
- Bloom;
- Carrier;
- Recurrence;
- Tempo Lock;
- Novelty Flash;
- Phosphor;
- Tube.

Expose advanced analysis parameters only in an expert panel or preset file.

---

# 15. Dead-code and scope-control rules

Do not leave speculative production shells in the active workspace.

A future feature must be one of:

- implemented and reachable;
- behind an explicit experimental feature;
- documented as a design note outside production modules;
- removed.

Avoid broad `#[allow(dead_code)]`.
For fields intentionally retaining ownership/lifetime, use a name and comment that
state the invariant.

Before release, check for:

- settings with no consumer;
- actions with no reachable UI/CLI path;
- schema fields absent from runtime;
- examples treated as production;
- stub crates;
- duplicate handoff/status documents;
- embedded assets absent from notice generation.

Optional tools such as `cargo machete` or `cargo udeps` may supplement review when
configured, but semantic dead state still requires human/agent inspection.

---

# 16. Licensing and attribution

When adding a dependency or embedded asset:

1. record source, author, version/commit, and license;
2. verify redistribution terms;
3. include required license/copyright text;
4. update `THIRD_PARTY_NOTICES.md`;
5. include notices in package outputs and About UI where appropriate;
6. run dependency license/advisory/source policy checks;
7. update the SBOM.

`cargo-deny` can check licenses, banned/duplicate crates, advisories, and sources.
It does not replace review of embedded fonts, icons, shader presets, images,
samples, or copied algorithms.

For visualizer presets inspired by MilkDrop/projectM:

- do not copy preset code, textures, or equations without compatible licensing;
- credit conceptual and algorithmic sources;
- keep original preset authorship metadata;
- distinguish independent reimplementation from imported content.

---

# 17. Review checklists

## 17.1 Universal PR checklist

- [ ] Owning subsystem named
- [ ] Clock source named
- [ ] State transition documented
- [ ] Failure and cancellation behavior documented
- [ ] No new duplicated representation of an existing fact
- [ ] End-to-end receipt added
- [ ] Deterministic fixture added or reused
- [ ] CPU/GPU and live/offline impact considered
- [ ] Agent-observable behavior considered
- [ ] X11/Wayland/platform capability considered
- [ ] Settings migration considered
- [ ] License/notice impact considered
- [ ] Relevant measurements included
- [ ] Full commands and skipped tests reported

## 17.2 Audio PR checklist

- [ ] Zero callback locks
- [ ] Zero callback allocations after startup
- [ ] Bounded callback work
- [ ] Explicit ring-full/empty policy
- [ ] Sequence/timestamp metadata preserved
- [ ] Xrun/drop counters exposed
- [ ] Channel positions/layout handled
- [ ] Stress test run
- [ ] Audible and visual timelines checked

## 17.3 Renderer/CRT PR checklist

- [ ] Mathematical model written before implementation
- [ ] `dt` is explicit and time-based
- [ ] Shared CPU/GPU constants or versioned uniforms
- [ ] Cross-FPS decay test
- [ ] CPU/GPU image tolerance test
- [ ] Live/offline timestamp test
- [ ] Color space and transfer function stated
- [ ] GPU rebuild uses coherent adapter/device/surface context
- [ ] No avoidable per-frame allocation
- [ ] Frame-time percentiles reported

## 17.4 Agent/protocol PR checklist

- [ ] Shared typed request/response/action
- [ ] Generated schema
- [ ] Pinned schema dialect
- [ ] Server handshake
- [ ] Explicit instance identity
- [ ] Correlation and revision IDs
- [ ] Structured disabled reason
- [ ] Repair command is executable
- [ ] CLI-to-server round trip tested
- [ ] Human-visible action feedback
- [ ] Stream termination reason distinguished

## 17.5 Visualizer PR checklist

- [ ] Audio-feature-to-visual mapping documented
- [ ] Deterministic seed and PRNG
- [ ] Analysis off RT callback
- [ ] Bounded history and particle state
- [ ] CPU fallback or explicit capability gate
- [ ] Quality degradation order
- [ ] Silence/mono/polarity/repetition fixtures
- [ ] No arbitrary unlicensed preset content
- [ ] Preset version and migration behavior
- [ ] Controls exposed through shared action registry

---

# 18. Definition of done

A change is done only when:

- the intended behavior is implemented at the correct ownership boundary;
- the real user/agent path is covered by an end-to-end receipt;
- timing behavior is independent of presentation FPS where required;
- real-time callbacks remain bounded and observable;
- CPU/GPU and live/offline contracts are preserved or deliberately versioned;
- state transitions cannot produce known impossible combinations;
- platform limitations are explicit;
- protocol/schema/help are generated from shared types;
- settings are valid, migrated, and atomically persisted;
- new assets and dependencies are attributed and packaged correctly;
- the PR contains enough evidence for the next agent to reproduce the result.

---

# 19. Agent handoff template

Use this exact structure at the end of substantial work:

```markdown
## Change summary
What changed and why.

## Contract
Owner:
Clock:
Inputs:
Outputs:
Failure behavior:
Compatibility:

## User path
Exact human gesture or command.

## Agent path
Exact observe/invoke/watch sequence and expected receipts.

## Verification
Commands run:
Fixtures:
Platforms/backends:
CPU/GPU:
Live/offline:

## Measurements
Before:
After:
Hardware/driver/sample rate/FPS:

## Not run
Tests or platforms not available, with reason.

## Risks and follow-up
Known limitations and the smallest next task.

## Attribution
Dependencies/assets/algorithms added and notice changes.
```

---

# 20. External engineering references

These references support the rules above. The repository lockfile and pinned API
versions remain authoritative for exact signatures.

1. Rust API Guidelines  
   https://rust-lang.github.io/api-guidelines/

2. CPAL documentation — callbacks, stream configuration, timestamps  
   https://docs.rs/cpal/latest/cpal/

3. WGPU documentation — adapter/device/surface model, validation, color spaces  
   https://docs.rs/wgpu/latest/wgpu/

4. Schemars documentation — deriving JSON Schema from Serde-compatible Rust types  
   https://docs.rs/schemars/latest/schemars/

5. JSON Schema specification — current published dialect information  
   https://json-schema.org/specification

6. Cargo test command reference  
   https://doc.rust-lang.org/cargo/commands/cargo-test.html

7. Clippy usage  
   https://doc.rust-lang.org/clippy/usage.html

8. cargo-deny — license, advisory, source, and dependency policy checks  
   https://github.com/EmbarkStudios/cargo-deny

9. projectM — PCM analysis, beat detection, FFT, equations, and shader presets  
   https://github.com/projectM-visualizer/projectm

10. librosa constant-Q chroma  
    https://librosa.org/doc/latest/generated/librosa.feature.chroma_cqt.html

11. librosa Tonnetz tonal centroid features  
    https://librosa.org/doc/latest/generated/librosa.feature.tonnetz.html

12. librosa tempogram  
    https://librosa.org/doc/latest/generated/librosa.feature.tempogram.html

13. librosa recurrence matrix  
    https://librosa.org/doc/latest/generated/librosa.segment.recurrence_matrix.html

---

# Final rule

Before adding another feature, ask:

> Which existing truth will own it, and what test will make disagreement impossible?

If there is no clear answer, the feature is not ready to implement.
