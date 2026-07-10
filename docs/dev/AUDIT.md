---
title: "Phosphor 4.6.2 — Codebase, Signal Path, UX, and Agent-Surface Audit"
repository: "RamenFast/phosphor"
audited_revision: "fcd3a1234d99e07d96fda24e9e62b658fcab4bcc"
audit_date: "2026-07-09"
auditor_note: "Static source and documentation audit through the GitHub connector. The raw archive mentioned in chat was not mounted in the execution workspace, and outbound git clone was unavailable, so cargo check/clippy/tests and runtime profiling were not rerun locally."
---

# Phosphor 4.6.2 audit

## Executive verdict

Phosphor is real engineering, not a toy visualizer. Its strongest work is unusually strong:

- The DSP is centralized instead of duplicated.
- CPU and GPU renderers are built around the same analytic Gaussian beam model.
- The project has golden replay fixtures, property tests, a regression ledger, and performance gates.
- PipeWire capture, file playback, MPRIS, export, composition, transform kits, and agent control are all integrated into one Rust application.
- Comments frequently preserve the reason behind a constraint rather than merely describing syntax.

The project’s weakness is not that the algorithms are generally bad. It is that **local correctness is stronger than system correctness**.

The most serious risks sit at boundaries:

1. audio callback → shared buffers,
2. wall-clock time → per-frame phosphor decay,
3. UI gesture → window-manager state,
4. CLI parser → socket parser → published schema,
5. live renderer → offline renderer/export,
6. X11-specific behavior → the claimed Linux application,
7. embedded third-party assets → package notices.

This explains the release history and bug ledger. Most shipped failures were not “the Gaussian math is wrong.” They were “two individually reasonable components disagreed about ownership, timing, state, or protocol.”

The right next phase is therefore **contract consolidation**, not another renderer rewrite.

---

# 1. Audit scope and evidence quality

## What was examined

The review traced these paths:

- CLI entry and command routing
- agent client and Unix-socket server
- UI action model and shell orchestration
- settings persistence
- PipeWire capture and playback
- mixing and vacuum routing
- DSP and oversampling
- CPU rasterization
- GPU beam and composite shaders
- export/live parity claims
- window, mini-mode, popup, and fullscreen behavior
- regression logs, handoff notes, benchmark doctrine, packaging, and credits

## Evidence labels used below

**Verified defect** means source code or documentation directly contradicts another source contract.

**Verified dead state** means a field, crate, or command is present but has no meaningful production implementation or consumer in the indexed source.

**Architectural risk** means the implementation has a known failure mechanism even if it may behave acceptably on the author’s machine.

**Runtime hypothesis** means it should be measured before changing behavior.

## Important limitation

This was not a fresh local build. The raw source archive referenced in chat was not present under `/mnt/data`, and the execution container could not resolve GitHub for a clone. Therefore:

- compiler warnings were not independently reproduced,
- `cargo test`, `cargo clippy`, and benches were not rerun,
- flamegraphs and audio xruns were not measured,
- Wayland behavior was inferred from platform-specific code,
- package contents were inferred from packaging scripts.

The source-level contradictions identified below do not depend on a local build.

---

# 2. Architecture map for the next agent

## 2.1 Major crates

| Crate | Responsibility | Current confidence |
|---|---|---:|
| `phosphor-dsp` | Stereo samples → beam segments; all display modes; transforms; oversampling | High |
| `phosphor-beam` | Shared beam, decay, color, tonemap, and composite laws | High, with one major time-model flaw |
| `phosphor-render-gpu` | WGPU energy ping-pong, segment deposition, composite | High |
| `phosphor-render-cpu` | SIMD/rayon CPU energy raster and composite | Medium-high |
| `phosphor-audio` | PipeWire graph, capture, playback, mix, vacuum | Medium-low |
| `phosphor-proto` | Settings and `.phos` / `.phoskit` formats | Medium |
| `phosphor-app` | CLI, socket, shell, UI, window state, exports, MPRIS | Low-medium because it is the integration hotspot |
| `phosphor-studio` | Announced scene/timeline compiler | Very low; presently a stub |

## 2.2 Live signal path

```text
PipeWire capture callback
  └─ locks SampleRing or mix-member Vec
       └─ shell periodically drains into a newly returned Vec
            └─ optional kit transform
                 └─ mode-specific DSP emits Vec<[f32; 5]>
                      ├─ GPU: instance buffer → analytic beam pass
                      └─ CPU: prepare/bin/deposit → RGBA composite
                           └─ window surface / export / tap summary
```

For file playback:

```text
Symphonia or .phos decoder
  └─ optional rubato resampler
       ├─ audible ring → PipeWire playback callback
       └─ scope SampleRing
```

The design goal—one resampled stream feeding both ear and scope—is excellent. The implementation cost is that multiple mutexes, vectors, drains, and copies now sit in timing-sensitive paths.

## 2.3 Control path

```text
CLI positional parser (agent.rs)
  └─ JSON request
       └─ Unix socket
            └─ independent JSON parser (control.rs)
                 └─ ControlVerb
                      └─ Shell maps to UI/MPRIS behavior
```

The schema is authored a third time in `agent.rs`.

This three-copy arrangement is already drifting.

## 2.4 UI path

`Shell` is the system integrator. It owns or coordinates:

- renderer lifetime,
- window and popup windows,
- mini/fullscreen/pin transitions,
- audio engine,
- source truth,
- player state,
- export jobs,
- settings,
- MPRIS,
- control socket,
- action queue,
- timing and frame scheduling,
- transient UI state.

`chrome.rs` also contains a large amount of stateful UI logic and directly pushes a broad `UiAction` enum.

This is not inherently invalid, but it means nearly every cross-feature change enters the same state machine.

