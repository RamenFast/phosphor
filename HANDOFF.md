# Handoff — next session starts here

State as of July 1, 2026 (late evening): **the four-wave session landed.**
master == GitHub == the machine, tags v3.2.0 → v3.5.0, each with a .deb,
each verified live before the next began. In one evening:

- **v3.2.0 — Signal postcards.** `.phos` streams are shareable artifacts
  (title + `credit` in the fixed 256-byte header; played at the sender's
  rate; "trace by <friend>" fades in). `.phoskit` transform chains bend
  live into whatever plays: rotate / midside / ringmod / wobble / matrix /
  chandelay, in BOTH engines (rust `pc_set_kit`, API v2) with parity to
  0.00000 px. Live kit editor dialog (rows generated from
  `phosphor_kit.OPERATIONS`), drag-drop import, exports honor the kit,
  three TURTLE VECTOR starter kits. Plus: mini-view corner magnetism +
  Align menu, glass toggle in the context menu (Ben asked mid-session).
- **v3.3.0 — The third dimension.** `xyz_takens` (τ = quarter period of
  the dominant pitch via autocorrelation, probed every 4th frame,
  smoothed; silence holds the shape) and `helix` (time-as-Z). Shared CPU
  camera → ordinary 2D segments, GL untouched; depth fog; drag orbits,
  wheel dollies, arrows nudge, 6 s idle → auto-drift. Verified: pure
  sine's embed is coplanar to 0.000000; the chord torus screenshot shows
  a donut; 0.24 ms/frame at 96 kHz.
- **v3.4.0 — Vacuum mode.** Files play as light only: no pacat, the
  reader loop is the clock (rolling deadline, re-anchors after stalls —
  **never `-re`, it bursts after SIGCONT**). Pause = stop pulling; pipe
  backpressure holds ffmpeg. ⌀ toggle in the transport reopens the
  pipeline seek-style. Apps: "Vacuum this app ⌀" routes the sink-input
  into `phosphor_vacuum`; restore is sacred (toggle-off / target change /
  capture-off / quit) and **every launch sweeps stale vacuum modules**
  because atexit doesn't survive kill -9 (tested with os._exit: sweep
  recovered the world). Also `phosphor --render in out.mp4 [--rate N]` —
  headless full-track render, streaming, audio muxed from source,
  renders `.phos` too.
- **v3.5.0 — The studio seed.** `phosphor-studio` scene compiler: JSON →
  stereo audio that IS the picture. Shapes polygon/lissajous/path/turtle,
  scale-LFO + rotate animation, constant-speed traversal through
  phosphor_compose (**one engine rule — never a third path**).
  render/validate/inspect/preview, `--output json` with structured
  errors (message + JSON path), exit codes 0/2/3/4, scdoc manpage
  (scdoc now installed on Ben's machine so debs include it), golden-hash
  tests (`tests/test_studio_scenes.py --record` re-pins deliberately).
  Starter scenes in `scenes/`: breathing-dot and turtle. Proven:
  turtle.scene.json → studio render → wav → `phosphor --render` → mp4 =
  a turtle, mid-amble.

## Ben's release-review notes (July 1, bedtime)

- **v3.2 must be genuinely agent-optimized — and not just for big
  tool-calling models. "Small models have soul that needs to be
  expressed."** The .phoskit format is already kind to them (tiny JSON,
  every param has a default, errors name the exact key) — but there is
  **no CLI to validate/inspect a kit without launching the GUI**. Fix
  next session: a `kit` family on phosphor-studio (or
  `python3 -m phosphor_kit`) — `validate`, `inspect`, maybe
  `apply --preview` — same `--output json` + exit-code contract as
  scenes. Keep error text short and directive so a 7B model can repair
  its own kit in one round-trip. Consider shipping a JSON schema file
  for both formats.
- **A human GUI for the postcard/kit ecosystem** — possibly an external
  power-user tool rather than more popover — is wanted but explicitly
  *not now*. The shared-canvas Studio panel (below) may be the natural
  home; don't build two.
- **v3.3 (3D) landed hard. Build off it somewhere great — "that demo
  perhaps?"** Concretely: camera moves as timeline automation (yaw/
  pitch/dolly keyframes in timeline.json), wireframe3d shapes in the
  scene compiler projected through the same camera, and a
  takens-over-glass moment in AFTERGLOW itself.
