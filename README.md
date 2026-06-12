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
for clip export. The GPU renderer binds OpenGL through GTK's own libepoxy
with ctypes — no PyOpenGL needed.

## What to scope

The source picker offers three kinds of target:
- **APP** — one playing application (game audio without the music player,
  or vice versa); these come and go, the refresh button re-scans
- **OUT** — everything a given output plays (default: your default output)
- **IN** — microphones (hum into the Q2U for live Lissajous figures)

## Modes

| Mode | What it's for |
| --- | --- |
| XY (scope art) | Oscilloscope music. The real deal. |
| XY · goniometer | Ordinary songs: raw XY collapses stereo music into a diagonal line; rotated 45° the mono energy stands upright and stereo width blooms sideways. |
| Waveform | Dual trace with rising-edge triggering so pitched sounds hold still. |
| Spectrum | Log-frequency bars, fast attack / phosphor fall. |

## Controls

- **⏻ Live** — capture on/off. Off = stream closed, render loop stopped, ~0% CPU.
- **📌** — pin the window above others.
- **📷 / ⏺** — snapshot to `~/Pictures/Phosphor`, save the last 10 s as
  mp4 *with sound* to `~/Videos/Phosphor`. Both re-render the captured
  audio offline, so exports look exactly like the screen did.
- **⚙** — renderer (GPU/CPU), themes (P7 Green, Amber, Ice Blue, White,
  Custom with color pickers), grid on/off, AMOLED black background.
- **Mini** — borderless always-on-top square. Drag to move, Ctrl+scroll to
  resize (square stays square), right-click for the menu, double-click to
  restore. Window positions, sizes, and all settings are remembered in
  `~/.config/phosphor/settings.json`, including whether you quit in mini.
- **Keys** — `Space` capture · `M` mini · `S` snapshot · `C` clip · `P` pin
  · `G` grid · scroll = gain · `Q`/`Esc` quit.

## Resource behavior (measured)

| State | CPU |
| --- | --- |
| Capture off | ~0% (parec killed, render loop removed, PipeWire suspends the source) |
| Armed but silent | <1% (silence detected by content; monitors deliver zeros, so an empty-buffer check doesn't work) |
| Live, GPU renderer | ~10% of one core |
| Live, CPU renderer | ~25–35% of one core |

## Things to try

- Jerobeam Fenderson — *How To Draw Mushrooms*; whole albums at
  https://oscilloscopemusic.com
- Put on any normal song in **XY · goniometer** and watch the stereo image dance.

## Future

See [FUTURE.md](FUTURE.md) — top of the list: draw a shape, and Phosphor
generates the audio that draws it (make your own oscilloscope music).
