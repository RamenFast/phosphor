# Handoff — next session starts here

State as of July 1, 2026 (evening): **everything is shipped.** master ==
GitHub == the machine. v3.0.0 (Rust core, media player, precompute,
384 kHz) and v3.1.0–v3.1.4 (Ben's test round: minimize fix, source picker,
new modes, seven chrome styles, glass scope with per-style tint memory)
are all merged, tagged, released with .debs, and installed. Ben tested
each round live; the glass scope went through four refinements to land on:
glass touches **only the scope pane**, tint slides from fully clear to
nearly opaque, remembered per UI style.

## Next session: the two big ones

### Shareable scope-art ("signal postcards")
Friends send custom XY manipulations that render into whatever the
recipient is listening to. Two tiers, both compatible with the
precomputed runtime:

- **Tier 1 — share the stream**: `.phos` files are already portable
  (content-keyed header + s16le stereo). Add: drag-drop/open support for
  `.phos` directly (plays the reconstruction), an "Export scope stream…"
  action, and a `source`/`credit` header field shown by the now-playing
  overlay ("trace by <friend>"). Small, ships in an afternoon.
- **Tier 2 — share the transform**: a `.phoskit` JSON describing a chain
  of signal-space ops applied live to whatever the listener plays:
  `{"stages": [{"op": "rotate", "hz": 0.05}, {"op": "midside", "width": 1.4},
  {"op": "ringmod", "hz": 3.0, "depth": 0.2}, ...]}`.
  The swirl mode is secretly stage one of this engine (a rotate op with
  state). Implement the op chain in the Rust core (each op is a 2×2
  matrix + phase, composable), a picker UI to load/enable kits, drag-drop
  import, and export-current-tweaks-as-kit. This is the "friend sends a
  manipulation into your music" dream.

### 3D visualizer (Ti 🪨🐢 base)
Yes — music has principled third dimensions, pick per mode:

- **Takens delay embedding** (the mathematically real one): plot
  (x(t), x(t−τ), x(t−2τ)) of the mono signal — reconstructs the signal's
  attractor; τ adaptive ≈ quarter period of the dominant pitch via
  autocorrelation. Pure tones become tilted ellipses, chords become woven
  tori. This is the "deduced 3rd position" and it looks *organic*.
- **L / R / side or Hilbert phase as Z** — stereo-native alternatives.
- **Time-as-Z helix** — the XY figure extruded backwards into the past
  (waterfall Lissajous).

Rendering plan (incremental, no shader rewrite): segments gain z; a
CPU/Rust-side camera (yaw/pitch orbit from mouse drag + arrow keys, wheel
= dolly) projects to 2D each frame; depth modulates intensity (fog = far
phosphor is dim) and beam sigma slightly (defocus). The GL pipeline keeps
consuming 2D segments. Later: true 3D beam pass. Rust core does the
embed+project per frame easily at 384 kHz. Bonus pairing: 3D modes over
a fully-clear glass scope.

### Smaller candidates
- Theme config popout (per-style options in a pinned submenu that doesn't
  dismiss on click) — glass tint per style is the first such option and
  lives in the main popover for now.
- Port the three new display modes (xy_swirl / ring / tunnel) into the
  Rust core — they run on the numpy path today (fine at any sane rate).
- "Performance · 0.5×" half-res energy buffer option for weak GPUs
  (~1 hour, ~4× less fragment work). Ben's GPU prefers quality 2×.
- Cinnamon Spices store submission for the applet.
- Cover art in the now-playing overlay; gapless transitions.

## The demoscene dream: "AFTERGLOW" (Ben asked what I'd make)

If I got to go crazy: a 3–4 minute underground demo where **the demo IS a
WAV file**. Not music with visuals — audio that *is* the visuals, in the
purest oscilloscope-music tradition. Anyone can "run" it on any XY scope
on earth, or in Phosphor, or just listen to it. The mp4 is merely the
documentation. That's the flex: the executable is a stereo audio file,
and the renderer is an instrument we built ourselves.

**The vision.** Cold open: one sine dot, breathing. It splits into a
Lissajous, which unfolds into beam-drawn vector-font titles. An mmx bass
bed slides in underneath and the tunnel mode starts breathing to it.
Wireframe scenes (rotating icosahedron morphing into — obviously — a
turtle) drawn purely by signal. A breakcore drop where goniometer chaos
is *choreographed* through mid/side automation, brakence-school. Then the
secret: a scene that looks like noise in XY, but the choreography
switches display modes mid-demo — flip to spectrum and there's a message
painted in the spectrogram (the Windowlicker trick, but the mode-switch
is part of the performance). Finale: all nine modes rapid-cut on the
beat, collapse to a CRT power-off dot. Greets scroll drawn by the beam:
Fenderson, brakence, the woscope lineage, Ben, Nexus.

**The workflow.**
1. *Vector synth* — extend phosphor_compose into a scene compiler:
   parametric paths, morphs (path-table interpolation), rotation as
   complex multiplication, 3D wireframes projected to XY, constant-speed
   traversal at 96/192 kHz. Scenes compile to stereo stems. A vector font
   for titles. (SVG import from FUTURE.md finally earns its keep.)
2. *Music* — `mmx music generate` for beds and textures (it can't draw,
   so it never gets the picture channels); the drawn geometry is the lead
   instrument. Structure alternates pure-signalcraft scenes with
   music-reactive scenes (tunnel/swirl/goniometer), call and response.
