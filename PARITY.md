# PARITY.md — wave 2 exit checklist (Gate B)

One receipt per line. Receipts live in the wave-2 commit messages
(screenshots were verified live during the session; scripted receipts
are reproducible). Per V4PLAN the wave exits **green except applet +
timeline + recorded deferrals** (bottom section).

## Part A — phosphor-audio

| Item | State | Receipt |
|---|---|---|
| Capture core at pipe rate, v3 ring contract | ✅ | capture_probe: 47,275 frames/s ≈48 kHz, rms 0.118, ~432 Hz on a 440 tone; history exactly 96,000 samples/s |
| Target identity scheme verbatim (settings migrate) | ✅ | 14 unit tests pin combo ids, labels, ordering, resolve round-trips (`device:<sink>.monitor` / `app:<key>`+`+`dedup) |
| App capture (sink-input equivalent) | ✅ | capture_probe `app:tone-app` ~434 Hz via TARGET_OBJECT=serial |
| Playback: decode → PW stream, pause/seek/volume | ✅ | playback_probe 16/16 PASS; RT_PROCESS fix receipt (0.35×→1.00×, pw-top) |
| Gapless preload | ✅ | playback_probe: TrackStarted ×2, one PlaybackEnded; album test played 3 tracks seamlessly |
| Cover art from metadata | ✅ | playback_probe: 198-byte APIC extracted (UI display: wave-3 panel) |
| Vacuum port with sacred restore + sweep | ✅ | tests/vacuum/gate.sh **12/12**: link-verified route, kill -9 → module lingers + app silent in void, sweep rescues, graceful release → ORIGINAL sink |
| Hatch decision recorded | ✅ | pactl for module load/unload ONLY (native node destroy kills pulse-shim streams on PW 1.0.5 — "Connection terminated" receipt in gate.sh history) |
| Multi-app mixing | ✅ | mix_probe: 440+660 both hot, 550 control zero, ring backlog law visible |
| .phos playback at header rate | ✅ | playback_probe: 14,400 of 14,400 samples, position parks at true EOF |

## Part B — the shell

| Item | State | Receipt |
|---|---|---|
| One-surface shell (scope = render-gpu pass on surface view) | ✅ | shell-first-light/beam2 screenshots; goldens hold (origin=0 bit-identical, 19/19 suites) |
| Quiet law (1e-4 / 120 / 90, fps counts while quiet) | ✅ | fps-log: quiet:true transition at tone end, ticks continue, zero GPU while asleep |
| Cold-state pacing ≥157 | ✅ | 164.8 lock on the 164.8 Hz panel from t=2s (t=1 is window mapping); uncapped headroom 3,400 fps windowed (21× v3 ceiling) |
| max_fps semantics (0 = monitor) | ✅ | rolling-deadline pacing receipt (naive form measured 152 < law) |
| Transport/toolbar/sliders/settings to v3 labels | ✅ | shell-chrome1.png; save-immediately table enforced |
| Capture selector + refresh semantics | ✅ | live combo (APP·/OUT·/IN·), refresh without picked-a-source side effects |
| Playlist, both advance state machines, seek debounce | ✅ | album run: gapless into Bravo, finished-at-last-track law; asymmetry preserved (manual wraps, auto obeys repeat) |
| Now-playing overlay + .phos credit fade + ARTIST_NODS | ✅ | shell-player.png ("Bravo" fading); nods table verbatim |
| Keyboard map + Konami + escape cascade | ✅ | XTEST-driven receipt; shell-visitor.png (the turtle, all nine ellipses, over the live beam) |
| Mini mode: square/undecorated/above, magnetism, Align, presets | ✅ | 280×280 receipt + Escape restore to 980×640; snap via _NET_WORKAREA |
| Glass + per-style tints + aero coupling | ✅ | glass-on.png: desktop visible through the pane under Muffin (premultiplied path; offline identity) |
| UI styles as data (8 ids) | ✅ | UI_STYLES table; egui owns chrome (V4PLAN: a feature) |
| Snapshot / clip via offline pipeline | ✅ | phosphor-20260704-125257.png written from history (1.5 s, warmup law, xy_dots-wide quirk pinned by test) |
| 3D orbit + wheel dolly + idle drift | ✅ | constants verbatim (0.008, 0.92/1.09, 1.6..8.0, 6 s, 0.05 rad/s); drag blocked in mini, dolly allowed (the asymmetry) |
| App-vacuum ⌀ (context menu) + file-vacuum ⌀ (transport) | ✅ | two distinct vacuums preserved; app route via Gate A machinery |
| MPRIS both directions + media keys | ✅ | busctl receipts: identity/metadata/stable trackids, pause froze Position, Next-while-paused → Charlie (after the tick-level action-drain fix) |
| Settings write-back preserves foreign keys | ✅ | proto round-trip test: unknown keys byte-identical, owned keys updated |
| Kit selection + live apply | ✅ | kit combo scans kits dirs; build_computer re-applies (state zeroed law) |
| Easter eggs undocumented | ✅ | Konami turtle, ARTIST_NODS, --visitor (all ported; none documented — as intended) |
| Permanent gates | ✅ | `cargo test --workspace --release` 19/19; clippy silent; `phosphor bench` ALL PASS (189.7/151.7/26.5/1873 vs 171/79/6/326) |

