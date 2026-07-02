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