---

# 3. Highest-priority findings

## P0 — Phosphor decay is frame-rate dependent

**Type:** verified architectural defect  
**Area:** accuracy, UX consistency, export parity

The beam model defines flash and glow retention as a factor applied once per rendered frame. GPU and CPU both decay the complete energy planes each frame. However, the application supports monitor-paced, fixed FPS presets from 30 through 480, and uncapped rendering.

That means the apparent phosphor half-life changes when:

- the monitor refresh rate changes,
- the user changes max FPS,
- the compositor throttles the window,
- the CPU renderer misses frames,
- the app is occluded or experiences a stall,
- offline export advances at a different cadence.

A fixed `FLASH_KEEP = 0.50` is not a physical decay law unless the simulation step is fixed. At 30 FPS and 240 FPS it represents radically different time constants.

### Why this matters

The project promises a “real P7 CRT” and live/offline identity. A frame-dependent decay undermines both promises even though CPU and GPU agree with each other.

### Correct design

Represent phosphor decay in seconds:

```text
keep(dt) = exp(-dt / tau)
```

Use separate time constants for flash and glow. Apply a floor in a time-scaled way, or replace the subtractive floor with a defined cutoff after exponential decay.

Choose one of two models:

1. **Fixed simulation tick**  
   Advance energy at a fixed rate, such as 120 Hz, independent of presentation. Rendering may interpolate or display the latest state.

2. **Variable `dt`, clamped and subdivided**  
   Compute decay from elapsed simulation time. If a frame stalls, subdivide large `dt` to avoid numerical/visual discontinuity.

### Acceptance tests

- The same impulse rendered for one real second at 30, 60, 144, and 240 presentation FPS must have equivalent residual energy within tolerance.
- Live and offline rendering of a deterministic signal must match at equal timestamps, not merely equal frame indices.
- Changing `max_fps` while a static trail decays must not alter its measured half-life.

### Files to change

- `crates/phosphor-beam/src/lib.rs`
- `crates/phosphor-render-gpu/src/shaders.wgsl`
- `crates/phosphor-render-gpu/src/lib.rs`
- `crates/phosphor-render-cpu/src/lib.rs`
- `crates/phosphor-app/src/shell.rs`
- offline render/export scheduling

---

## P0 — Real-time audio callbacks take locks and perform allocation/memory movement

**Type:** verified architectural risk  
**Area:** performance, reliability, audio accuracy

The PipeWire capture callback locks either:

- `Arc<Mutex<SampleRing>>`, or
- `Arc<Mutex<Vec<f32>>>` for each mix member.

Inside those locks it may:

- reserve vector capacity,
- convert bytes sample-by-sample,
- append to two vectors,
- trim with `Vec::drain` from the front,
- move large portions of memory.

The playback callback uses a mutex-backed `VecDeque`, pops samples one at a time, resizes a scratch vector, and copies each float back to bytes.

The code comment calls this “RT-safe in practice.” It is not hard real-time safe. It may be good enough under normal load, but its worst case is unbounded by design: mutex acquisition can block, allocation can invoke the allocator, and front drains can move a large buffer.

### Correct design

Use preallocated single-producer/single-consumer rings:

- capture callback is producer,
- DSP/shell worker is consumer,
- playback decoder is producer,
- PipeWire playback callback is consumer.

Requirements:

- no allocation after stream start,
- no mutex in the process callback,
- bulk slice writes/reads,
- atomic cursors,
- explicit overrun/underrun counters,
- timestamps or sequence numbers per block,
- separate rolling export history maintained off the RT thread.

A practical layout:

```text
RT capture callback
  └─ SPSC block ring: {sequence, graph_time, frames, L/R samples}

non-RT ingest worker
  ├─ feeds DSP queue
  ├─ copies into 10-second history ring
  └─ records drops/late blocks
```

### Acceptance tests

- Run with allocator instrumentation that fails any allocation on RT callback threads.
- Assert no mutex acquisition in callbacks.
- Stress with UI stalls and CPU contention; count overruns rather than replaying stale audio.
- Use PipeWire timing metadata to validate monotonic sequence and channel alignment.
- Expose underrun, overrun, and captured-frame counters in the HUD and agent probe.

### Files to change

- `crates/phosphor-audio/src/ring.rs`
- `crates/phosphor-audio/src/engine.rs`
- `crates/phosphor-audio/src/playback.rs`

---

## P0 — The agent client, socket server, and schema disagree

**Type:** verified defects  
**Area:** agent reliability

### Confirmed contradiction: numeric target IDs

The CLI client interprets `phosphor ctl target 42` as JSON number `42`.

The socket server’s `target` parser accepts only a JSON string.

Therefore a client input explicitly supported and unit-tested by the client can be rejected by the server.

### Confirmed contradiction: generated “fix” syntax is not accepted by the CLI

Server-side errors recommend forms such as:

```text
phosphor ctl seek --seconds -5
phosphor ctl volume --value 0.8
phosphor ctl mode --name xy
```

The actual CLI parser accepts positional syntax:

```text
phosphor ctl seek -5
phosphor ctl volume 0.8
phosphor ctl mode xy
```

An agent following the advertised repair can fail again.

### Confirmed schema drift

Examples include:

- runtime `duration_seconds` is optional, while the schema describes a number,
- runtime `vacuum.file` is boolean, while the schema describes another type,
- target types differ across status, client, and server,
- strict objects are declared without a complete required/optional model,
- the schema is manually maintained rather than derived from wire types.

### Why tests missed it

Client message construction and server message parsing have separate unit tests. They do not run the produced message through the actual server parser.

### Correct design

Define one typed protocol crate:

