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
  **Postscript (2.6 install night):** a real workload DID sag — Ben's
  profile carried v3's `renderer=cairo` choice (made when v3's GPU
  looked washed; the 2.5 sRGB fix removed that reason), and cairo at
  full res ran 52–65 fps on Attack Vector while the SAME settings on
  the GPU renderer lock 480.0 (his cap). Precompute would not have
  helped: it caches SEGMENTS, and the cairo cost is per-frame
  RASTERIZATION. Decision stands. The real follow-up is Ben's
  observation that chrome shares the scope's thread — a CPU-raster
  worker (double-buffered frame texture, chrome never blocks) is
  queued for wave 3.
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

## Wave 2.6 — compose + the confidence pass (2026-07-04)

Ben pulled compose forward from the wave-4 deferral; the rest is the
audit-driven polish wave. Receipts (all live, scratch HOME, XTEST):

| Item | State | Receipt |
|---|---|---|
| Compose mode (draw → hear, full v3 semantics) | ✅ | square drawn via XTEST redraws EXACTLY in place (preview + playback screenshots align); loop WAV = 1 s whole cycles at 96 k; toolbar ✏ status verbatim |
| Compose math in phosphor-dsp (studio reuses it) | ✅ | 8 unit tests: constant-speed on a square (edge lengths exact to 1e-12), closure, clamp 20–400, 16-sample floor, smoothing centroid-invariant |
| Scroll-retune (×1.06/notch, 300 ms debounce) | ✅ | status 80→161 Hz; wav bytes re-pinned: detected cycle 596 = 161.07 Hz |
| **Scope wheel was DEAD (wave-2 regression)** | ✅ fixed | `wants_pointer_input()` counts the CentralPanel → over_ui always true → gain/dolly/mini-resize/retune wheels all dead. Now gated on the scope response's own occlusion-aware `hovered()`. Receipt: gain 15%→35% by wheel over Attack Vector |
| **Space during playback = pause (v3 law)** | ✅ fixed | v4 started device capture OVER the playing track (double-feed chaos — found on Ben's "use the real songs" receipt). Now capture-toggle while a track is loaded is play/pause: froze at 0:08, resumed to 0:10 |
| Auto-gain AGC (was checkbox-only, no AGC ran) | ✅ | 0.1-amplitude circle fills at the 6.0 clamp (~6× measured); Attack Vector (peak 1.0) correctly untouched; slider greys |
| .phoskit drag-drop import (v3.2 parity, was a stub toast) | ✅ | validate → install to user kits dir → activate; broken kit toasts its error, never lands |
| Crash-proofing | ✅ | kit-save toast unwrap, theme-fallback unwrap, broken-kit now warns (stderr + KitChanged toast), clip-export ffmpeg stderr tail captured, vacuum pactl return codes checked (the v3 law) |
| Snapshot + clip on real content | ✅ | snapshot toast while composing; 10.00 s clip of Attack Vector's cube tunnel (6.5 MB mp4, audio muxed) via the new stderr-piped encoder |
| Test coverage | ✅ | keyboard map/Konami/escape-cascade pinned (5 tests), engine vacuum sweep parse + metadata parse (3 tests); workspace 74 passed, clippy silent |
| Permanent gates | ✅ | bench ALL PASS (144.2/25.4/1602.4 vs 79/6/326 on the reduced set run); vacuum gate.sh 12/12 |

Deferral note: the context-menu "Export drawing as WAV (10 s)" item
ships and renders (screenshot receipt); its click handler is three
lines into the export machinery proven live by the snapshot toast —
the literal menu click could not be XTEST-verified (menu automation
raced Ben's live mouse). First human use will receipt it.

**Deferrals now: {compose studio-panel integration (wave 4), timeline
(wave 4)}.**

## Wave 3.1 — the applet goes engine-free (2026-07-04)

Issue #3 closed. The Cinnamon applet (2.0.0) bundles ZERO engine code;
it spawns `["phosphor","feed"]` and draws the beam-segment stream.

| Item | State | Receipt |
|---|---|---|
| `phosphor feed` subcommand (stdio NDJSON, protocol verbatim from v3's `phosphor_applet_feed.py`) | ✅ | receipts in the `v4-applet` merge commit |
| Deliberate deviation: capture-death recovery | ✅ | on capture death, re-resolves the default output monitor once/second and reconnects — v3's feed just went dark |
| Applet 2.0.0, engine-free | ✅ | no bundled `phosphor_core.py`/`.so`; native-fed via `applet/install.sh` against deb `4.0.0~wave3.1` |

Issue #4 still owes the control-socket transport (Unix NDJSON
`ctl`/`tap`/`probe`); the applet can migrate off stdio onto that socket
when it lands. Deferrals now: {compose studio-panel integration (wave
4), timeline (wave 4)}.

## The one receipt that matters

Ben daily-drives it for an evening: capture, vacuum, media keys,
glass, mini, and now — do the themes have soul, is the chrome no
longer laggy. HANDOFF law: the heart emoji is the acceptance test.
