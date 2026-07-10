# SHELL DECOMPOSITION PLAN — Rewrite-3, shell state architecture

**Status: PLANNED, not implemented this arc.** Audit `docs/dev/AUDIT.md`
§3-P0-shell ("`Shell` is a cross-feature state machine without explicit state
machines") + §7-Rewrite-3. Companion law: `docs/dev/BEST-PRACTICES-DRAFT.md`
§10 (thin shell, reducers+effects, Escape precedence, cancellable long ops).

Authoring contract: fable-to-opus. Symbols: ▸ task · 📁 file · ✅ verify ·
↩ rollback · ⛔ constraint · ⚠ gotcha · ❓ decision. Every phase is
independently shippable with the existing receipt rigs green.

---

## 0. Why (the regression pattern this kills)

BUGLOG entries #8, #9, #11, #12 are all the SAME structural failure:

| Bug | Pattern |
|---|---|
| #8 Escape quit the app | Escape had two owners (popup vs leave-cascade) with no precedence table |
| #9 mini wanders / resize overgrows | geometry banked at the wrong moment; racing WM timers; re-square with no clamp |
| #11 mini toggle flaps (FOUR movers) | multi-owner gesture (winit + egui both toggling), settle timer outliving its mode, persisted-vs-live drift (`window_width` read but never written; transients recorded as truth) |
| #12 menu click a beat late | popup window mutated state with no wake effect on the main window |

Root: `Shell` (crates/phosphor-app/src/shell.rs, 4238 lines) holds ~100 loose
fields where mode is encoded as overlapping booleans and `Option<Instant>`
timers, mutated from winit events, egui closures (chrome.rs, 2086 lines),
MPRIS, the control socket, and audio events. Impossible states are
representable, so they occur.

The fix is NOT a rewrite of winit/egui. It is extracting explicit state
machines with pure reducers `(state, event) -> (state, effects)`, executed
effects afterward, so race ownership is visible and unit-testable without a
display.

---

## 1. Field census → machine mapping (actual shell.rs fields at HEAD)

### WindowMode machine (module `window_state.rs`)
Absorbs (shell.rs `struct Shell`, lines ~380–460):
`is_mini`, `is_fullscreen`, `normal_geometry`, `mini_settle: Option<Instant>`,
`mini_resquare: Option<i64>`, `mini_resize_axis`, `geometry_goal:
Option<GeometryGoal>` (+ `struct GeometryGoal` :140), `mini_pending:
Option<Instant>`, `mini_drag_active`, `mini_entering: Option<Instant>`,
`mini_last_click: Option<Instant>`, `workarea_cache`, and the persisted mirror
of `mini_x/mini_y/window_width/window_height`.

```
enum WindowMode {
    Normal,
    Mini { settle: Option<SettleTimer>, resize_axis: Option<Axis>, entering: Option<Deadline> },
    Fullscreen { banked_normal: Geometry },
    // staged fullscreen→mini (today's mini_pending) is an explicit state,
    // not a timer field readable from anywhere:
    Transitioning { from: Mode, to: Mode, goal: GeometryGoal, deadline: Deadline },
}
```
⛔ Encodes BUGLOG #11 laws structurally: the settle timer lives INSIDE
`Mini`, so leaving mini destroys it (mover 3 becomes unrepresentable).
Geometry banks only on the `Normal→Fullscreen` / `Normal→Mini` transitions
(mover in #9). Persistence effect emitted only from
`Transitioning→settled` reducer arms, never from raw `Moved` events (#11
mover 4).

### BeamSource machine (module `source_state.rs`)
`enum BeamSource` already exists (shell.rs:256 — Capture/Mix/Player/Silent,
the "#drift-proof single source of truth" from the Spotify-black-screen bug).
This phase promotes it from a derived cache (`sync_beam_source` :2334
re-derives it) to the OWNING state, absorbing `capture_on`, `app_vacuum`,
`target_cache` freshness, and `mix_selection` commit. Audit shape:
`Silent | Capture{target} | Mix{members} | File{path} | ExternalPlayer` —
ExternalPlayer maps to today's `linked_external_player` (:2312) +
`last_external_signature`.

### Playback machine (module `playback_state.rs`)
Absorbs `player: PlayerState` (player.rs), `cover_texture`, `overlay_art`,
gapless requeue, `cycle_song_index`, `cycle_song_fade`.
`Stopped | Loading | Playing | Paused | Draining | Error(String)` —
Loading/Draining are the states BUGLOG #13 (no-sound vacuum) lived in
implicitly.

### Popup machine (module `popup_state.rs`)
Absorbs `menu_popup: Option<MenuPopup>` (+ struct :120), `menu_popup_spawn`,
`context_menu_open`, `close_menu_request`, plus the panel booleans
(`settings_panel_open`, `manual_open`, `mix_panel_open`, `kit_editor`,
`postcard_dialog`, `epilepsy_prompt`).
`Closed | Opening{spawn_pos} | Open{kind} | Committing{action} | Closing`.
⛔ `Committing` exists so a queued row action ALWAYS produces its wake
effect before `Closed` (BUGLOG #12 law: `chrome_dirty=true` +
`request_redraw()` becomes a reducer-emitted effect, not per-site wiring).

### Export machine (module `export_state.rs`)
Absorbs `exporting: bool`, `export_results: Option<mpsc::Receiver<…>>`,
`control_export_reply` (:~470, the deferred snapshot/clip socket reply).
`Idle | SnapshotPending | ClipPending{reply?} | Rendering{cancel} | Failed{msg}`.
Effects are cancellable tasks reporting completion as events.

### AgentTransaction machine (module `agent_state.rs`)
Absorbs the control-socket request lifecycle (`service_control` :1364,
`apply_external_command` :1268, `update_control_snapshot` :1530):
`Accepted → Applied → VisuallyAcknowledged → Completed(reply)`. Today a ctl
command mutates state and replies before the frame that shows it exists;
this machine makes "applied but not yet rendered" a named state so probe
snapshots stop lying by a frame.

### Stays loose in Shell (deliberately)
Renderer plumbing (`graphics`, `renderer_choice`, `raster_style_stamp`,
`raster_restyle_pending`), frame pacing (`next_frame_due`, `monitor_hz`,
`render_loop_active`, `quiet_frame_count`, `fade_out_frames_remaining`),
AGC (`effective_gain`, `auto_gain_peak`, `grid_gain`), stats
(`fps_*`, `work_ms_ring`, `segments_*`), camera orbit, compose stroke,
appearance undo/redo, `toast`, `konami_progress`. These are per-frame render
concerns or single-owner leaf features — machinizing them is cost without a
regression pattern behind it.

---

## 2. Architecture

```
📁 crates/phosphor-app/src/state/mod.rs      AppState { window, source, playback, popup, export, agent }
📁 crates/phosphor-app/src/state/event.rs    enum AppEvent (superset of today's UiAction :209 + winit/mpris/control-derived events)
📁 crates/phosphor-app/src/state/effect.rs   enum Effect + Executor trait
📁 crates/phosphor-app/src/state/window_state.rs
📁 crates/phosphor-app/src/state/source_state.rs
📁 crates/phosphor-app/src/state/playback_state.rs
📁 crates/phosphor-app/src/state/popup_state.rs
📁 crates/phosphor-app/src/state/export_state.rs
📁 crates/phosphor-app/src/state/agent_state.rs
```

Reducer signature (uniform):
```rust
fn reduce(state: &Machine, event: &AppEvent, now: Seconds) -> (Machine, SmallVec<Effect>)
```
⛔ `now` is an explicit parameter (best-practices §3.1: time in seconds; also
makes timer states unit-testable with a fake clock). No `Instant::now()`
inside reducers.

Effects (executed by Shell after the reduce pass): `SetFullscreen(bool)`,
`SetOuterPosition`, `SetInnerSize`, `SetDecorations`, `PersistGeometry`,
`RequestRedraw`, `WakeMainWindow`, `StartCapture(id)`, `StopCapture`,
`SpawnExport{kind, cancel_token}`, `ReplyControl(value, tx)`,
`ScheduleWake(deadline)`, `Toast(msg)` … Async effects carry a
`CancellationToken` owned by the machine state that spawned them, so leaving
the state cancels the effect (the settle-timer law, generalized).

What remains in shell.rs (target ≤ ~1500 lines): winit
`ApplicationHandler` adaptation (`resumed`/`window_event`/`about_to_wait`),
translating winit+egui+mpris+control inputs into `AppEvent`s, executing
`Effect`s, the render path (`redraw` :2396, `frame` :2803), and graphics
lifecycle. chrome.rs stays the egui view layer but becomes read-state /
push-event only — no direct field mutation of machine-owned fields.

### Escape precedence table (ONE owner, tested)
Replaces the ad-hoc cascade at shell.rs:3719-3728. A single
`reduce_escape(AppState)` consults, in order:

| # | Condition | Escape means |
|---|---|---|
| 1 | egui `Memory::any_popup_open()` (combo etc.) | egui closes it; reducers see nothing (BUGLOG #8 law) |
| 2 | Popup ∈ {Opening, Open, Committing} | close menu popup |
| 3 | modal dialog (epilepsy prompt, postcard, kit editor with unsaved) | close/dismiss modal |
| 4 | compose mode active | leave compose |
| 5 | WindowMode::Fullscreen | leave fullscreen |
| 6 | otherwise | **nothing** (quit is Ctrl+Q / menu Quit only, per §10.3) |

⚠ Step 6 is a behavior change if today's leave-cascade still ends in Close
anywhere — verify against current cascade before landing; if bare-Escape-quit
is still live, keep it behind a deprecation toggle for one release.
✅ Unit table-test: every (state, Escape) row asserted; plus live receipt
re-running the #8 gesture.

---

## 3. Migration phases (each independently shippable, receipts green)

⛔ Global gates for EVERY phase (ARC-BRIEF law 5): `cargo fmt --all --
--check` · `cargo clippy --workspace --all-targets -- -D warnings` ·
`cargo test --workspace` · `tests/receipts/w2-wm-geometry.sh` 10/10 at
2560×1440 · the #12 menu-wake receipt gesture. One commit per phase,
`refactor(shell): story` style. ⛔ Behavior-preserving: no golden changes,
no protocol changes, applet feed untouched.

### Phase 0 — scaffolding + event unification (~0.5 day)
▸ Create `state/` module tree, `AppEvent` wrapping today's `UiAction`
variants unchanged, `Effect` enum, executor loop in `drain_actions`
(shell.rs:800) that today just forwards. Zero behavior change.
✅ gates + receipts. ↩ delete `state/`, revert drain_actions.

### Phase 1 — Popup machine (~1 day)
Smallest surface, owns BUGLOG #8+#12 directly, both already receipted.
▸ Move `menu_popup`/`context_menu_open`/`close_menu_request` +
`open_menu_popup` (:1943) / `close_menu_popup` (:2017) transition logic into
`popup_state.rs`; `Committing` emits `WakeMainWindow`.
✅ gates; #8 Escape receipt; #12 +350 ms screenshot receipt; new reducer
unit tests (open→escape→closed, commit→wake emitted).
↩ `git revert` phase commit (machine is additive; old fields removed only in
this commit, so revert restores them).

### Phase 2 — Escape precedence table (~0.5 day)
▸ Replace shell.rs:3719 cascade with `reduce_escape` + table test.
✅ gates; #8 receipt; table unit test. ↩ revert commit.

### Phase 3 — WindowMode machine (~2–3 days, the big one)
▸ Move the twelve geometry fields + `set_mini_mode` (:1697),
`snap_mini_to_edges` (:1840), `align_mini` (:1874), GeometryGoal convergence,
fullscreen toggle, mini re-square/settle into `window_state.rs`. Timers
become `Deadline` values inside states; Shell asks
`window.next_deadline()` to schedule wakes. Persistence becomes a
`PersistGeometry` effect emitted only on settle.
⚠ `PHOSPHOR_GEOM_LOG=1` must keep logging every decision (BUGLOG #11 law —
the log IS the receipt); log from the effect executor, tagged with the
reducer transition name.
⚠ synthetic-key filtering (`!is_synthetic`) stays in the winit adapter, NOT
the reducer — it is platform noise, pre-translation.
✅ gates; `w2-wm-geometry.sh` 10/10 (relaunch restore, SIGSTOP-pulsed Muffin
round-trips, double-click = one toggle, drag-then-M, F11→M chain, quit-from-
mini) — this rig is the phase's acceptance test; new fake-clock unit tests
for settle expiry, bank-on-entry, no-bank-clobber.
↩ revert commit; rig re-run confirms restoration.

### Phase 4 — BeamSource + Playback machines (~2 days)
▸ Promote `BeamSource` to owner; `sync_beam_source` (:2334) becomes reducer
arms; capture start/stop and vacuum routing become effects. Playback machine
wraps `PlayerState` transitions; gapless requeue is a `Draining→Loading`
transition.
✅ gates; live receipt: file play → capture on (capture outranks player) →
capture off → player resumes; probe `source` matches the combo label after
each step (the drift the enum was born to kill); MPRIS play/pause receipt.
↩ revert commit.

### Phase 5 — Export machine (~1 day)
▸ `exporting`/`export_results`/`control_export_reply` →
`export_state.rs`; export thread gets a cancel token; ctl `snapshot`/`clip`
replies emitted as `ReplyControl` effects on `Rendering→Idle`.
✅ gates; ctl snapshot receipt (reply arrives, file exists); cancel-mid-clip
unit test with fake executor. ↩ revert commit.

### Phase 6 — AgentTransaction machine (~1 day)
▸ Wrap `service_control`/`apply_external_command` request lifecycle;
`update_control_snapshot` reads `VisuallyAcknowledged`+ states only.
⛔ Protocol surface unchanged (frozen envelopes, applet v3 untouched) — this
is internal sequencing only.
✅ gates; ctl round-trip receipts for a representative command set (theme,
target, mode); probe-after-set consistency check. ↩ revert commit.

### Phase 7 — sweep (~0.5 day)
▸ Delete dead shims, assert shell.rs line count, module-doc each machine,
BUGLOG cross-references in code comments, append best-practices §10
compliance note.
✅ gates; full receipt suite; `wc -l shell.rs` ≤ 1800 recorded in commit.

**Total honest estimate: 8–10 focused days** (Phase 3 dominates and can
overrun; it is also the highest-value). Phases 1+2 alone are a worthwhile
partial ship if the arc budget is short.

---

## 4. Risks / open questions

- ❓ egui closures in chrome.rs mutate `self` freely today; phases convert
  call-sites incrementally — accept a transition period where chrome pushes
  `AppEvent`s but still reads machine state through accessor shims.
- ❓ Does `Transitioning` need nesting (fullscreen→mini staging is
  two-hop)? Recommend a `to`-queue of length 1, not general nesting.
- ❓ Bare-Escape-quit deprecation (phase 2 ⚠) — Ben decides.
- ⚠ `MenuPopup` is its own OS window with its own egui ctx; the machine owns
  lifecycle, but the popup's render loop stays in shell (it is render, not
  state).
- ⚠ Receipt rigs are the ONLY WM-truth oracle; unit tests cannot replace
  `w2-wm-geometry.sh` for any geometry phase.