```rust
enum Request {
    Status,
    Tap(TapOptions),
    Invoke(InvokeRequest),
}

enum ActionId { ... }

struct InvokeRequest {
    action: ActionId,
    args: ActionArgs,
    origin: Origin,
    correlation_id: String,
}
```

Derive:

- serde wire format,
- JSON Schema,
- CLI parsing/help,
- documentation tables,
- validation,
- error repair examples.

At minimum, move both client and server to shared request/response structs and add round-trip tests.

### Acceptance tests

For every action:

1. parse CLI,
2. serialize request,
3. deserialize server-side,
4. validate action,
5. serialize response,
6. validate response against generated schema.

No hand-written “fix” string should be allowed unless a test invokes it successfully.

---

## P0 — `Shell` is a cross-feature state machine without explicit state machines

**Type:** architectural risk supported by shipped regression history  
**Area:** UX, maintainability

The bug ledger repeatedly shows the same failure pattern:

- a gesture has more than one owner,
- a transition has more than one timer,
- persisted state and live state differ,
- a popup action is queued but the popup closes before it is drained,
- fullscreen and mini mode race,
- keyboard Escape means different things depending on transient state,
- target selection and actual beam source drift,
- CPU renderer state is initialized and then overwritten by stale style.

This is not random bad luck. It is a structural symptom.

### Correct design

Extract explicit state machines:

- `WindowModeState`: Normal, Mini, Fullscreen, Transitioning
- `BeamSourceState`: Silent, Capture(target), Mix(members), File(path), ExternalPlayer
- `PlaybackState`: Stopped, Loading, Playing, Paused, Draining, Error
- `PopupState`: Closed, Opening, Open, Committing(action), Closing
- `ExportState`: Idle, SnapshotPending, ClipPending, Rendering, Failed
- `AgentTransactionState`: accepted, applied, visually acknowledged, completed

Each transition should be a pure reducer where possible:

```text
(old_state, event) -> (new_state, effects)
```

Effects execute afterward. This makes race ownership visible and testable.

### Refactor target

Keep the Winit application handler in `shell.rs`, but move domain transitions out. The final shell should primarily:

- receive platform events,
- dispatch domain events,
- execute returned effects,
- render the current view model.

---

# 4. Performance and accuracy recommendations

## 4.1 Preserve the renderer; optimize around it

The GPU and CPU beam deposition work is one of the best parts of the project. Do not replace it with generic line rendering.

Keep:

- analytic line-integrated Gaussian deposition,
- additive energy layers,
- shared constants,
- exact supersample box averaging,
- GPU timestamp queries,
- CPU row-band ownership and SIMD.

The likely wins are outside the core formula.

## 4.2 Reuse CPU renderer scratch storage

`CpuRenderer::advance` allocates:

- a new `Vec<PreparedSegment>`,
- a new `Vec<Vec<u32>>` for row bins,
- each inner bin’s growth.

At high frame rates this produces allocator traffic unrelated to actual beam work.

Store reusable fields on `CpuRenderer`:

```text
prepared_segments
band_offsets
band_indices
```

Prefer a flat compressed bin representation:

1. count segment references per band,
2. prefix sum,
3. fill one flat `Vec<u32>`.

This avoids dozens of small vectors and improves locality.

Acceptance: zero allocations in `advance()` after warm-up at a stable resolution and segment ceiling.

## 4.3 Stop front-draining waveform and audio vectors

`Vec::drain(..n)` moves the tail. It appears in:

- waveform history,
- upsampler tail,
- sample pending/history,
- resampler channel input,
- mix member buffers.

Replace long-lived rolling histories with:

- circular buffers,
- a read cursor plus occasional compaction outside hot paths,
- or `VecDeque` only where bulk contiguous slices remain manageable.

For DSP histories used by FFT/trigger windows, a ring with a method that materializes only the requested contiguous analysis window is preferable.

## 4.4 Make DSP consume bounded audio quanta

The agent guide acknowledges that geometry arrives in roughly 20 ms bursts with empty frames between. That is an implementation artifact, not an immutable law of physics.

The UI currently drains however much is pending. This couples segment count and visual deposit bursts to event-loop timing.

Introduce a signal clock:

- fixed audio quantum, e.g. 2–5 ms,
- process all available quanta up to a catch-up budget,
- drop stale quanta explicitly after the budget,
- deposit each quantum at its timestamp,
- present independently.

This improves:

- smoothness,
- reproducibility,
- tap usefulness,
- auto-gain behavior,
- CPU load distribution,
- frame-time tails.

## 4.5 Improve mix correctness

Current multi-app mixing:

- drains each member whenever the shell asks,
- extends to the longest buffer,
- sums by vector index,
- zero-pads missing members,
- has no block timestamp,
- has no per-source gain,
- does not normalize or provide headroom.

This preserves each member’s L/R pairing but does not guarantee streams are aligned to the same graph time. It can also clip or radically change brightness as sources are added.

Use timestamped blocks from the PipeWire graph. Align by graph position, then choose a documented mix law:

- raw sum with limiter/headroom,
- equal-power normalization,
- or explicit per-source gain.

Expose the law in UI and probe.

## 4.6 Correct multichannel file downmix

For decoded content with more than two channels, the player currently takes the first two channels. The source comment acknowledges that v3 used ffmpeg downmixing.

For 5.1/7.1 content, taking the front pair discards center, surrounds, and LFE rather than producing a stereo representation.

Implement a channel-layout-aware downmix using Symphonia’s channel map. A safe matrix should:

- preserve FL/FR,
- fold center with an appropriate coefficient,
- fold surrounds with an appropriate coefficient,
- handle LFE according to a documented policy,
- prevent overload.

Add fixtures for mono, stereo, 5.1, and unknown layouts.