## Recorded deferrals (allowed by the wave plan)

- **Kit EDITOR dialog** (rows from OPERATIONS, live editing/save):
  selection+apply shipped; the editor itself → wave 3 (v3's own dialog
  was never scripted-verified — HANDOFF audit #2). "Extend tables, not
  UIs" still holds: the op table lives in phosphor-dsp.
- **Compose/draw mode**: deferred to the studio panel work (wave 4
  shares the canvas); `phosphor-studio` remains the compose path.
- **Stream-precompute**: NOT ported — the decision point exercised.
  Rationale: the engine holds 384 k live at 26.5 fps CPU-noise (worst
  case) and GPU 1873 fps-eq; realtime reconstruction never drops on
  this hardware. Revisit only if a real workload sags.
- **Playlist panel DnD reorder** — v3 had no reorder either; drops
  replace the list verbatim (parity, not a gap).

## Wave 2.5 — the Feel Wave (2026-07-04, cleared from Ben's first drive)

His hands-on feedback, resolved. Three confirmed roots fixed:
focus-trap (clicked buttons kept egui focus → every shortcut died);
repaint starvation (egui repaint_delay ignored → laggy buttons, black
resize bands); sRGB double-encode on the live GPU path (formats[0] was
sRGB, shader also gamma'd → washed beam, "CPU crisper than GPU").
Plus: multiplicative wheel gain, Uncapped fps preset, mini
drag-move + corner-resize, fullscreen = scope only (receipt:
2560×1440, zero chrome), CPU-resolution honored live, focus floor
0.6→0.3. New design system (theme.rs) from Ben's data-rep skill:
sharp corners, hairline frames, mono data, carved dimensional primary
controls; **six original themes** (blossom default / light / dark /
chromacore / basalt / afterglow-follows-beam); status bar killed,
fps→top-right overlay, track state consolidated to the transport,
on-scope toasts. Phosphor icon font replaces emoji; new 4-panel app
icon. And the former deferrals LANDED: kit editor (rows from
OPERATIONS), cover-art display, postcard export (receipt: valid
header + playback), window position restore. Aero-coupling retired
(glass now manual, any theme).

**Deferrals now: {compose → studio panel (wave 4), applet (wave 3),
timeline (wave 4)}.** Everything else from Ben's list is done.

## The one receipt that matters

Ben daily-drives it for an evening: capture, vacuum, media keys,
glass, mini, and now — do the themes have soul, is the chrome no
longer laggy. HANDOFF law: the heart emoji is the acceptance test.
