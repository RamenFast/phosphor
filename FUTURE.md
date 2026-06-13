# Phosphor — future features

Agreed-on ideas, roughly in the order we got excited about them.

## Draw-your-own oscilloscope music ("the inverse") — ✅ shipped in 2.4
Compose mode: pencil button / `D`, draw on the scope, release to hear it.
Constant-speed resampling, scroll-to-retune (20–400 Hz), live preview
while drawing, seamless loop playback through the existing file pipeline
(so snapshots/clips of drawings work), WAV export to ~/Music/Phosphor.
Still open from the original idea:
- import an SVG instead of drawing by hand
- multi-stroke shapes (needs retrace-blanking decisions)

## Cinnamon panel applet
A true panel-embedded mini scope (St/Clutter canvas applet that reads the
same monitor source), click to open the full window. The mini window covers
this for now.

## .deb package
Proper packaging once features settle: `/usr/bin/phosphor-scope`,
icon/desktop in system paths, dependencies declared.

## Multi-app mixing
Per-app capture currently scopes one app at a time (one parec per
sink-input). Mixing several selected apps means summing multiple streams —
doable, just needs a small mixer in the capture layer.

## Misc candidates
- GL bloom pass (bright-pass + blur) for an even juicier beam
- triggered single-shot capture ("freeze when something draws")
- fullscreen / screensaver mode
- MIDI or LFO-driven Lissajous generator for live playing
