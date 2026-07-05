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

Issue #4 owed the control-socket transport (Unix NDJSON
`ctl`/`tap`/`probe`); shipped in wave 3.2 (below).

## Wave 3.2 — the agent CLI + control socket (2026-07-04)

Issue #4 closed. Phosphor speaks the station convention natively;
zero convention retrofit (its exit-code scheme predated the
convention). Receipts live in the merge commit.

| Item | State | Receipt |
|---|---|---|
| Agent CLI: `probe`/`ctl`/`tap`/`kit`/`schema` (envelope, fix-bearing errors, exit 0/2/3/4, isatty auto-switch + `--json` force) | ✅ | receipts in the wave-3.2 merge commit |
| `probe` live status one-shot (`running:false` when no GUI; `--at` a past ts stubbed → studio wave, exit 2) | ✅ | probe against a live GUI and a dead one |
| Control socket `$XDG_RUNTIME_DIR/phosphor/ctl.sock` (NDJSON) with **EventLoopProxy wake** (reaches the GUI while quiet-asleep) + **deferred reply** for snapshot/clip (returns the written path) | ✅ | `ctl mode`/`theme`/`snapshot` round-trips; wake verified against zero-GPU tick state |
| `tap` NDJSON stream (hello, frame{ts,mode,segments,bbox,centroid,peak,polyline,trace_size}, tick heartbeat) | ✅ | `tap \| jq` receipt in the merge commit |
| Kit schemas shipped: `kit validate\|inspect` + `schema` + `docs/phoskit.schema.json` | ✅ | kit-repair law: a 7B model fixes its kit in one round-trip |
| `feed` unchanged: locked v3-verbatim applet protocol (documented envelope exception: no `event` field) | ✅ | applet 2.0.0 still byte-compatible |

Deferrals now: {`studio` compiler + `probe --at` (wave 4), compose
studio-panel integration (wave 4), timeline (wave 4)}.

## Wave 3.3 — the second feel wave (2026-07-04, Ben's feedback list)

| Item | State | Receipt |
|---|---|---|
| **Blossom Dark** palette (wine-plum warm dark, sakura resting accent, `accent_follows_beam` — the blossom×dark×afterglow fusion Ben asked for); 7 palettes now, combo auto-lists | ✅ | blossom-dark-final.png; A/B against cool `dark`; set as Ben's active ui_style |
| More defined buttons, all themes (rest stroke → line_strong, raised-stone face lerp, hover accent stroke + expansion; bevels stay the carved-primary privilege; corners stay sharp) | ✅ | same screenshot; equal-care audit noted in merge commit |
| Animations: `style.animation_time` 0.12 s eased hover/active everywhere; **theme-switch crossfade** (180 ms smoothstep over every token incl. glass alpha); carved toggles ease via `animate_bool` | ✅ | theme-crossfade-mid.png caught mid-lerp |
| Glass tint at **1 %** steps with a percent readout (`step_by(0.01)` + formatter/parser) | ✅ | "5%" visible in the settings receipt |
| Mini resize: full **8-zone** edge+corner hit-test (26 px corners, 8 px edges, pure fn + unit tests) with live resize-cursor hints | ✅ | 5 new shell tests; square law preserved |
| Mini glitchiness, three root causes: xprop-per-settle → 30 s workarea cache; mid-drag resquare churn → drag-aware deferral (one resquare+snap at settle); set_mini_mode burst → 400 ms entry grace | ✅ | mini_square.png; WM-grab paths flagged for Ben's live pass (Xvfb has no WM) |

## Wave 4.0-truth — correctness (2026-07-05, the ship session)

