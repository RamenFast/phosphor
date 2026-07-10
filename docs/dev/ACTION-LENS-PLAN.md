# ACTION LENS — implementation plan (planned 2026-07-10, NOT implemented this arc)

Audit §9 ("the smallest, truest missing feature"), audit Phase 5, audit §10.5,
best-practices §12. Authored under the fable-to-opus contract: a zero-context
executor session must be able to run this plan with no additional exploration.

**Symbol legend** (fable-to-opus): ▸ task · 📁 file · ✅ verify · ↩ rollback ·
⛔ constraint · ⚠ gotcha · ❓ decision-for-Ben.

**Status: PLAN ONLY.** The prerequisite (audit PR1, the shared typed protocol)
landed this arc as commit `1ec2f6c`. Everything below builds on that commit's
real types. Nothing in this file is implemented.

---

## 0. What exists today (the real foundation — verified at 1ec2f6c)

- 📁 `crates/phosphor-app/src/protocol.rs` — the ONE typed wire module:
  - `CtlRequest` (typed verb enum, 19 verbs, `protocol.rs:73-91`)
  - `TargetId::Node(i64) | Name(String)` (`protocol.rs:23`)
  - `from_cli` / `to_wire` / `from_wire_args` — CLI text ↔ typed ↔ wire by
    construction (`protocol.rs:176,252`)
  - `WireError` with executable fix strings (test-enforced re-parseable)
  - `hello_event` / `end_event` (`protocol.rs:349,363`), `PROTOCOL_VERSION: u32 = 1` (`protocol.rs:17`)
  - `CtlRequest::EXAMPLES` (`protocol.rs:123`) — driven through a REAL
    UnixListener + real `handle_connection` in
    `every_verb_round_trips_over_a_real_socket` (control.rs tests)
- 📁 `crates/phosphor-app/src/control.rs` — server: `ControlVerb` (`:32`),
  `ControlRequest { verb, reply }` (one-shot reply channel), `StatusSnapshot`
  (`:62`, the probe wire contract), `TapEvent`, `handle_connection` (takes
  socket path + wake closure — already testable without winit).
- 📁 `crates/phosphor-app/src/agent.rs` — client: `build_ctl_message` is a
  thin wrapper over `CtlRequest`; `connect_to` honors `--socket` +
  `PHOSPHOR_CTL_SOCKET`; tap emits `end` events and exits 4 on server loss.
- 📁 `crates/phosphor-app/src/shell.rs` — `UiAction` (`:218`, ~30 deferred UI
  intents drained by `frame()`), `BeamSource` (`:262`, single source of truth
  for the beam).
- 📁 `crates/phosphor-app/src/mpris.rs` — `MprisCommand` (`:34`), still a
  separate transport payload (audit noted; NOT yet unified).
- 📁 `crates/phosphor-app/src/chrome.rs` / `keyboard.rs` — egui chrome and
  keybindings, each pushing `UiAction`s.

Today there are FOUR ways to express "do a thing": `UiAction` (chrome/keys),
`ControlVerb` (socket), `MprisCommand` (D-Bus), `CtlRequest` (CLI/wire). They
converge only informally. Action Lens makes the convergence a data structure.

⛔ Standing laws (ARC-BRIEF): never touch Ben's live ctl.sock; isolate test
instances (own `XDG_RUNTIME_DIR`, `PIPEWIRE_RUNTIME_DIR=/run/user/1000`,
`PHOSPHOR_NO_SINGLE_INSTANCE=1` / `--background`); UI receipts at 2560×1440
exercising the actual gesture; fmt/clippy/test gates before every commit;
time is seconds; one fact one representation; applet feed v3 frozen.

---

## 1. Product shape (audit §9 minimal product, verbatim scope)

```text
phosphor observe                    # semantic snapshot: window, dialogs, control tree, actions
phosphor invoke <action-id> [args]  # dispatch the SAME domain action as the human widget
phosphor watch                      # NDJSON stream: revisions, receipts, dialogs, toasts, errors
```

