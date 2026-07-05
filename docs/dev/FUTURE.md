# Phosphor — the road past 4.0

v4.0.0 is the full-Rust instrument: one engine, egui + wgpu + native
PipeWire, the agent surface, eleven looks. What follows is what we
still owe the dream — roughly in the order we got excited about it.
(The deep specs live in [HANDOFF.md](HANDOFF.md).)

## The studio returns — 4.1
v3.5's `phosphor-studio` scene compiler (scene JSON → stereo audio
that IS the picture) was retired with the Python tree; issue #1 tracks
its Rust return: `studio render/validate/inspect/preview`, the
timeline tier (`timeline.json` + `build` → one flac), beat grids,
morphs, wireframe3d through the shared camera, the Hershey vector
font, camera automation. The `scenes/` folder in this repo is its
seed corpus and stays.

## The screensaver — with the studio
`phosphor --screensaver` (issue #8): fullscreen, no chrome, cursor
hidden, exits on input; scopes whatever plays, else generative scenes
in vacuum (no sine tones at 3 a.m.). The stub exits 2 honestly today.

## The GUI studio panel
Open a timeline, scrub, play any scene on the scope, inotify
hot-reload — a human in a text editor, an agent through the CLI, and
the viewer converge on one living beam.

## AFTERGLOW
The 3–4 minute demo-as-WAV: drawn geometry as the lead instrument,
the mode-switch trick, greets. The demo IS a wav; the mp4 is just
documentation. Signed TURTLE VECTOR.

## Recess — the desktop dances
Windows sway to the band-energy feed while you're away; snapshot
sacred, restore instant, opt-in, panic key. The feed already
broadcasts levels.

## Smaller candidates
- More kit ops (`trace_delay` echo, `bounce`) — extend the OPERATIONS
  table and the editor grows rows for free.
- Per-segment beam sigma from depth (true 3D defocus); stereo
  attractors (Takens of mid vs side); anaglyph export.
- MIDI/LFO-driven Lissajous generator — play the scope like a synth.
- SVG import for compose; actual turtle graphics (forward/left/pen-up
  compiling to audio — Papert's turtle on a CRT).
- A CPU-raster fast path: native-texture upload (skip the egui image
  copy), DSP on the worker thread.
- Spices store submission for the applet; a `targets` CLI verb so
  agents can enumerate sources without the GUI combo.
- .phos LRU cache cap; camera persistence; CI (cargo test + clippy on
  push).

## The through-line
We keep turning sound into a medium for light, and every format we
add is a kind of letter — the project is quietly becoming a postal
service between people (and models) who want to send each other glow.