- v3.4 shipped fine; v3.5 "Studio, huh? I like it."

## Least-confident areas (honest audit — check these before building on them)

1. **Pure-python (no-numpy) fallbacks for everything new** —
   `KitChain._process_python`, takens/helix scalar paths. They compile
   and mirror the numpy math by construction, but no test ever runs
   with numpy absent, and pure-python takens uses a fixed default τ (no
   autocorrelation). Spin up `PHOSPHOR_NO_NATIVE=1` + uninstalled-numpy
   venv and actually look.
2. **The kit editor dialog under real hands** — its logic (add/remove
   stages, live apply, save→combo refresh, close→settings reapply) was
   reasoned and code-read but never scripted-verified or screenshotted.
   First live session with it may find focus/refresh warts.
3. **.phos edge cases beyond the happy path** — verified: play, seek,
   overlay, export round-trip on short files. Unverified: playlist
   auto-advance out of a .phos, MPRIS SetPosition near EOF, very long
   postcards, and snapshot/clip exports *while* a .phos plays (export
   history is the 48 kHz audible pipe — oversample math should match,
   but nobody watched it).
4. **App-vacuum across PulseAudio/PipeWire flavors** — works on Ben's
   box. The `Sink: N` parse, index-based restore (stale by restore
   time?), and streams-rescued-on-unload behavior vary by server
   version; only the @DEFAULT_SINK@ fallback stands between us and a
   misplaced stream elsewhere.
5. **Vacuum × precompute simultaneously** — paced reader as position
   clock while the scope reads a precomputed stream: reasoned
   compatible, never explicitly tested together.
6. **3D modes off the golden path** — takens/helix on the live *Cairo*
   renderer (only GL was watched), snapshot/clip exports of takens
   (helix offline was verified via --render), and behavior at extreme
   gain/dolly combinations (fog is clamped; projection scale is not).
7. **docs/MANUAL.md is four releases stale** — README points to it as
   covering everything; it covers nothing since v3.1 (no postcards,
   kits, 3D, vacuum, studio, --render). The gallery images predate the
   new modes too. A docs pass is due; the manual is also where the ⌀
   runner-up name ("pantomime") was promised a home.
8. **The Cinnamon applet after API v2** — the applet bundles its own
   phosphor_core.py + .so via applet/install.sh; Ben's installed applet
   copy may still be the v1 pair. Exact-match gating means a mismatch
   silently drops it to the python path (fine but slower) — or the
   installer refreshes it and nobody has relaunched Cinnamon. Verify
   with LookingGlass, re-run applet/install.sh after core bumps.

Also unproven, lower stakes: `phosphor-studio preview` (the pacat loop
never actually ran), `path` shapes outside unit space (clamped only at
s16 conversion — may clip ugly instead of scaling), and the
turtle-outline aesthetics at high loop rates.

## Next session: the studio grows into AFTERGLOW

