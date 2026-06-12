# Phosphor — future features

Agreed-on ideas, roughly in the order we got excited about them.

## Draw-your-own oscilloscope music ("the inverse")
A compose mode: sketch a shape on the scope (or import an SVG), Phosphor
resamples the path into a closed loop at audio rate — left channel = X,
right channel = Y — and plays/exports it as a WAV that draws that shape on
any oscilloscope, including itself. Draw a mushroom, hear the mushroom.
- path resampling needs constant *speed*, not constant parameter, so the
  beam brightness stays even
- loop at ~50–200 Hz; pitch control = loop frequency
- export to WAV + live preview while drawing

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