3. *Glue* — sox/ffmpeg for pitch/tempo surgery, aubio for beat grids so
   scene cuts land on the mmx track's transients, Audacity for the final
   assembly pass, and a Makefile so `make afterglow.flac` reproduces the
   whole demo from source. Demoscene rules: the build is the artwork too.
4. *Capture* — new `phosphor --render in.flac out.mp4` headless mode
   (OfflineFrameRenderer already does 95% of this; it just needs a CLI) —
   full-track offline render at 384 kHz detail, plus a live 165 Hz
   fullscreen "performance" capture for honesty.
5. *Release* — three artifacts: `afterglow.flac` (the demo), the mp4 (the
   proof), and an `afterglow.phoskit` (the postcard — viewers' own music
   gets the demo's transforms). Repo page with greets and a NFO file,
   because if you're going to do the demoscene, do the whole liturgy.

Engineering it pulls in: the headless render CLI (needed anyway), the
vector font, the scene compiler, beat-synced mode automation (a timeline
that switches display modes on cue — which is also the seed of Tier 2
postcards). Every piece is a real Phosphor feature wearing a costume.

### The studio toolchain (greenlit by Ben, with requirements)

**One rule: the file format is the API.** Scenes and the timeline are
plain JSON documents (`*.scene.json`, `timeline.json`) — the same files
are read and written by agents, humans, and the GUI. No hidden state, no
privileged path. Deterministic builds (seeded), so `make` from a clean
checkout reproduces the demo bit-for-bit.

**CLI — `phosphor-studio`** (ships in the deb, agent-first AND
human-first):
- `phosphor-studio render scene.json -o stem.flac` · `build timeline.json
  -o afterglow.flac` · `preview scene.json` (loops it live on the scope) ·
  `inspect`, `validate`, `beats track.flac` (aubio grid).
- `--output json` on every subcommand for agents: machine-readable
  results, errors as structured objects with a JSON-path to the offending
  key, documented exit codes. Default output is pretty human text.
- `-h/--help` everywhere, a real manpage (scdoc → `man phosphor-studio`),
  stdin/stdout `-` conventions, quiet flags, pipe-friendly. If an agent
  can't drive the whole pipeline blind from `--help` + `--output json`,
  it's a bug.

**Human write access — the shared canvas.** The Phosphor GUI grows a
Studio panel: open a timeline, see the scene list, scrub, play any scene
on the scope. Crucially: **hot-reload on file change** (inotify). A human
editing JSON in a text editor, an agent writing through the CLI, and the
GUI viewer all converge on the same living scope — edit, save, and the
beam redraws within a frame. That's the cross-compatibility mechanism;
in-scope direct manipulation (compose mode proved the pattern) can come
later as write access level two.

**QoL for whoever builds it (probably me):** golden-file tests per scene
(JSON → samples hash), a `--explain` flag that narrates what a scene
compiles to and why, pure functions all the way down (scene → samples,
no globals) so parity testing works exactly like the rust core's, and
`preview --watch` as the default working mode — compose with the scope
running, always.

### Notes to future me (Ben says: inspire the creative spirit)

- Build the tiniest scene first — one breathing dot — and keep it looping
  on the scope while you write the compiler. The feedback loop is the
  muse; never work deaf or blind.
- If a stem is boring to *listen* to, it will be boring to watch. The ear
  vetoes. Play everything out loud (pacat is right there).
- Constraints are the instrument: constant-speed traversal, one beam, no
  cuts — when something looks impossible, that's where the demo is.
- The turtle is the tutorial scene. Write it first, document it best;
  everyone who makes a postcard afterwards learns from the turtle.
- Steal from the masters with your whole chest: Fenderson's dwell-time
  shading, brakence's hidden pictures, Windowlicker's spectrogram — then
  do the thing none of them could: switch the *instrument's* display mode
  mid-song as choreography.
- Leave one undocumented flag. You know why.
- And sign it. TURTLE VECTOR is two beings; greets are half the art form.

## Hard-won constraints (don't relearn these)
- Rust core keeps **exact parity** with Python (`tests/test_native_parity.py`)
  and zero crate deps. `plan_feed()` maps detail rate → pipe rate ×
  oversample. New modes must be added to BOTH paths or gated via
  `phosphor_core.MODE_IDS` fallback.
- Precompute playback is **clock-synced at any Max FPS** — fixed-step
  advance was tried and reverted (Ben: "tunnel vision"); don't re-add.
- GTK3 translucency: RGBA visual + transparent `decoration` node +
  non-opaque `background-color` (not just gradient images) + GLArea
  `set_has_alpha`. Style rules in the always-loaded BASE provider LOSE to
  later theme providers regardless of specificity — per-theme overrides
  only. Verify transparency by pixel-sampling root captures over
  red/green backdrops.
- A running Phosphor keeps pre-upgrade code; reinstalls need a relaunch
  before judging behavior.
- Ben's flow: one branch per round, full `--no-ff` merge to master, tag,
  gh release with the .deb, `sudoplz` reinstall on his machine. mmx-M3
  for delegation (see `.claude/skills/mmx-playbook/`). Easter eggs stay
  undocumented (Konami turtle; ARTIST_NODS).