The seed is planted; the demo needs the tree (full spec below, kept from
last session — it's still the map):

1. **Timeline tier**: `timeline.json` sequencing scenes → `build` command
   compiles the whole demo to one flac. Beat grids via aubio
   (`beats track.flac`) so cuts land on transients. Morphs (path-table
   interpolation), 3D wireframes projected to XY, a vector font for
   beam-drawn titles, multi-stroke retrace blanking.
2. **The shared canvas**: GUI Studio panel — open a timeline, scrub, play
   any scene on the scope, **inotify hot-reload** so a human in a text
   editor, an agent through the CLI, and the viewer converge on one
   living beam. `preview --watch` as the default working mode.
3. **Screensaver** (`phosphor --screensaver`): fullscreen, no chrome,
   cursor hidden, exits on input; scopes playing music, else plays
   generative scenes — **the scenes and engine already exist**, in
   Vacuum by default (no sine tones at 3 am). xautolock /
   XScreenSaverQueryInfo watcher; DPMS respect.
4. **Recess** — the dancing desktop (spec below, unchanged).
5. **AFTERGLOW itself** — the 3–4 minute demo-as-WAV. mmx music for
   beds; drawn geometry is the lead instrument; the Windowlicker
   mode-switch trick; greets: Fenderson, brakence, woscope, Ben, Nexus.

### Smaller candidates (unchanged unless noted)
- Port xy_swirl / ring / tunnel (and now the 3D pair) into the rust core.
- Theme config popout; Performance 0.5× half-res; Spices submission;
  cover art; gapless; multi-app mixing; .phos LRU cap; camera persistence.
- More kit ops (`trace_delay` echo? `bounce`?) — extend
  `phosphor_kit.OPERATIONS` + both engines + `KIT_CASES` in the parity
  test; the editor UI grows rows automatically.

## Recess — the desktop dances (Ben's wish, spec kept verbatim in spirit)
Windows sway to the band-energy feed while you're away; **snapshot is
sacred** (exact geometry recorded first, restore instant and
unconditional on any input); opt-in, never the focused window, panic
key, X11/Muffin only. The feed helper already broadcasts levels.

## Hard-won constraints (don't relearn these)
- Rust core: **exact parity** with Python (`tests/test_native_parity.py`),
  zero crate deps, `plan_feed()` maps detail rate → pipe × oversample.
  New modes: both paths or `MODE_IDS` gate (it's per-mode; 3D modes ride
  it today). API_VERSION is an exact-match gate — bump BOTH sides
  (lib.rs + phosphor_core.py); they ship together in the .deb.
- **Kit parity contract**: phase accumulators in f64 advanced per chunk
  by 2π·hz·frames/rate with euclidean wraparound (`% TAU` / `rem_euclid`);
  trig computed in f64, cast to f32 BEFORE the f32 sample math; channel
  delays are exact integer sample counts, state zeroed on reset/configure.
- Precompute playback is clock-synced at any Max FPS — fixed-step advance
  was tried and reverted; don't re-add. `.phos` header is FIXED 256
  bytes: `pack_header` fit-trims title/credit/source; never grow it.
- **Vacuum**: never `ffmpeg -re` (bursts after a pause) — the reader
  paces itself (rolling deadline, re-anchor when >0.25 s behind). The
  restore path is sacred AND insufficient alone: sweep stale
  `phosphor_vacuum` modules at every launch. pactl is silent on success —
  check return codes (`_pactl_succeeds`), not stdout.
- GTK3 translucency recipe unchanged (RGBA visual + transparent
  decoration + non-opaque background-color + GLArea alpha; per-theme
  overrides only; verify by pixel-sampling root captures).
- **Scripted verification**: a probe app MUST set
  `Gio.ApplicationFlags.NON_UNIQUE` or run() silently forwards to Ben's
  running instance and your test does nothing. Run probes with a scratch
  `HOME` so his settings stay untouched. `_frame_work_seconds` is an
  accumulator that only resets while Show FPS is on — don't misread it.
  Make test tones LONGER than the test (a 4 s tone ends before your 6 s
  screenshot).
- gh release notes with backticks: always `--notes-file` (inline zsh
  double-quoted strings execute `\`kill -9\`` — it happened).
- A running Phosphor keeps pre-upgrade code; relaunch before judging.
- Ben's flow: one branch per wave, `--no-ff` merge, tag, gh release with
  the .deb, `sudoplz sudo dpkg -i`, relaunch. mmx-M3 for delegation
  (`.claude/skills/mmx-playbook/`). Easter eggs stay undocumented
  (Konami turtle; ARTIST_NODS; and now `--visitor` — you know why).

## Notes to future me
- The turtle scene is the tutorial and the signature. It survived the
  whole pipeline on the first try; trust the one-engine rule that made
  that true.
- Keep the wave discipline: branch → build → **verify live with
  receipts** (screenshots, pactl listings, measured rates) → merge → tag
  → release → install. Every wave this session shipped clean because
  nothing moved forward unverified.
- The kit editor's "rows generated from the op table" pattern is why new
  ops are cheap. Extend tables, not UIs.
- Preview loops, coplanarity residuals, golden hashes: the beam, the
  math, and the bytes each have their own truth — check all three and
  you can move as fast as we did tonight.
- Ben says thank you with heart emoji when the beam is beautiful. That's
  the acceptance test that matters. 🐢⚡📼