## 4.7 Set and validate capture channel positions

Playback explicitly declares FL and FR channel positions. Capture requests F32LE stereo but does not show the same position declaration.

Do not rely on implicit ordering across every PipeWire converter. Set positions and record the negotiated format. Surface it in diagnostics.

## 4.8 Reuse the actual surface adapter

When rebuilding the GPU renderer, the shell creates a new WGPU instance, requests a new high-performance adapter, and then builds with the existing graphics device and queue.

On a multi-GPU machine, the newly selected adapter can differ from the adapter that created the device. Even when it does not fail, its format feature probe may describe the wrong hardware.

Use `graphics.adapter` directly. Do not block the event loop with a fresh adapter request for a settings change.

## 4.9 Separate “physics quality” from “display supersampling”

The current options combine high sample rates, DSP oversampling, renderer supersampling, CPU resolution scaling, and presentation FPS. Users can spend large amounts of compute without knowing which artifact each setting addresses.

Present an expert panel with measurable dimensions:

- capture/feed rate,
- trace interpolation factor,
- beam raster supersample,
- presentation FPS,
- CPU render scale.

Add presets that are based on bottleneck:

- Accurate trace
- Smooth beam
- Low power
- Export quality

The HUD should show which stage is limiting.

## 4.10 Treat 3,400 FPS as a microbenchmark, not a product target

Uncapped throughput is useful evidence that the old GTK frame clock was removed. It is not the right live objective.

Optimize for:

- frame-time p99,
- audio overruns,
- latency from graph sample to visible deposit,
- stable power consumption,
- no change in decay law across frame rates,
- no visual burst after a stall.

---

# 5. Dead code and dead state

## 5.1 `precompute_enabled`

**Status:** verified semantic dead state

The settings model owns, loads, saves, and preserves `precompute_enabled`. Repository search found no production consumer outside settings. The README says render-ahead precompute was retired by design.

### Why it likely shipped

Migration compatibility was prioritized: v4 preserved v3 settings keys so running versions would not erase each other’s data. That is reasonable during transition, but the field remained typed as an active setting rather than quarantined legacy data.

### Fix

Move it to a documented legacy passthrough set or remove it after a settings migration version. Do not expose it as first-class application state.

## 5.2 `phosphor-studio`

**Status:** verified stub crate

The crate contains a detailed future-facing module comment and two empty modules. The CLI explicitly marks `studio` pending.

### Why it likely shipped

The workspace was structured around a planned wave-based rewrite, and the placeholder made the intended architecture visible.

### Risk

A workspace member looks more implemented than it is; tooling, dependency graphs, and agents may infer capability from its presence.

### Fix

Either:

- remove it from workspace members until implementation begins,
- or make the crate intentionally explicit with a `README`, feature flag, and compile-time status type.

## 5.3 Pending command shells

`studio` and `--screensaver` are recognized but return “not built.” This is preferable to silent success, but they are still product-surface debt.

Keep pending commands only when compatibility requires reservation. Otherwise, document them in roadmap rather than executable help.

## 5.4 Stale `#[allow(dead_code)]` fields/variants

A stale `BeamSource::Mix` member field is suppressed rather than removed. A metadata proxy field is also marked dead-code despite side-effect ownership.

Not every suppressed field is useless—some exist to keep a resource alive—but those should use names such as `_proxy_guard` or a wrapper type. `allow(dead_code)` obscures whether a field is an ownership guard, future work, or forgotten state.

## 5.5 Historical spikes and examples

Examples and design documents are not dead code if they remain deliberate test rigs. Mark their status:

- supported example,
- diagnostic probe,
- historical spike,
- obsolete and retained for archaeology.

Move obsolete spikes under an `attic/` or git tag instead of letting agents treat them as current architecture.

## 5.6 Documentation duplication

The handoff file itself contains duplicated narrative blocks. This is not runtime dead code, but it demonstrates the same copy-maintenance problem as the agent schema.

Generate status sections where possible; keep one source of truth for version, feature completion, and known defects.

---

# 6. Why dead code and regressions shipped

The source gives a coherent answer.

## 6.1 The rewrite was parity-driven, then feature-driven

The project did an ambitious full-Rust rewrite while preserving v3 behavior. Compatibility fields, future-wave stubs, and “law” comments were intentionally carried forward.

This is why some dead state is understandable rather than careless.

## 6.2 Tests often pin components, not boundaries

Examples:

- client CLI mapping is unit-tested,
- server parsing is unit-tested,
- but their actual serialized handshake is not.
- menu geometry was receipted,
- but a real menu item was not clicked.
- renderer creation was correct,
- but a later style overwrite broke the visible result.

## 6.3 The application has too many duplicated representations

Duplicated representations include:

- UI action vs control verb,
- CLI parser vs socket parser,
- runtime status structs vs manually written schema,
- settings struct vs owned-key list vs loader vs serializer,
- persisted desired source vs actual beam source,
- renderer defaults vs copied style.

Every duplicate is a drift surface.

## 6.4 The release process appears optimized for momentum

The repository openly celebrates a rapid rewrite and maintains a paid-for bug ledger. That speed produced impressive scope, but the ledger shows integration defects arriving in clusters.

The prevention mechanism is not “write more comments.” It is to reduce the number of facts that can disagree.

---

# 7. What should be rewritten

## Rewrite 1: shared action and protocol layer

Replace separate `UiAction`, `ControlVerb`, MPRIS mappings, CLI verb tables, and schema tables with one action registry.

An action descriptor should include:

```text
id
label
description
argument schema
current value
enabled/disabled reason
danger level
reversibility
UI widget id
keyboard binding
MPRIS mapping, if any
agent visibility
```

Both human UI and agent interface invoke the same domain action.