The exit bar is the audit Phase-5 7-point list. An agent can:

1. discover a control,
2. know whether it is enabled,
3. invoke the same domain action as a click,
4. observe completion,
5. inspect the changed state,
6. obtain visual evidence,
7. while the human sees exactly what was changed.

Semantic operation is the mechanism; any highlight animation is evidence for
the human, never the mechanism. Coordinate clicking is at most a debug
fallback (best-practices §12.5) and is out of scope for this plan.

---

## 2. Core data model

### 2.1 `ActionDescriptor` (best-practices §12.3, field-for-field)

📁 NEW `crates/phosphor-app/src/lens/registry.rs`

```rust
#[derive(Clone, serde::Serialize)]
pub(crate) struct ActionDescriptor {
    pub id: &'static str,            // "player.toggle", "beam.mode", "capture.on"
    pub label: &'static str,         // human label as shown in chrome
    pub description: &'static str,
    pub args: ArgSchema,             // None | Number{min,max,unit} | Enum{values} | Path | Text | Bool
    pub current_value: Option<serde_json::Value>, // filled per-observe from Shell state
    pub enabled: bool,
    pub disabled_reason: Option<String>,          // structured: {code, message}
    pub danger: Danger,              // Safe | Disruptive | Destructive (quit, vacuum, overwrite)
    pub reversibility: Reversibility,// Reversible | ReversibleWithUndo | Irreversible
    pub ui_id: Option<&'static str>, // semantic widget id in the tree ("toolbar.mode")
    pub keybinding: Option<&'static str>,   // "Space", "Ctrl+M" — from keyboard.rs table
    pub mpris: Option<&'static str>, // "PlayPause", "Next" — MprisCommand name
    pub agent_visible: bool,         // false for pure-chrome cosmetics if any
}
```

⛔ One fact, one representation: the registry is a `const`/static TABLE and
every other surface derives from it:

- `CtlRequest` verbs ↔ registry ids: a test walks both directions (mirror of
  the existing `schema_ctl_verbs_match_the_shared_protocol_exactly` pattern).
- `UiAction` variants map to registry ids via a single
  `fn action_id(&self) -> Option<&'static str>` on `UiAction`; a test asserts
  every registry entry with `ui_id` has a `UiAction` producer.
- `MprisCommand` mapping asserted the same way.
- `keyboard.rs` bindings asserted against `keybinding` fields.

This is how "UI, keyboard, MPRIS, and CLI invoke the same action ID"
(best-practices §12.3) becomes test-enforced rather than aspirational.

⚠ `UiAction::TargetPicked(String)` carries the `TargetId` string-rendering
gotcha from PR1 (numeric node ids rendered to decimal strings — flagged
unchecked in the impl-protocol artifact). The registry's `beam.target` arg
schema must reuse `protocol::TargetId` parsing, not reinvent it.

### 2.2 Semantic UI tree (audit Phase-5 "produce a UI/control tree each revision")

📁 NEW `crates/phosphor-app/src/lens/tree.rs`

```rust
#[derive(Clone, serde::Serialize)]
pub(crate) struct UiNode {
    pub id: String,                 // stable semantic id, NOT egui::Id hash
    pub role: &'static str,         // "button" | "combo" | "slider" | "toggle" | "menu" | "dialog" | "toast"
    pub label: String,
    pub value: Option<serde_json::Value>,
    pub enabled: bool,
    pub disabled_reason: Option<String>,
    pub action_id: Option<&'static str>,
    pub bounds_logical: [f32; 4],   // x, y, w, h in egui points
    pub bounds_physical: [f32; 4],  // multiplied by pixels_per_point
    pub children: Vec<UiNode>,
}
```

Capture mechanism: a `LensRecorder` (thread-local or `&mut` on the chrome
struct) that chrome code feeds during layout via a tiny helper:

```rust
lens.record(ui_id, role, label, value, response.rect, enabled, disabled_reason);
```

