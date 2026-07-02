# Phosphor manual

Everything the [README](../README.md) doesn't cover. Phosphor 3.0.

## What to scope

The source picker offers three kinds of target:

- **APP** — one playing application (game audio without the music player,
  or vice versa). App targets are remembered **by application name**, not
  stream number: when Chrome finishes a song its stream dies, and Phosphor
  waits up to 3 minutes for that same app to play again and re-grabs it
  instead of dumping you onto another source.
- **OUT** — everything a given output plays (default: your default output).
- **IN** — microphones.

The speaker/mic/app icon beside the picker shows what kind of target is
selected; the refresh button re-scans.

## The built-in player

Skip capture entirely: **open an audio file** (`O`, the folder button, the
right-click menu, or drop files anywhere on the window) and Phosphor decodes
it with ffmpeg, plays it out loud, and scopes it directly.

- **Playlist** — opening a file discovers every track in its folder;
  dropping several files makes them the queue. The side panel (`L` or the
  headerbar list button) shows it: double-click plays, the current track is
  highlighted, and the header has **shuffle** and a cycling **repeat**
  (off / all / one) that also steer auto-advance.
- **Transport** — ⏮ ⏯ ⏭ in the title bar, a seek slider with elapsed/total
  readout, and a per-stream **volume** button (moves only Phosphor's own
  playback stream via PulseAudio, never the whole system).
- **Track info** — artist/title/album fade into the corner when the song
  changes (toggle in settings). This also works for music Phosphor *isn't*
  playing: an MPRIS watcher notices track changes in browsers, Spotify,
  etc. while you scope them.
- **MPRIS** — Phosphor itself appears as `org.mpris.MediaPlayer2.phosphor`:
  media keys, sound applets, and `playerctl` control play/pause/next/
  previous/seek, with metadata and live position published.
- Pause freezes decode and playback in place (SIGSTOP — zero CPU), and the
  Live button doubles as sound on/off for the loaded track.

## Precomputed scope streams

For slow machines, or detail rates beyond what the live pipeline sustains:
flip **Precompute files** in settings and opened tracks are decoded and
sinc-reconstructed once, in the background, into
`~/.local/share/phosphor/precomputed` (a compact int16 stream keyed by track
content and detail rate).

While a precomputed track plays, the audible pipe drops to 48 kHz and the
scope reads the stream by the playback clock — no realtime reconstruction,
and a slow frame traces more audio late instead of dropping samples. Seeks
are an index jump. Snapshots and clips re-render at matching detail.

Disk cost is real: ~92 MB per track-minute at 384 kHz, proportionally less
at lower rates. The right-click menu offers one-shot precompute for the
current track and **Clear precomputed streams** with a size readout.

## Compose (draw your own oscilloscope music)

The scope, run in reverse. Hit the ✏ pencil (or `D`), draw a shape, release
— Phosphor resamples your path into a constant-speed audio loop (left
channel = X, right channel = Y), plays it out loud, and the scope draws it
back.

- **scroll** retunes the loop frequency (20–400 Hz): same shape, new pitch
- draw again to replace the shape; `Esc` or the pencil exits
- right-click → **Export drawing as WAV** writes 10 s to `~/Music/Phosphor/`
  — that file draws your shape on *any* XY oscilloscope
- snapshots and clips work while a loop plays

## Modes

| Mode | What it's for |
| --- | --- |
| XY (scope art) | Oscilloscope music. The real deal. |
| XY · goniometer | Ordinary songs: raw XY collapses stereo music into a diagonal line; rotated 45° the mono energy stands upright and stereo width blooms sideways. |
| XY · dots | The same XY field as discrete sample dots — shows where the beam *dwells*. |
| Waveform | Dual trace with rising-edge triggering so pitched sounds hold still. |
| Spectrum | Log-frequency bars, fast attack / phosphor fall. |
| Spectrum · radial | The same analysis swept around a circle, bass at twelve o'clock. |
| XY · swirl | The goniometer with a slowly revolving stereo field — one orbit every ~18 s. |
| Ring · oscillogram | The waveform bent around a circle, one ring per channel, trigger-locked. |
| Spectrum · tunnel | Concentric rings, bass innermost, each breathing with its band. |