| Item | State | Receipt |
|---|---|---|
| **BeamSource: one truth for what feeds the beam** (combo + probe render from it; `settings.target_id` is only the remembered preference) | ✅ | probe gains `source:{kind,detail}`; receipts below |
| **TargetPicked dead-state fixed** (guard read `is_playing_file()` AFTER stopping playback → nothing started, fade → frozen black, combo lied) | ✅ | `tests/receipts/w1-geometry.sh`: pick during playback → track pauses, capture starts, `source.kind=capture` |
| Resume takes the beam back (Space/toggle during capture stops capture — double-feed law made symmetric) | ✅ | receipt: `source.kind=player`, `capture.on=false` after toggle |
| No-signal label (silent target past the sleep window says so on-scope) | ✅ | code path pinned by the quiet-law counters; live-drive owed to Ben's eyes |
| **Single instance** (plain launch forwards `raise`/`open` to the live socket and exits 0; dev flags + `PHOSPHOR_NO_SINGLE_INSTANCE` bypass) | ✅ | receipt: second launch exit 0, "already running", window count unchanged; file forward lands in the player |
| CLI front door: `--help`/`-h` print help (exit 0), unknown flags exit 3 — **`--help` used to launch the GUI** | ✅ | receipt script asserts both |
| `kit validate|inspect` accepts N files (used to silently take the first) | ✅ | receipt: two missing files → two `valid:false` reports |
| New ctl verbs `raise`, `open` (+ schema/usage) | ✅ | control.rs parse tests + forward receipts |
| **Geometry truth (the "squished goniometer" investigation)** | ✅ proven round | L=sin/R=cos circle: bbox aspect **1.000** in xy AND xy45 through PLAYER and CAPTURE (private null sink); L-only = horizontal line; mono = M-axis line. Squish does not exist in v4's pipeline |
| Live composite path armored (`composite_into` had ZERO coverage — a viewport bug could ship 19/19 green) | ✅ | `live_viewport.rs`: live == offline byte-exact at origin 137,41; clamp CROPS never stretches; round-circle canary |
| σ HiDPI parity (v4 traces physical px; σ was 1/scale too thin vs v3 at HiDPI) | ✅ | `display_scale` on both renderers, live-only; offline stays 1.0 → goldens byte-identical |
| CPU-live gamma (egui upload path was the one unverified encode) | ✅ single-encode | live window screenshots, scope-region mean: GPU 7.61 vs CPU 7.45 of 255 (2% = beam phase); double-encode would inflate mids ~40% |
| Mini context menu (window re-squared/snapped UNDER the open menu; 20+ items overflowed the 200–520 px square) | ✅ | settle defers while a menu is open; compact scrolling menu in mini |
| Permanent gates | ✅ | workspace tests green (57+23+…+3 live-viewport), clippy silent, vacuum untouched. Bench under the session's 1440p60 recording load: branch 163.3 vs master 158.7 on offline-96k (env tax, branch faster); absolute gates re-run in the release lap |

## Wave 4.0-regalia — the look (2026-07-05, the ship session)

| Item | State | Receipt |
|---|---|---|
| Blossom Dark is the ACTUAL default (settings default was `dark` — the wanted look never shipped to fresh installs) | ✅ | fresh-HOME probe: `ui_style: blossom_dark`; pinned by test |
| The type system: IBM Plex Sans body (14.5) + Medium headings, JetBrains Mono data, `weak_text_alpha` 0.55→0.7, labels read primary ink | ✅ | w3 screenshots — the thin-Ubuntu-Light era is over |
| Selection law: `override_text_color` removed; selected rows render `on_accent` on accent | ✅ | w3-themepicker.png: "Blossom Dark" dark-on-sakura, readable |
| Tofu purge: ✕ ⌀ ◰◳◱◲▣ ⏭⏮ → icon-font glyphs; settings ✕ was literally the "blank square" | ✅ | settings/manual shots; playlist gains its first close button |
| Button depth tiers: carved (primary) / **beveled** (standard — real 2-stroke bevel, pressed inverts + 1 px nudge) / flat (rows) | ✅ | toolbar/transport/kit-editor swapped to bevel_button/bevel_toggle |
| Slider system: accent-filled tracks + REAL-unit mono readouts (×2.13 gain, 71 % glow, ×8 beam, volume % it never had, mono time) | ✅ | w3-blossom_dark.png |
| Four NEW looks — Stonework 95 (loudest bevel in the table, pinned by test), AMOLED (true #000), Paper, CRT Amber; 11 palettes, swatch-chip picker | ✅ | w3-theme-strip.png (5-row A/B) |
| Toolbar law: gear at the true right end of the sliders row (directly BELOW the source icon), Manual (book) beside it | ✅ | all w3 shots |
| In-app Manual: sections, keys table, agent pointer, GitHub link | ✅ | w3-manual.png |
| Light clarity: vacuum verbs say "light only, no sound" in label/tooltip/status/toast | ✅ | code + VacuumApp status wording |
| Permanent gates | ✅ | 20 suites green, clippy silent, w1-geometry 15/15 re-run |

## The one receipt that matters

Ben daily-drives it for an evening: capture, vacuum, media keys,
glass, mini, and now — do the themes have soul, is the chrome no
longer laggy. HANDOFF law: the heart emoji is the acceptance test.