Applied at each existing widget site in `chrome.rs` (toolbar combos at
`chrome.rs:173,248`, buttons, sliders), `compose.rs`, dialogs. The recorder
rebuilds the tree each frame chrome runs; the shell stores the latest tree +
a monotonically increasing `state_revision: u64` bumped whenever the tree or
`StatusSnapshot` changes (compare serialized hash, not per-frame).

⚠ egui immediate mode: widgets inside collapsed combos/closed menus don't
run, so they won't appear in the tree. That is CORRECT (they are not
invocable by a human either), but `observe` must also list registry actions
that have no current ui node, so agents can still `invoke` them. The audit's
"available actions in the current state" covers both.

⚠ `pixels_per_point` changes on monitor moves — read it per-frame from the
egui context, never cache.

### 2.3 Receipts lifecycle (best-practices §12.4)

📁 NEW `crates/phosphor-app/src/lens/receipt.rs`

```text
accepted -> applied -> completed
                  \-> failed
                  \-> cancelled
```

```rust
#[derive(Clone, serde::Serialize)]
pub(crate) struct Receipt {
    pub correlation_id: String,     // caller-supplied or server uuid-ish (time+pid+counter, no new deps)
    pub action_id: String,
    pub stage: Stage,               // Accepted | Applied | Completed | Failed | Cancelled
    pub revision_before: u64,
    pub revision_after: Option<u64>,
    pub message: String,            // human-readable, "Glow 70% → 82%"
    pub error_code: Option<&'static str>,
    pub fix: Option<String>,        // executable CLI syntax, WireError discipline (test-enforced)
}
```

- `accepted`: server parsed + validated against registry (enabled, args typecheck).
- `applied`: shell drained the `UiAction`/`ControlVerb` and mutated state.
- `completed`/`failed`: async effects finished (renderer rebuild, file open,
  vacuum) — reuse the existing deferred-reply plumbing in `ControlRequest`
  (snapshot/clip already defer, `control.rs` reply channel).
- Idempotency: repeated `invoke` with the same `correlation_id` within a
  window replays the stored receipt instead of re-applying (best-practices
  §12.4 "idempotency key where useful"). Keep a small ring of recent ids.

⛔ Time is seconds: the pulse duration, receipt timestamps, idempotency
window are all seconds (`f64`/`Duration`), never frames.

---

## 3. Wire surface

### 3.1 New verbs (extend, don't fork, `protocol.rs`)

▸ Extend `CtlRequest`… no — ❓ resolved by design: `observe`/`watch` are not
ctl verbs (they are read/stream ops like the existing `Op::Probe`/`Op::Tap`),
and `invoke` is a superset of ctl. Concretely:

- `{"op":"observe"}` → one JSON document (§3.2). New `Op::Observe` beside
  `Op::Probe` in the server match; probe stays untouched (frozen contract).
- `{"op":"invoke","action":<id>,"args":{…},"correlation_id":<opt>}` →
  immediate `accepted`/error line, then receipts flow on `watch`. Also
  returned inline: the terminal receipt when the action completes fast
  (reuse the deferred reply channel), so scripts get a one-shot answer.
- `{"op":"watch"}` → NDJSON stream like tap: first line is the SERVER hello
  (reuse `hello_event`, add `"stream":"watch"` + `state_revision` field —
  this also delivers audit §10.5 "state revision numbers" in the handshake,
  and is the natural place to add the `instance_id` §10.5 asks for),
  then `receipt`, `revision` (with diff summary), `dialog`, `toast`,
  `error`, `end` events. `end_event` reused as-is.
- CLI: `phosphor observe [--socket]`, `phosphor invoke <id> [args…]
  [--correlation-id X]`, `phosphor watch [--socket]` in `agent.rs`, all via
  the existing `connect_to` (socket override + env var already work).

Bump `PROTOCOL_VERSION` to 2; hello carries it; old clients ignore unknown
ops server-side with a `WireError` whose fix is executable (existing
discipline).

