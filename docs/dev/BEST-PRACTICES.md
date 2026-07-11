# Phosphor best practices — the adopted law

Distilled 2026-07-10 from `BEST-PRACTICES-DRAFT.md` (the GPT's 1388-line draft,
an arc input) against repo reality at `9553c9c`. The draft is kept verbatim as
the reference; THIS file is the binding version. Rule of the house applies:
one fact, one home — where a law already lives somewhere (BUGLOG, GOLDEN.md,
lib.rs doctrine), this file points instead of copying.

**How to read the tags:**
- **LAW** — binds every change, today.
- **NEW-CODE LAW** — binds anything you write from now on; the existing code it
  indicts is grandfathered until the named repair ships (don't "fix" it
  piecemeal — that's what the repair plans are for).
- **ASPIRATIONAL** — true only after the named repair; do not claim it before.

---

## 1. The gate (what actually runs before every commit)

**LAW.** `cargo clippy --workspace --all-targets -- -D warnings` silent, and
`cargo test --workspace` green (20 suites + whatever you added). Intentional
golden changes carry a migration note in the commit (GOLDEN.md law).

The draft's Step-6 matrix names commands that are not real here (verified
2026-07-10): `cargo fmt --all -- --check` fails on ~50 files — the tree was
NEVER rustfmt-clean, there is no rustfmt.toml, and hand-formatting is the
house style; `cargo deny` is not installed and has no config. ARC-BRIEF gate 5
inherited the fmt line from the draft — void.
**RULING (Ben, 2026-07-10): rustfmt is STRUCK, permanently.** Hand-formatting
is the style; never run `cargo fmt`, never "fix" formatting drive-by, and no
future doc reintroduces an fmt gate. `cargo deny` (license/advisory audit)
remains sensible as a CI addition — FUTURE.md material, not a local gate.

## 2. Ownership map (LAW)

Crates are ownership boundaries (draft §2, verified accurate):
audio transport/clocks = `phosphor-audio` · PCM→segments = `phosphor-dsp` ·
beam physics (one law) = `phosphor-beam` · deposition = `phosphor-render-gpu`
/ `-cpu` (same model, documented tolerance) · formats & wire types =
`phosphor-proto` + `phosphor-app/src/protocol.rs` (the one typed wire module
since `1ec2f6c`) · state/effects = `phosphor-app` · `phosphor-studio` =
out of the workspace entirely (stub archived 2026-07-10, `archive/`);
when the studio returns (FUTURE #1) it starts fresh.

Direction of flow: callback → bounded transport → analysis → renderer-neutral
segments → deposition → time-domain persistence → presentation/export. No
reverse dependencies (renderers reading UI state, callbacks touching widgets,
protocol handlers mutating renderer internals).

## 3. Time (NEW-CODE LAW → DECAY-DT-SPEC)

Every new decay, animation, timeout, or transition states its clock (audio
graph / media / monotonic / simulation tick / presentation) and is written in
seconds — `keep(dt) = exp(-dt/τ)`, never a bare per-frame constant. Mode
geometry clocks advance by samples consumed (the swirl doctrine,
phosphor-dsp lib.rs) — deterministic and export-safe.
The existing FLASH/GLOW per-frame constants are the grandfathered core;
`DECAY-DT-SPEC.md` (FINAL, REF_DT = 1/60) is the one authorized repair.
ASPIRATIONAL until it ships: "decay agrees at 30/60/144/240 FPS."

## 4. Real-time audio (NEW-CODE LAW → SPSC-RING-DESIGN)

After stream start, stream callbacks: no locks, no allocation, no I/O, no
formatting, no front-drains, no unbounded loops. Bounded work only: block
copy/convert, preallocated SPSC ring write, sequence/timestamp attach,
lock-free counters, silence on underrun. Drops beat blocking, and drops are
counted and observable.
Today's capture/playback paths violate this (audit P0-2, verified in
`SPSC-RING-DESIGN.md` + `thread_probe.rs`); that design is the one authorized
repair — don't half-fix callbacks outside it. Mix/downmix correctness law
lives in `DOWNMIX-DESIGN.md` (timestamp alignment, per-source gain, headroom,
layout-aware folds; stable coefficients n1.0..n8.0 per the A.5 probes).

## 5. One fact, one representation (LAW — partially repaired)

No duplicated CLI/server parser types (repaired: `protocol.rs` round-trips by
construction), no handwritten schema beside runtime types (the
snapshot-vs-schema test guards it — extend BOTH halves together or it fails),
no copied settings-key lists, no copied mode lists. Known surviving
duplicates, each with its assigned fix: `DISPLAY_MODES` vs `Mode::ALL`
(pin test lands with RECURRENCE-BLOOM-PLAN Phase 4) · HANDOFF status prose
duplication (audit PR-2 leftover). Add a new list only with a test that pins
it to its source of truth.