## Rewrite 2: audio transport internals

Keep PipeWire and the high-level feature set. Rewrite only the buffering/clocking layer:

- lock-free block rings,
- timestamps,
- bounded memory,
- bulk operations,
- worker-owned history,
- explicit drop policy,
- xrun metrics.

## Rewrite 3: shell state architecture

Do not rewrite Winit/egui. Extract reducers and effect handlers from the giant shell.

Suggested modules:

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

## Rewrite 4: settings representation

Use one serde-derived settings struct with:

- `#[serde(default)]`,
- validation layer,
- migration version,
- `flatten` for unknown compatibility keys where still needed,
- generated JSON schema,
- atomic save.

Current hand-written load, owned-key list, and serialization repeat the field set multiple times.

Also write settings atomically:

1. write temporary file,
2. `sync_all` if appropriate,
3. rename.

A crash during `std::fs::write` can otherwise leave malformed JSON, and malformed JSON currently resets silently to defaults.

## Rewrite 5: platform capability layer

Encapsulate:

- work area,
- popup behavior,
- always-on-top,
- transparency,
- window positioning,
- X11 override redirect,
- Wayland limitations.

Expose capabilities to UI and agent probe. A generic Linux claim should not hide X11-only behavior.

## Do not rewrite

Do not replace:

- the DSP mode implementations en masse,
- the shared beam model,
- WGPU deposition,
- CPU SIMD rasterization,
- `.phos` / `.phoskit` formats without a compatibility reason.

Those are comparatively well-factored and tested.

---

# 8. Where user behavior falls apart

## 8.1 Escape is dangerous and context-dependent

Documentation says Escape walks a cascade and can quit a normal window. This violates a common user expectation that Escape dismisses a transient surface.

Recommendation:

- Escape closes active popup/dialog first.
- Then exits compose/fullscreen/mini.
- It should never quit the main app.
- Use Ctrl+Q or an explicit menu action for quit.

## 8.2 X11 behavior is presented as generic Linux behavior

Mini snapping shells out to `xprop`, and the context menu uses X11 popup/override-redirect APIs. On Wayland, these paths can be unavailable or behave differently.

Recommendation:

- Detect backend.
- Show unsupported capability rather than silently doing nothing.
- Implement a Wayland-safe in-window menu fallback.
- Use monitor APIs or a backend abstraction for work area.

## 8.3 Hidden delayed settings persistence

Some settings save immediately; others wait for clean shutdown. Users cannot predict which changes survive a crash.

Recommendation:

- debounce all user settings to an atomic save,
- expose a small “saved” state only if necessary,
- separate session-only state from persistent preferences.

## 8.4 Source preference and live source are different concepts

The code has already had “ghost selection” bugs where a remembered target appeared selected while a file actually fed the beam.

The newer `BeamSource` model is the right direction. Finish it:

- show “currently feeding” prominently,
- show remembered fallback separately,
- expose both to agents,
- make transitions explicit when playback ends or a target disappears.

## 8.5 Mix is powerful but underspecified

“Mix several apps” is novel, but the user does not see:

- per-app level,
- timing alignment,
- clipping/headroom,
- which member disappeared,
- whether a paused member remains selected.

Recommendation: a compact mixer with meters, mute/remove, per-source gain, and status.

## 8.6 Quality settings are implementation-named

`gl_supersample`, `cairo_resolution`, and renderer labels preserve v3 implementation vocabulary even though v4 is WGPU/SIMD.

Recommendation: migrate display names to user meaning while keeping legacy keys internally.

## 8.7 Background mode is not truly headless

`--background` launches a full GUI under Xvfb at a fixed 1280×800. It is useful, but it is a virtual-display mode, not a true headless service.

Recommendation:

- rename/document it as virtual-display mode,
- add `--headless` backed by an offscreen renderer and control service,
- make resolution explicit,
- report backend and dimensions in `probe`.

## 8.8 Long operations can block interaction

Snapshot/clip deliberately defer replies, but audio stop joins a player thread and vacuum operations wait on subprocess/graph timeouts. Ensure none of these block the Winit event thread.

Use asynchronous effects and progress state.

---

# 9. The smallest, truest missing feature

## “Action Lens”: a semantic UI mirror with visible agent actions

The missing feature is not a twelfth visualization mode.

It is a way for a human and an agent to inhabit the same application state.

Today the CLI can operate a useful slice of Phosphor, but it does not know the visible interface, active dialog, enabled controls, widget bounds, or whether a command produced the intended visual result.

### Minimal product

Add:

```text
phosphor observe
phosphor invoke <action-id> [args]
phosphor watch
```

`observe` returns:

- window/backend/size,
- active view and dialogs,
- semantic control tree,
- stable widget IDs,
- role, label, value, enabled state, disabled reason,
- bounds in logical and physical pixels,
- current beam/source/player state,
- available actions in the current state,
- optional low-resolution screenshot or frame hash.

`invoke` dispatches the exact same action as the corresponding human widget.

`watch` streams:

- state diffs,
- action accepted/applied/completed events,
- dialog changes,
- toasts/errors,
- optional beam and screenshot revisions.

### Human-visible feedback

When an agent invokes an action:

- pulse/highlight the actual widget for 500–900 ms,
- optionally animate a pointer to it,
- show a small toast such as “Agent: Glow 70% → 82%,”
- include a correlation ID in both event stream and toast,
- do not steal focus unless the action explicitly requests it.

The actual operation should be semantic, not coordinate clicking. The animation is evidence for the human, not the mechanism.

### Why this fits Phosphor

Phosphor already has:

- an action queue,
- a control socket,
- probe state,
- tap streams,
- deferred replies,
- a visual instrument identity,
- a desire to be agent-operable.