### 3.2 `observe` payload (audit §9 list, complete)

```json
{
  "protocol": 2, "state_revision": 417,
  "window": { "backend": "x11", "size_logical": [1280,800], "size_physical": [2560,1600], "pixels_per_point": 2.0, "focused": true, "mini": false, "fullscreen": false },
  "view": "scope", "dialogs": [ { "id":"dialog.kit_editor", "title":"Kit editor" } ],
  "tree": { …UiNode root… },
  "actions": [ …ActionDescriptor with current_value/enabled filled… ],
  "status": { …existing StatusSnapshot verbatim, same struct… },
  "visual": { "frame_hash": "…", "thumbnail_png_base64": null }
}
```

`status` embeds the existing `StatusSnapshot` — no duplicate representation.
Schema: extend `phosphor schema` with `observe`/`watch` sections and add a
`default_observe_validates_against_the_schema` test mirroring PR1's
`default_status_snapshot_validates_against_the_probe_schema`.

### 3.3 Dialogs and disabled reasons

Every dialog gets a semantic id and appears in `tree` + `dialogs`; opening
and closing emit `dialog` events on watch. Disabled controls carry a
structured reason at the SOURCE of the disablement (e.g. "player.next
disabled: playlist has one entry" comes from the same predicate chrome uses
to gray the button — one predicate, called by both).

### 3.4 Optional visual evidence (audit §9 "optional low-res screenshot or frame hash")

- `frame_hash`: FNV-1a (already used in `mpris.rs` trackid) over the
  composited frame buffer, updated per raster; cheap, always on.
- `thumbnail`: on-demand only (`observe --thumbnail`), reusing the snapshot
  export path (`exports.rs`) scaled to ≤256px, base64 PNG. Never per-frame.

⚠ GPU path readback is the expensive part — hash on the CPU-side composite
where available, and mark `frame_hash: null` where a readback would stall
the render loop. Do NOT add a per-frame GPU readback.

---

## 4. Human-visible agent feedback (audit §9)

📁 NEW `crates/phosphor-app/src/lens/feedback.rs`, drawn by chrome:

- **Widget pulse**: when a receipt reaches `applied` and the action has a
  `ui_id` with known bounds, chrome draws a rounded-rect glow over those
  bounds fading over **0.7 s** (spec range 500–900 ms), theme accent color.
  Clock: seconds via egui `input().time`.
