# Phosphor — future features

Agreed-on ideas, roughly in the order we got excited about them.

## Draw-your-own oscilloscope music ("the inverse") — ✅ shipped in 2.4
Compose mode: pencil button / `D`, draw on the scope, release to hear it.
Still open from the original idea:
- import an SVG instead of drawing by hand
- multi-stroke shapes (needs retrace-blanking decisions)

## Cinnamon panel applet — ✅ shipped in 2.6
Live panel vectorscope with hover popup, themes, CRT power toggle; 3.0
added power autostart (on/off/remember) and the rust-core feed.
Still open: a true St/Clutter render (vs the current Cairo draw), and
shipping it to the Cinnamon Spices store.

## Native signal core — ✅ shipped in 3.0
`core/` Rust cdylib: all six modes with exact Python parity, polyphase
sinc oversampling (384 kHz detail from a 96 kHz pipe), automatic fallback.
Still open: SIMD-tuning the beam math further; moving the offline clip
renderer's rasterization native.

## Built-in media player — ✅ shipped in 3.0
Playlist panel with shuffle/repeat, drag-and-drop queues, per-stream
volume, now-playing overlay (own files + other players via MPRIS watch),
Phosphor as an MPRIS player (media keys work).
Still open: gapless transitions; reading embedded cover art into the
overlay.

## Precomputed scope streams — ✅ shipped in 3.0
Render-ahead `.phos` cache read by the playback clock.
Still open: an LRU size cap; precomputing a whole folder in one go.

## Shareable scope-art ("signal postcards") — ✅ shipped in 3.2
.phos streams play/export with title + "trace by" credit; .phoskit
transform chains (rotate/midside/ringmod/wobble/matrix/chandelay) run in
both engines with exact parity, composed live in the kit editor, imported
by drag-drop, three starter kits bundled.
Still open: more ops (the beat-synced mode-automation timeline is the
AFTERGLOW seed), kit browsing/sharing beyond files.

## 3D visualizer — ✅ shipped in 3.3
xyz_takens (delay embedding, autocorrelation-adaptive τ) and helix
(time-as-Z) with a shared orbit camera: drag/scroll/arrows + idle drift,
depth fog, numpy path (MODE_IDS gates native fallback per mode).
Still open: porting 3D modes into the Rust core; a true 3D beam pass
(per-segment sigma); persisting the camera.

## Multi-app mixing
Per-app capture currently scopes one app at a time (one parec per
sink-input). Mixing several selected apps means summing multiple streams —
doable, just needs a small mixer in the capture layer.

## Misc candidates
- GL bloom pass (bright-pass + blur) for an even juicier beam
- triggered single-shot capture ("freeze when something draws")
- screensaver mode + the dancing desktop (\"Recess\") — spec'd in HANDOFF.md
- MIDI or LFO-driven Lissajous generator for live playing