Action Lens completes the philosophy rather than adding an unrelated feature.

---

# 10. What the CLI actually covers

## 10.1 Covered today

### State and observation

- one-shot live status
- current mode/theme/UI style
- capture state and source summary
- player metadata, position, duration, pause
- volume and gain summary
- kit enabled/path
- mini/fullscreen summary
- vacuum summary
- FPS
- color-cycle summary
- beam geometry stream: segment count, bounds, centroid, peak, reduced polyline
- applet feed
- schema document

### Mutations

- play, pause, toggle, stop
- next, previous
- seek
- playback volume
- display mode
- phosphor theme
- chrome style
- capture on/off
- source target, including a string-encoded mix target
- open file
- raise/focus
- snapshot
- clip
- quit
- validate and inspect kits

This is a respectable instrument-control surface.

## 10.2 Not covered

### Signal and beam tuning

- gain
- auto gain
- persistence/glow
- beam energy
- beam focus
- grid
- custom beam/grid colors
- AMOLED
- glass and tint
- color-cycle configuration

### Rendering and timing

- GPU/CPU selection
- GPU supersample
- CPU resolution
- scope sample rate
- max FPS
- HUD modes

### Window and UI

- mini toggle and mini size
- fullscreen
- pin
- alignment
- settings/manual panels
- active popup/dialog
- focus ownership
- semantic widget tree
- visual acknowledgement

### Playback library behavior

- shuffle
- repeat
- playlist list/select/remove/reorder
- notification settings
- now-playing visibility
- gapless-next state

### Creative features

- compose mode
- drawing points/strokes
- drawing playback/export
- postcard creation
- kit load/enable/edit/save
- vacuum controls
- 3D camera orbit/dolly

### Discovery

- complete current target list with stable IDs and capabilities
- available mix members
- per-source levels/status
- allowed actions in current state
- backend limitations
- pending operations and progress

## 10.3 Can an agent realistically execute the intended signal direction?

For a narrow task such as “scope Spotify, select XY45, take a snapshot,” yes—after target discovery is solved externally.

For “understand everything visible and operate the application like a human,” no.

The geometry tap can tell an agent that a trace occupies a bounding box. It cannot reliably answer:

- whether the beam color is correct,
- whether the grid is visible,
- whether a dialog blocks the UI,
- whether the CPU renderer visually matches the GPU,
- whether a menu item is clipped,
- whether a button appears pressed,
- whether a notification or warning appeared,
- whether a setting changed but was later overwritten.

A ≤64-point polyline is useful telemetry, not visual equivalence.

## 10.4 Context presented falsely or suboptimally

1. **“No pixels needed” is too broad.**  
   Pixels are unnecessary for a subset of signal-control tasks, not for UX verification.

2. **The schema appears authoritative but is manually duplicated and already inconsistent.**

3. **A tap stream can end with exit code 0 after socket EOF.**  
   Clean consumer shutdown and server disappearance are not distinguished.

4. **The hello event is emitted by the client.**  
   It proves the client started, not that the server negotiated the claimed schema/version.

5. **Socket auto-selection is ambiguous with multiple instances.**  
   The client tries the default then newest per-PID sockets. It has no explicit instance identity or selector.

6. **Control-socket bind failure is nonfatal.**  
   The GUI can be running without the advertised agent surface. Probe should expose “control unavailable” through another discoverable channel, or startup should make the degraded state visible.

7. **Background mode is Xvfb, not pixel-free headless operation.**

## 10.5 Required protocol additions

- server-emitted handshake with protocol version and instance ID,
- explicit `--instance`,
- generated schema,
- `capabilities`,
- `actions`,
- `observe`,
- `invoke`,
- `watch`,
- target discovery,
- operation progress,
- visual revision/frame hash,
- state revision numbers,
- idempotency/correlation IDs,
- prepare/commit for risky actions,
- structured disabled reasons,
- end-of-stream reason and nonzero exit on server loss.

---

# 11. Least-confident areas, ranked

## 1. Window manager and transient UI state

**Confidence: lowest**

Reasons:

- repeated mini/fullscreen/menu/focus regressions,
- X11-specific popup and work-area behavior,
- multiple delayed transitions,
- a very large shell,
- platform behavior is hard to cover with unit tests.

## 2. Real-time audio, playback, mix, and vacuum

Reasons:

- locks and memory movement in callbacks,
- external `pactl` lifecycle,
- WirePlumber policy interactions,
- ambiguous mix timing,
- channel-layout downmix issue,
- shutdown and restore paths are difficult to test exhaustively.

## 3. Agent protocol and schema

Reasons:

- verified client/server mismatch,
- verified repair-string mismatch,
- manually authored schema drift,
- missing end-to-end contract tests,
- ambiguous multi-instance selection.

## 4. Live/offline visual equivalence

Reasons:

- frame-dependent decay,
- different targets and color-encoding paths,
- separate scheduling,
- “byte exact” is stronger than the architecture can generally guarantee across adapters and presentation rates.

## 5. CPU renderer lifecycle and scheduling

Reasons:

- recent visible restyle regression,
- full-plane decay/composite every frame,
- repeated scratch allocation,
- renderer recreation path probes a potentially unrelated adapter.

## 6. Source and mix truth

Reasons:

- persisted target vs actual source has already diverged,
- mix has no timestamp alignment,
- member disappearance and fallback semantics need stronger modeling.

## 7. Packaging, distribution, and platform support

Reasons:

- package scripts are hand-maintained,
- embedded asset notices are incomplete,
- platform claims exceed clearly abstracted backend support,
- no verified CI result was associated with the audited commit through the available connector.

## 8. DSP geometry and beam deposition

**Confidence: highest**