- **Toast**: "Agent: Glow 70% → 82% · #a1b2c3" — message from the receipt,
  correlation id short-form appended so human and stream correlate. Reuse
  the existing toast/notify surface (`notify.rs`) if present, else a small
  top-right stack; auto-dismiss ~4 s, click-to-dismiss (Ben's UI language).
- **No focus theft**: invoke never raises/focuses. `raise` remains its own
  explicit action.
- ❓ Pointer-flight animation (audit "optionally animate a pointer") —
  deferred, propose after pulse+toast ship; Ben taste call.

⛔ BUGLOG law: read `docs/dev/BUGLOG.md` before touching chrome/menu code;
the pulse must not affect layout (paint-layer only, `Painter` on top).

---

## 5. Phases, receipts, exit criteria

Each phase = one or more commits, gates run (`cargo fmt --all -- --check`,
`cargo clippy --workspace --all-targets -- -D warnings`,
`cargo test --workspace`), BUGLOG appended if a bug is fixed en route.

### Phase A — Registry (effort ~1.5 days)

▸ `lens/registry.rs` with the full `ActionDescriptor` table for every verb
`CtlRequest` covers today plus the `UiAction`s reachable by keyboard/MPRIS.
▸ Parity tests: registry↔CtlRequest, registry↔UiAction, registry↔MprisCommand,
registry↔keyboard table (all bidirectional where meaningful).
✅ Exit: `cargo test` proves the four surfaces cite one table; delivers
capability (2) statically (enabled logic lands in Phase C).
↩ Pure additive module + tests; revert the commit.

### Phase B — Semantic tree capture (effort ~2-3 days)

▸ `lens/tree.rs` + `LensRecorder`; instrument `chrome.rs` toolbar, transport,
sliders, dialogs, `compose.rs`; `state_revision` in `Shell`.
✅ Exit: a headless (`--background`, isolated runtime dir) instance answers a
direct tree dump (temporary debug op or unit-level render harness) with
stable ids, correct roles/labels/values, logical+physical bounds that agree
with `pixels_per_point`. Receipt: 2560×1440 run where a slider's physical
bounds match a screenshot crop. Delivers capability (1).
↩ Recorder calls are no-ops if the module is feature-gutted; revert commits.

### Phase C — observe + invoke + receipts (effort ~3 days)

▸ `Op::Observe`, `Op::Invoke` in `control.rs`; `lens/receipt.rs`; CLI
subcommands in `agent.rs`/`main.rs`; schema sections + validation tests;
disabled-reason predicates shared with chrome.
✅ Exit: round-trip integration test in the PR1 style — every registry action
driven CLI text → typed → real socket → real `handle_connection` → receipt
asserted through `accepted`/`applied` (stub shell), plus a live isolated-GUI
receipt: `phosphor invoke player.toggle` flips `probe`'s player state and
returns a completed receipt with before/after revisions. Delivers
capabilities (2), (3), (5).
↩ New ops are additive; ctl/probe/tap untouched; revert commits.

### Phase D — watch + feedback (effort ~2-3 days)

▸ `Op::Watch` stream (hello + receipts + revisions + dialogs + toasts + end);
`lens/feedback.rs` pulse + toast wired to `applied` receipts.
✅ Exit: scripted receipt at 2560×1440 — run `phosphor watch` in background,
`phosphor invoke beam.mode ring --correlation-id test-1`, assert the watch
stream shows accepted→applied→completed with `test-1` and revision bump, and
a screenshot taken during the 0.7 s window shows the pulse + toast containing
`test-1`. No focus change asserted via window-state probe. Delivers
capabilities (4) and (7).
↩ Watch is a new op; feedback is paint-only; revert commits.

### Phase E — visual evidence + polish (effort ~1-2 days)

▸ `frame_hash` in observe/watch revisions; `observe --thumbnail`; dialog id
coverage sweep; docs (`docs/AGENTS.md` new section, MANUAL note); BUGLOG.
✅ Exit: the full 7-point audit list executed as ONE end-to-end scripted
receipt against an isolated instance, transcript saved. Delivers (6).
↩ Revert commits; hash field is nullable by design.

**Total honest effort: ~10-12 focused agent-days.** The tree capture (B) and
the live-GUI receipts (C/D) are the schedule risks — egui instrumentation
touches the most BUGLOG-scarred files in the repo.

---

## 6. Open questions for Ben

- ❓ Bind-failure visibility (audit §10.4-6, deferred from PR1): observe/watch
  make this worse if the socket never bound. Where does the degraded state
  surface — window title, toast, or log only?
- ❓ Should `invoke` cover Destructive actions (`quit`, overwrite-on-save) at
  all, or require the §10.5 prepare/commit two-step? Plan default: Danger ≥
  Destructive requires `--confirm` flag which maps to a prepare/commit pair.
- ❓ MPRIS unification depth: registry MAPS to `MprisCommand` (Phase A) but
  does not rewrite the D-Bus dispatch. Full unification is a follow-up.
- ❓ Pointer-flight animation (see §4).
- ❓ Instance identity (`--instance`, audit §10.5): watch hello can carry an
  `instance_id` cheaply; a full selector flag is a separate small PR.

## 7. What this plan deliberately excludes

Coordinate clicking (debug-only, not planned), per-frame GPU readback,
schemars migration (PR1 chose test-validated handwritten schema; keep that
discipline and extend the tests), applet feed changes (frozen v3), any
release/tag/version bump (Ben decides).
