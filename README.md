# Phosphor

A software XY oscilloscope for everything your PC plays — built to watch
"oscilloscope music" (Jerobeam Fenderson et al.) draw its hidden pictures,
and to visualize any system audio the rest of the time.

![concept] In XY mode the left channel moves the beam horizontally and the
right channel moves it vertically. Scope music is composed so this traces
actual drawings. Beam brightness falls with beam speed (like a real CRT),
which is what makes the images look right.

## Run

```bash
python3 ~/Dev/ClaudeWorkspace/phosphor/phosphor.py
```

No dependencies beyond what Mint ships: PyGObject (GTK3), pycairo, and
`parec` from pulseaudio-utils. Audio is tapped from the PipeWire/PulseAudio
monitor of the default output, so nothing needs rerouting — just play the
music anywhere (YouTube, mpv, …) and watch.

## Controls

| Control | What it does |
| --- | --- |
| ⏻ Live | Toggle capture. Off = stream closed, render loop stopped, ~0% CPU. |
| Source | Any output's monitor (`OUT`) or microphone (`IN`), refresh button re-scans. |
| XY / Waveform | Scope-art mode vs. conventional dual-channel trace. |
| Gain / Glow / Beam | Deflection scale, phosphor persistence, beam brightness. |
| Mini | Borderless always-on-top square — drag to move, double-click to restore. |
| Keys | `Space` capture · `M` mini mode · scroll = gain · `Q`/`Esc` quit. |

## Resource behavior

- Capture off: parec killed, render loop removed → effectively zero cost;
  PipeWire re-suspends the monitor source.
- Armed but silent: silence is detected by content and rendering stops once
  the glow fades (<1% CPU measured).
- Live rendering: ~25% of one core (cairo at 60 fps).

## Things to try

- Jerobeam Fenderson — *How To Draw Mushrooms* (the classic), and
  https://oscilloscopemusic.com for whole albums made for this.
- Switch the source to your Samson Q2U and hum — Lissajous figures live.

## Launcher (optional)

Create `~/.local/share/applications/phosphor.desktop`:

```ini
[Desktop Entry]
Type=Application
Name=Phosphor
Comment=XY oscilloscope for system audio
Exec=python3 /home/ben/Dev/ClaudeWorkspace/phosphor/phosphor.py
Icon=utilities-system-monitor
Categories=AudioVideo;Audio;
```