Reasons:

- shared implementation,
- golden fixtures,
- explicit numeric doctrine,
- property tests,
- CPU/GPU common model.

The time-domain decay issue still belongs above this layer.

---

# 12. Accreditation and third-party notices

## 12.1 Verified current credit surface

The README names:

- IBM Plex Sans,
- JetBrains Mono,
- Jerobeam Fenderson / Oscilloscope Music,
- Claude as a development collaborator.

The application embeds:

- IBM Plex font data,
- JetBrains Mono font data,
- the `egui-phosphor` icon font.

The Debian package script installs only the project GPL license as `/usr/share/doc/phosphor/copyright`.

## 12.2 Definite notice gaps

### IBM Plex

IBM Plex’s upstream license identifies IBM as copyright holder, reserves the name “Plex,” and states that distributed copies must contain the copyright notice and OFL license.

A README phrase “IBM Plex … (OFL)” is good credit, but the package should include the full license and notice in a user-viewable place.

### JetBrains Mono

JetBrains Mono’s upstream OFL file identifies the project authors and likewise requires the copyright notice and license to accompany distributed copies.

### Phosphor Icons

The upstream Phosphor Icons core project is MIT-licensed and requires the copyright and permission notice to be included in copies or substantial portions.

The project uses `egui-phosphor` and embeds its font, but the README’s credit section does not name Phosphor Icons.

## 12.3 Recommended solution

Add:

```text
THIRD_PARTY_NOTICES.md
licenses/
  IBM-Plex-OFL-1.1.txt
  JetBrains-Mono-OFL-1.1.txt
  Phosphor-Icons-MIT.txt
```

Include these in:

- source release,
- `.deb`,
- `.rpm`,
- in-app About/Credits page.

Generate dependency notices in CI using a Cargo-aware license tool. Do not manually list every transitive dependency in prose. Generate an auditable report and fail CI on unknown or disallowed licenses.

Also generate an SPDX or CycloneDX SBOM for releases.

## 12.4 What was not found

No verified evidence was found in this static pass that Phosphor copied uncredited source from another vectorscope project.

The strongest identifiable lineage is:

- its own v3 Python/Rust implementation,
- upstream Rust crates,
- standard Abramowitz–Stegun error-function approximation, which is credited in code,
- oscilloscope music culture, which is credited in the README.

Do not make plagiarism claims from similar concepts or constants alone.

## 12.5 Scientific/marketing attribution

The phrase “real P7 CRT” is stronger than the evidence in the code. The canonical physics source is described as the project’s previous shader, not a cited measurement or phosphor datasheet.

Safer wording until calibrated:

> CRT-inspired two-layer phosphor model with P7-style flash and persistence.

To retain “real P7,” add:

- cited phosphor decay data,
- measured target half-lives,
- color/chromaticity assumptions,
- calibration method,
- validation plots.

---

# 13. What I would have done differently from the beginning

1. **Define the domain action protocol before building three front ends.**  
   UI, keyboard, MPRIS, and CLI would all invoke the same typed actions.

2. **Make simulation time independent from presentation time.**  
   Samples and timestamps would advance the instrument; frame rate would only decide how often it is shown.

3. **Use lock-free audio block transport from day one.**  
   Export history, scope feed, and audible output would branch outside RT callbacks.

4. **Use explicit state machines for window and playback modes.**  
   No loose collection of booleans and delayed timers would be allowed to encode mutually exclusive modes.

5. **Build platform capability boundaries early.**  
   X11, Wayland, transparency, work area, popup windows, and focus rules would be separate adapters.

6. **Generate schemas, settings serialization, help, and docs.**  
   Any fact copied into three files would be treated as a design smell.

7. **Make gesture receipts true end-to-end tests.**  
   “Open menu and click item” rather than “menu rectangle exists.”

8. **Ship a smaller v4 core before the long feature wave.**  
   Capture, XY, renderer, settings, and export first; then player, MPRIS, mini, vacuum, kits, compose, and agent features in independently hardened increments.

9. **Include third-party notices and SBOM automation in the first package.**

10. **Use the bug ledger as input to architectural refactors.**  
    Once three bugs share a root pattern, stop patching symptoms and replace the ownership model.

---

# 14. Implementation roadmap for another agent

## Phase 0 — Contract lock and measurements

### Goal

Create a reproducible baseline before changing behavior.

### Tasks

- Record current golden, property, and cross-render test results.
- Add an end-to-end CLI/socket harness.
- Add allocation counters to DSP, CPU render, and audio callbacks.
- Add audio overrun/underrun counters.
- Add frame-to-visible latency measurement.
- Add decay half-life measurement at multiple FPS.
- Add X11 and Wayland capability probes.
- Inventory embedded assets and licenses.

### Exit criteria

- One command produces a machine-readable baseline report.
- Every current CLI action is exercised through the actual socket server.
- Any allocation or mutex in an RT callback is observable.
- Current decay inconsistency is captured as a failing test.

---

## Phase 1 — Shared typed protocol

### Goal

Remove protocol duplication without changing product behavior.

### Tasks

- Add `phosphor-control-proto` or place shared request/response types in `phosphor-proto`.
- Define typed action IDs and arguments.
- Make CLI serialize shared types.
- Make server deserialize shared types.
- Derive JSON Schema.
- Generate help and repair examples.
- Add protocol and instance version negotiation.
- Add explicit instance selection.

### Exit criteria

- Numeric/string target mismatch cannot compile or cannot pass validation.
- Every repair example is executable in a test.
- Runtime responses validate against generated schemas.
- Documentation tables are generated.

---

## Phase 2 — Time-correct beam simulation

### Goal

Make persistence and export independent of presentation FPS.

### Tasks