## Settings reference

The gear popover is organized into sections:

- **Renderer** — GPU (CRT beam simulation in shaders, recommended) or CPU
  (cairo); GPU quality up to 3× supersampling; CPU resolution down to 50%.
- **Scope** — **Scope detail** (48/96/192/384 kHz feed: higher rates trace
  the true curves *between* samples, so fine scope-art detail stops washing
  out), **Precompute files**, beam **Focus**, **Auto gain** (autosize: gain
  follows the signal's peak — instant attack when something would clip
  off-screen, slow glide release), **Grid**, **AMOLED scope** background, and **Glass scope** — a
  translucent scope pane (the window opens a smoked panel and the beam's
  own light stays opaque), so the trace floats over your desktop; needs a
  compositing WM, auto-enabled with the Aero glass style, wonderful in
  the mini view.
- **Appearance** — themes (P7 Green, Amber, Ice Blue, White, Vaporwave,
  Red Phosphor, Ultraviolet, Solar Gold, Cyan Tube, Custom with two color
  pickers), UI style (System / Dark / Ice Blue ❄ / AMOLED pink / Bloom neon /
  Stonework 95 / Stonework · bloom / Aero glass — Aero is genuinely
  translucent when a compositor is running), pin button.
- **Player** — the track-info overlay.
- **Performance** — **Max FPS** (Monitor follows the display's refresh
  rate; presets to 480 or type anything up to 1000) and the FPS overlay,
  which reads like `GPU·rs · 165 fps · 0.3ms py · max 9ms`: renderer and
  signal engine, achieved rate, Python time per frame, and the worst gap
  between frames (a spike there means the main loop stalled).

Everything is remembered in `~/.config/phosphor/settings.json`, including
window/mini geometry and whether you quit in mini mode.

## The signal engines

The samples→segments math runs on the fastest available engine:

1. **rust** — `libphosphor_core.so` (built from `core/`): all modes plus a
   polyphase windowed-sinc oversampler, so 96/192/384 kHz detail is
   reconstructed in-process from a 48/96 kHz pipe instead of piping
   full-rate audio through PulseAudio.
2. **numpy** — vectorized Python, the previous fast path.
3. **pure python** — always works, keeps the package dependency-light.

The FPS chip shows which one is active (`·rs` = rust). Build the core with
`cd core && cargo build --release`; `packaging/build-deb.sh` bundles it
automatically when cargo is available.

## Mini mode, fullscreen, exports

- **Mini** (`M`) — borderless always-on-top square. Drag to move, drag the
  bottom-right corner or Ctrl+scroll to resize (140–1000 px), double-click
  to restore, right-click for the full menu.
- **Fullscreen** (`F11`) — chrome-less scope; compositors hand fullscreen
  windows direct scanout, so this is also the path to the monitor's full
  refresh rate.
- **📷 / ⏺** — snapshot to `~/Pictures/Phosphor`, or the last 10 s as mp4
  *with sound* to `~/Videos/Phosphor`. Both re-render the captured audio
  offline through the same signal path, so exports look exactly like the
  screen did.

## Keys

`Space` capture · `O` open file · `L` playlist · `D` compose · `M` mini ·
`S` snapshot · `C` clip · `P` pin · `G` grid · `F` fps · `F11` fullscreen ·
scroll = gain (pitch while composing) · `Q`/`Esc` quit.

## Resource behavior (measured)

| State | CPU |
| --- | --- |
| Capture off | ~0% (parec killed, render loop removed) |
| Armed but silent | <1% (silence detected by content) |
| Live, GPU renderer | ~10% of one core (less with the rust core) |
| Live, CPU renderer | ~25–35% of one core |

## Panel applet (Cinnamon)

`applet/install.sh` installs a live vectorscope for your panel, fed by a
headless helper that reuses the app's exact capture + signal path (rust
core included when built). Hover popup, all six modes, theme-follow or
Phosphor colours, AMOLED background, refresh rate up to 480 fps, a CRT
power-off toggle with a collapse animation, and a choice of how the display
comes up at login: always on, always off, or remembering its last state.
See [`applet/README.md`](../applet/README.md).
