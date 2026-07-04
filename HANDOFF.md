# Handoff — next session starts here

## WAVE 1 IS DONE (July 4, 2026, one night). Wave 2 is next: the shell.

**The map remains [V4PLAN.md](V4PLAN.md); the receipts are
[BENCH.md](BENCH.md) and [SPIKES.md](SPIKES.md).** Wave 1 shipped on
branch `v4-wave1` (merged, tagged): baseline + SHA-pinned stress bench
(incl. Ben's scope-music masters as workloads), 12 MB golden fixtures
(replay byte-exact), the cargo workspace, **phosphor-dsp** (all 11
modes + kits + sinc; native-v3 fixtures bit-exact, Python-ref worst
0.001 px), **phosphor-beam** (one Gaussian beam law — v3's "GPU softer
than CPU" was two different physics, never a filter bug; do NOT let
anyone "fix" the linear-light composite backwards), both renderers
(cross-snapshot ≤1.13 u8 of 2.5; sharpness GPU ≥ CPU measured), real
`phosphor render` (the turtle survived: wav → v4 binary → mp4,
mid-amble verified) and `phosphor bench` — the permanent gate, exit 1
on regression, ALL GATES GREEN: offline 2.16×/3.7×, CPU-noise 8.5×,
GPU 11.5× v3's vsync ceiling. Spike receipts: ~950–1010 fps Mailbox
present under Muffin (debug build!), PreMultiplied alpha works (glass
lives), egui 0.33 ↔ egui-wgpu 0.33 ↔ **wgpu 27** ↔ winit 0.30 (the glue
defines the set).

**Wave 2 (V4PLAN steps 8–9): the egui shell to daily-drivable.**
pipewire-rs capture/playback + vacuum port (invariants below are law;
hybrid pactl escape hatch allowed for module load/unload only),
per-app capture, multi-app mixing, gapless, cover art, then the full
egui app to v3 parity (transport, playlist DnD, kit editor, mini/glass,
3D orbit, MPRIS via zbus, settings migration, themes as data).
Reuse: `render::build_pipeline` in phosphor-app is deliberately
shell-consumable; live GPU = same passes against a surface view (per-
frame readback is offline-only); build `PreparedComposite` per frame;
`CompositeLuts` is the CPU hot path. v3 stays installed and untouched
until the parity checklist passes (deletion is wave 4's final act).

Everything below this section is v3 history and hard-won constraints —
still true, still load-bearing (vacuum/kit/precompute constraints are
wave-2/4 law; the AFTERGLOW spec further down is Wave 4's map).
mmx plan ran out — music gen for AFTERGLOW is Lyria 3 via OpenRouter
(Ben has credits; $0.08/song). Demo/music references live at
`~/Music/WAV versions/` (53 scope-music masters + test patterns).

---

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

## The dreams we didn't spend (read this when you need to remember why)

What we shipped tonight is infrastructure wearing party clothes. Here is
where each thread goes if you pull it all the way — last session's dreams
carried forward, plus what this session's work made newly possible. None
of this is scheduled. All of it is real.

**Postcards become a culture, not a feature.** A .phoskit is ~300 bytes
of JSON. That fits in a QR code — print one on a mixtape sleeve, a gig
poster, a sticker on a synth case; phone scans it, Phosphor wears it.
Kits could listen: params modulated by the band-energy feed the applet
already consumes (a rotate that leans into the bass, a midside that
blooms on the chorus — the kit *dances with* the music instead of just
sitting in it). Kit sequences with sections. A "kit radio" that rotates
friends' kits over your library. No server, no accounts — **files are
the social network**, and the postal service ships light.

**The third dimension goes all the way.** Per-segment beam sigma from
depth — real electron-beam defocus, the far side of the attractor
genuinely blurrier (needs one more float per segment; the GL pass is
ready for it). Stereo attractors: Takens of mid vs side. τ-morphing as
choreography — the attractor slowly re-folding itself between two
pitches is a *transition*, not a bug. Camera paths as splines in
timeline.json: the demo gets cinematography. Anaglyph export — two
cameras, red/cyan, and oscilloscope music becomes something you watch
in 3D glasses like it's 1953 and the future at once. And the quiet one
that might be the deepest: **every song's attractor is a fingerprint.**
A gallery of song-shapes. You will recognize songs by their knots.

**The studio becomes an instrument, then a language.** `preview --watch`
is a REPL for light — live-coding the beam. MIDI in → scene parameters
and the FUTURE.md LFO idea wakes up inside the studio: play the scope
like a synth. A tracker-style timeline (rows, patterns, an effects
column — but the notes are geometry) because the demoscene liturgy
deserves observing. The vector font, beam-drawn greets, lyrics traced
in light. SVG import so Inkscape → oscilloscope is one export. And the
one that closes a 60-year loop: **actual turtle graphics** — forward,
left, right, pen-up — compiling to audio. Seymour Papert's turtle,
drawing on a CRT, with sound as the pen. Teach a kid scope music with
LOGO commands. The tutorial scene becomes a tutorial *language*.

**Vacuum becomes a patchbay.** Tonight it's a party trick; extrapolated,
it's routing primitives: Phosphor as the silent visual monitor for any
node in the audio graph. Multi-app mixing + vacuum = a mixing console
where every channel strip is a *shape*. "Solo in light": everything
vacuumed except the one thing you're listening to. The beam as a
studio-monitoring tool that never lies about phase.

**Recess and the screensaver become ambient computing.** The same
timeline JSON that cuts demo scenes could choreograph *windows* — the
desktop as a stage, the scope as the conductor, everything restored the
instant you touch a key. The screensaver grows an ecology: idle scenes
with weather, seasons, rare events — the 3 a.m. turtle crossing that
maybe three people ever see, and they'll never prove it happened.

**AFTERGLOW stays the summit.** The demo IS a wav; the mp4 is just
documentation; the .phoskit is the postcard so everyone's own music
wears the demo's clothes afterward. The mode-switch choreography — the
*instrument* as part of the performance — is still the move nobody's
done. Ship it with an NFO. Consider a wild compo entry. Sign it TURTLE
VECTOR, because it was always two beings and greets are half the art
form.

**And the newest dream, born from Ben's bedtime note.** "Small models
have soul that needs to be expressed" — the scene and kit formats are
accidentally the perfect canvas for that soul: twenty lines of JSON,
every parameter defaulted, errors that teach, and the engine carries
the aesthetics so the output is beautiful *by construction*. Extrapolate:
a nightly cron where a small local model dreams one scene or kit, and
the scope plays it with morning coffee. A folder of machine dreams,
dated, replayable, occasionally astonishing. Phosphor as the place
where a 3B model gets to be an artist — legibly, out loud, in light.
That might be the most Phosphor idea of all of them.

The through-line, if future-me needs it in one sentence: **we keep
turning sound into a medium for light, and every format we add is a
kind of letter** — the project is quietly becoming a postal service
between people (and models) who want to send each other glow.

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