## 6. Rendering (LAW)

One beam model, two implementations, documented tolerance, deterministic
fixtures — never "looks close." Goldens are frozen ground truth
(docs/dev/GOLDEN.md); live path pinned by live_viewport.rs; both stay. New
modes emit renderer-neutral segments and get NO golden fixtures (they have no
v3 reference) — they get deterministic unit tests instead. Live and offline
agree at equal media timestamps. The egui 0.33 / egui-wgpu 0.33 / wgpu 27 /
winit 0.30 quartet moves together or not at all. Reuse the existing surface
adapter on rebuild (repaired: `0582e24`).

## 7. State & UI (NEW-CODE LAW → SHELL-DECOMPOSITION-PLAN)

Impossible states unrepresentable (enums over boolean soup); a transition owns
preconditions, update, effects, receipt, cancellation. The current `Shell` is
the grandfathered offender (audit P0-4); `SHELL-DECOMPOSITION-PLAN.md` is the
authorized repair. Until then: **read `docs/dev/BUGLOG.md` before touching
UI/input/menu/playlist/window code and append every root-caused fix** — the
regression ledger IS the working state-machine law. UI receipts exercise the
user's actual gesture, at 2560×1440 (BUGLOG standing laws). Settings writes
are atomic + quarantining (repaired: `73d7270`); new settings keys get clamps
and round-trip tests.

## 8. Agent surface (LAW)

Every mutation reachable by humans is reachable semantically (one action path:
widget/keyboard/MPRIS/ctl converge on the same handling); errors name the fix;
schema mirrors runtime (the honesty test). Handshake, action registry, and
observe/invoke/watch are `ACTION-LENS-PLAN.md`'s territory (ASPIRATIONAL —
plan ready, prerequisite landed).

## 9. Claims (LAW)

Don't call X11-only behavior "Linux," an Xvfb GUI "headless," a palette a
"physical phosphor simulation," or a geometry tap "visual inspection." Bench
numbers carry hardware + load context (the BENCH environmental law). Tests
skipped for missing local capability are reported as NOT RUN, never silently
green.

## 10. Visualizers (LAW for the genre)

Three layers, strictly: measurable features → deterministic geometry →
presentation romance. Seeded, never random; scene changes are driven by
audible properties, not timers. Degrade decoratives before dropping audio.
`RECURRENCE-BLOOM-PLAN.md` supersedes draft §14 where they differ — the
deliberate divergences: analysis runs in `compute()` on the frame thread
(measured within budget; the draft's separate analysis worker presumes the
SPSC rewrite — revisit after it ships), no new CPU/GPU parity fixture (bloom
emits segments; the shared beam law already carries parity), and Tempo Lock /
"Tube" controls are FUTURE (need tempogram / scanline machinery that doesn't
exist — honest-ledger rule).

## 11. Scope control (LAW)

No speculative production shells in the active workspace (studio precedent);
`#[allow(dead_code)]` states its owner and its expiry; examples/spikes are
labeled as rigs or deleted. Licensing: embedded third-party assets (fonts are
the known case — OFL texts ship) get generated notices at package time —
audit §12 lists the gaps; that's Phase-6/release-hardening work, tracked in
FUTURE.md, not a per-commit gate.

---

## Critique of the draft (what didn't survive distillation, and why)

1. **Unreal gate commands** (fmt / deny / --all-features) — §1 above. The
   draft was written by a static auditor that could not build; it guessed a
   conventional Rust gate. Ours is narrower and real.
2. **Skill formatting** (frontmatter, "use this skill") — it isn't installed
   as a skill. **RULING (Ben, 2026-07-10): project-specific skills stay in
   the project** — this file is repo law, no harness mirror. (Skills earn a
   harness home only when they apply across workspaces.)
3. **Worker-architecture prescriptions stated as present-tense law** (§14.3,
   parts of §5) — they describe the post-SPSC world. Kept as NEW-CODE /
   ASPIRATIONAL with the repair named, so nobody "complies" early with a
   half-architecture.
4. **Duplicated repo lore** (§2 data path tables, golden rules, BUGLOG
   restatements) — pointed at the real homes instead of copied; the draft
   version WILL drift, the pointers won't.
5. **Everything else held up.** The invariants (§3), RT rules (§5), claims
   honesty (§3.7), visualizer three-layer contract (§13), and the §14 bloom
   profile were genuinely good — they shaped RECURRENCE-BLOOM-PLAN.md and
   this file adopts their substance.
