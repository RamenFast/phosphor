# Phosphor

A software XY oscilloscope for everything your PC plays — built to watch
"oscilloscope music" (Jerobeam Fenderson et al.) draw its hidden pictures,
and to make any system audio look good.

In XY mode the left channel moves the beam horizontally and the right
channel moves it vertically; scope music is composed so this traces actual
drawings. Beam brightness falls as the beam moves faster and the phosphor
decays in two layers (a blue-white flash where the beam lands, a colored
glow that lingers) — P7 phosphor physics, the details that make it look
like the real instrument.

## Run

```bash
python3 ~/Dev/ClaudeWorkspace/phosphor/phosphor.py
```

Or use the **Phosphor** launcher (menu and desktop icon are installed).
No dependencies beyond stock Mint: PyGObject, pycairo, `parec`, `ffmpeg`
for clip export and file playback. The GPU renderer binds OpenGL through
GTK's own libepoxy with ctypes — no PyOpenGL needed.

To build an installable package:

```bash
packaging/build-deb.sh     # -> packaging/dist/phosphor_<version>_all.deb
```

## What to scope

The source picker offers three kinds of target:
- **APP** — one playing application (game audio without the music player,
  or vice versa); these come and go, the refresh button re-scans
- **OUT** — everything a given output plays (default: your default output)
- **IN** — microphones (hum into the Q2U for live Lissajous figures)

Or skip capture entirely: **open an audio file** (`O`, the folder button,
or the right-click menu) and Phosphor decodes it with ffmpeg, plays it out
loud, and scopes it directly — no separate player needed.

## Modes

| Mode | What it's for |
| --- | --- |
| XY (scope art) | Oscilloscope music. The real deal. |
| XY · goniometer | Ordinary songs: raw XY collapses stereo music into a diagonal line; rotated 45° the mono energy stands upright and stereo width blooms sideways. |
| Waveform | Dual trace with rising-edge triggering so pitched sounds hold still. |
| Spectrum | Log-frequency bars, fast attack / phosphor fall. |

## Controls

- **⏻ Live** — capture on/off, with the status readout right beside it.
  Off = stream closed, render loop stopped, ~0% CPU.
- **Title bar** — open file, mini view, pin-above (hideable in settings),
  and the settings gear, sysmon-style.
- **📷 / ⏺** — snapshot to `~/Pictures/Phosphor`, save the last 10 s as
  mp4 *with sound* to `~/Videos/Phosphor`. Both re-render the captured
  audio offline, so exports look exactly like the screen did.
- **Sliders** — Gain / Glow / Beam, each with an editable percent box:
  click and type an exact value. Scroll the scope to zoom gain; the
  graticule grows and shrinks with it, octave-stepped like a volts/div
  switch.
- **⚙ settings** — renderer (GPU/CPU), GPU quality (2× supersampling) and
  CPU resolution selectors, beam Focus (sharper beams keep dense scope-art
  scenes from washing out), themes (P7 Green, Amber, Ice Blue, White,
  Vaporwave, Red Phosphor, Ultraviolet, Solar Gold, Cyan Tube, Custom),
  grid, AMOLED scope background, and UI style (System / Dark / AMOLED
  black chrome).
- **Mini** — borderless always-on-top square. Drag to move, Ctrl+scroll to
  resize (square stays square), right-click for the full menu (modes,
  themes, grid, pin, sizes…), double-click to restore. Window positions,
  sizes, and all settings are remembered in
  `~/.config/phosphor/settings.json`, including whether you quit in mini.
- **Keys** — `Space` capture · `O` open file · `M` mini · `S` snapshot
  · `C` clip · `P` pin · `G` grid · scroll = gain · `Q`/`Esc` quit.

## Resource behavior (measured)

| State | CPU |
| --- | --- |
| Capture off | ~0% (parec killed, render loop removed, PipeWire suspends the source) |
| Armed but silent | <1% (silence detected by content; monitors deliver zeros, so an empty-buffer check doesn't work) |
| Live, GPU renderer | ~10% of one core |
| Live, CPU renderer | ~25–35% of one core (signal math + phosphor decay now run on a worker thread, so the UI stays smooth and slow frames drop instead of queueing) |

## Things to try

- Jerobeam Fenderson — *How To Draw Mushrooms*; whole albums at
  https://oscilloscopemusic.com
- Put on any normal song in **XY · goniometer** and watch the stereo image dance.

## Future

See [FUTURE.md](FUTURE.md) — top of the list: draw a shape, and Phosphor
generates the audio that draws it (make your own oscilloscope music).