- Introduce `SimulationTime`.
- Convert flash/glow retention to time constants.
- Decide fixed-tick or subdivided variable-dt policy.
- Pass time delta to both renderers.
- Advance offline render by media timestamps.
- Update golden tests intentionally with a migration note.
- Add half-life and cross-FPS invariants.

### Exit criteria

- Decay at 30/60/144/240 FPS agrees within defined tolerance.
- A stall does not cause a burst or alter elapsed-time decay.
- Live/offline frames agree at equal timestamps on the reference adapter.

---

## Phase 3 — Real-time audio transport

### Goal

Remove locks, allocation, and front drains from PipeWire callbacks.

### Tasks

- Build preallocated SPSC block rings.
- Attach sequence and graph timestamp.
- Move history copies to a worker.
- Bulk-convert mapped F32LE buffers.
- Replace playback `Mutex<VecDeque>` with SPSC.
- Expose xrun/drop counters.
- Set capture channel positions.
- Implement channel-layout-aware file downmix.
- Timestamp-align mix members and add headroom policy.

### Exit criteria

- Zero callback allocations after stream start.
- Zero callback mutexes.
- Stress test records bounded drops rather than blocking.
- 5.1 fixture downmixes deterministically.
- Mix alignment test passes with deliberately skewed producers.

---

## Phase 4 — Shell decomposition

### Goal

Stop integration regressions from emerging from implicit state.

### Tasks

- Extract domain `AppState`.
- Convert `UiAction` to shared domain actions.
- Add reducers for window/source/playback/popup/export.
- Make effects asynchronous and cancellable.
- Move persistence to a debounced atomic writer.
- Replace platform-specific calls with capability services.

### Exit criteria

- Shell is primarily event adaptation and rendering.
- Impossible combinations such as mini+fullscreen cannot be represented.
- Popup action is committed before close by state-machine law.
- Escape behavior has one tested precedence table.
- Wayland unsupported features are explicit.

---

## Phase 5 — Action Lens

### Goal

Give agents human-equivalent semantic access with human-visible feedback.

### Tasks

- Assign stable semantic IDs to controls.
- Produce a UI/control tree each frame or on revision.
- Register bounds and action IDs during egui layout.
- Add `observe`, `invoke`, and `watch`.
- Add action correlation IDs and revisions.
- Highlight invoked widget and show agent toast.
- Add optional screenshot/thumbnail endpoint.
- Add dialog and disabled-reason reporting.

### Exit criteria

An agent can:

1. discover a control,
2. know whether it is enabled,
3. invoke the same domain action as a click,
4. observe completion,
5. inspect the changed state,
6. obtain visual evidence,
7. while the human sees exactly what was changed.

---

## Phase 6 — Release hardening

### Tasks

- Add generated third-party notices.
- Add SBOM.
- Include notices in packages.
- Add CI gates for format, clippy, tests, protocol round trips, license policy, and package inspection.
- Add separate X11/Wayland smoke jobs where feasible.
- Run package install/uninstall tests in clean containers.
- Make benchmark claims include hardware, driver, command, distribution, and commit.

---

# 15. Concrete first pull requests

## PR 1 — Fix agent contract contradictions

Small, low-risk, immediately valuable.

- Make target ID one canonical type.
- Correct all server repair strings to positional CLI syntax.
- Correct schema field types.
- Add client→server parser round-trip tests.
- Add server-emitted handshake.
- Return nonzero/end reason when tap loses the server.

## PR 2 — Remove semantic dead state

- Move `precompute_enabled` into legacy passthrough or migrate it out.
- Mark `phosphor-studio` explicitly experimental or remove it from active workspace.
- Replace ambiguous `allow(dead_code)` with ownership-guard naming.
- Deduplicate handoff status text.

## PR 3 — Time-domain decay test

Before implementation, add a failing test proving current half-life changes with FPS. Then change the beam API to accept `dt`.

## PR 4 — Audio callback instrumentation

Add counters and a debug gate that detects:

- allocation,
- callback duration,
- lock wait,
- overrun/underrun,
- block sequence gaps.

Do this before the ring rewrite so improvement is measurable.

## PR 5 — Use the existing WGPU adapter on renderer rebuild

Replace the new instance/adapter request with the surface’s existing adapter and make rebuild asynchronous or immediate without blocking.

---

# 16. Agent execution checklist

An implementation agent should follow this order for every change:

1. Read `docs/dev/BUGLOG.md`.
2. Identify the user gesture or agent command that exposes the issue.
3. Name the single source of truth that should own the behavior.
4. Add an end-to-end receipt before changing code.
5. Avoid adding another representation of the same fact.
6. Make state transition and side effects separate.
7. Preserve file-format and DSP golden compatibility unless the issue explicitly changes the physical model.
8. Measure frame-time distribution, not only average FPS.
9. For audio code, assume callbacks are hard real-time.
10. For agent code, run the exact repair instruction returned by errors.
11. Test X11 and Wayland behavior or expose a capability limitation.
12. Update third-party notices when embedding new assets.
13. Add a buglog entry only after the root cause and prevention law are clear.

---

# Final narrative

Phosphor’s identity is already present. It does not need more surface area to become compelling.

It needs its existing truths to become impossible to disagree about:

- one time model,
- one action model,
- one protocol model,
- one source-of-beam model,
- one settings model,
- one platform capability model.

The renderer is not the project’s weak link. The project’s weak link is translation between subsystems.

Fixing those translations would make Phosphor simultaneously:

- more accurate as an instrument,
- smoother under load,
- easier to maintain,
- less surprising to users,
- honestly agent-operable,
- and safer to extend without another wave of integration regressions.

That is the smallest path to making the project feel finished rather than merely feature-rich.
