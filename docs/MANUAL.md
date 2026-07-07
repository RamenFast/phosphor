# Phosphor 4.0 — the manual

Everything the instrument does, in the order you'll meet it. The
essentials also live inside the app: the book icon, left of the gear.

## What to scope

The combo at the toolbar's right edge picks the source, grouped:

- **OUTPUTS** — everything playing on a sink (its monitor).
- **APPLICATIONS** — one program's audio, captured natively by stream
  serial. What you pick is what the beam hears, nothing else.
- **MICROPHONES** — line/mic inputs.

Picking a source **starts scoping it immediately** — even from idle.
If a file was playing, it pauses (press Space to take the beam back).
The combo's text and checkmark always show **what actually feeds the
beam**; when a silent source has been quiet past the sleep window the
scope says `no signal · <source>` instead of leaving you a black
mystery. **⏻ LIVE** toggles capture; off costs ~0% CPU.

**Mix several apps** ("Mix several apps…" at the bottom of the combo):
tick running apps and every stream folds into one beam — solo the
synth, keep the game, lose the call. Agents:
`phosphor ctl target mix:app:one+app:two`.

### The light (vacuum mode)

The ⊘ controls play sound as **light only**: the track (transport ⊘)
or a whole app (right-click → *Vacuum this app*) is routed into a
silent sink, plays full-tilt, and arrives only on screen. Sound can't
cross a vacuum; a CRT is a vacuum tube. Preview loud things at 3 a.m.,
scope a muted game, watch a second player silently. **The restore path
is sacred**: releasing the vacuum — or crashing, or killing Phosphor
with -9 — always puts the stream back where it was (every launch
sweeps stale vacuum sinks).

*The runner-up name for this mode was "pantomime" — sound acting out
its shapes with the volume stripped away. It lost to physics, but it
was too good to leave undocumented.*

## The player

Open a file (`O`, the folder button, drag-and-drop, or
`phosphor song.flac` from anywhere — a second launch forwards the file
to the running window and focuses it). The folder becomes a playlist:
**gapless** transitions, shuffle, repeat off/all/one, seek with a
250 ms debounce, per-stream volume (cubic taper, % readout), embedded
cover art in the transport, artist/title fading in on the scope.

Space is play/pause — and it arbitrates the beam: resuming a track
takes the beam back from capture; picking a capture source pauses the
track. One source of truth, no double-feeds.

**When the beam scopes another player** (Spotify, a browser tab…), the
transport shows *that* player — title, artist, "via Spotify" — and the
buttons **drive it** over MPRIS. On whole-output capture it links
whoever is actually playing.

**MPRIS both ways**: media keys drive Phosphor; other apps see stable
track ids, real Seeked signals, a writable Volume.

**Track notifications**: a systemwide toast with the album art on
every track change — your files (embedded art) and the scoped player
(fetched art). Settings → Appearance → *Track notifications*.

## Eleven modes

`M` or the toolbar combo: **XY (scope art)** · **XY · goniometer**
(the 45° mid/side rotation — stereo width lives here) · **XY · swirl**
(slow rotation) · **XY · dots** (unconnected samples) · **Waveform**
(triggered) · **Ring** (oscillogram on a circle) · **Spectrum** ·
**Spectrum · radial** · **Spectrum · tunnel** (breathing depth) — and
two true-3D views: **3D · attractor** (Takens delay-embedding, τ
chasing the dominant pitch — pure tones are tilted ellipses, chords
weave tori) and **3D · time helix** (the XY figure extruded into the
past). Drag orbits the 3D views, scroll dollies, arrows nudge; left
alone they drift. Depth dims the beam like far phosphor.

## Sliders — real units

- **Gain** — deflection scale, `×1.00` is unity. Scroll on the scope
  does the same. The `auto` tag means auto-gain is breathing it.
- **Glow** — phosphor persistence, in %.
- **Beam** — brightness budget, `×N`: higher keeps fast strokes
  visible (a faster beam is dimmer, like the real tube).
- **Focus** (settings) — beam width in px.
- Every readout is mono, draggable, and type-able (double-click).

## Beam colors — the cycle

The scope's phosphor is a **Beam** preset (P7 Green, Amber…) or
**Custom**. Custom grew a cycle in 4.1: up to **three colors** that
crossfade into each other on a **transition timer** (default 3 s per
leg, eased so the beam lingers on your picks before gliding on). One
color stays static, two ping-pong, three walk the ring. Flash and
background derive from the moving beam, the grid stays your pick, and
chrome accents that follow the beam (Blossom Dark, Afterglow) ride
along. Snapshots, clips, and `phosphor render` reproduce the cycle
exactly — exports re-live the colors you watched. Agents can poll it:
`phosphor probe` carries `beam_cycle.current` while it runs.

Setting the transition **below 1 s** asks for an explicit
photosensitivity confirmation first — rapid whole-scope color flashing
can trigger seizures in people with photosensitive epilepsy. The timer
holds at 1 s unless you accept, and the question returns next launch.

## Compose — draw your own oscilloscope music

`D` (or right-click → Compose): draw on the scope; release and the
shape **is** audio, looping until you leave. Scroll retunes the pitch
(80–400 Hz, constant-speed traversal so corners stay corners).
Right-click → *Export drawing as WAV (10 s)* writes a file any scope
on earth can play. Esc leaves.

## Kits & postcards

A **`.phoskit`** is ~300 bytes of JSON: a chain of signal transforms —
`rotate · midside · ringmod · wobble · matrix · chandelay` — applied
live before every display mode. Drop one on the window to wear it.
The **kit editor** (Settings → Signal kit) builds chains against the
running beam; Save writes a postcard you can send. Three starters
ship: `haunt`, `heartbeat`, `orbit`. Kits are validated on the way in
— a broken kit toasts its error and never lands.

A **`.phos`** is the scope itself, recorded: export the playing track
as a signal postcard (with your credit) and a friend drops it on their
Phosphor — it plays at *your* detail rate, "trace by you" fading in.

## Mini, glass, fullscreen

- **M** — mini: a square, undecorated, always-on-top scope. Drag to
  move (edges snap to the work area), 8-zone edge/corner resize,
  double-click or Esc restores, right-click for the compact menu +
  Align + size presets.
- **Glass** (settings) — the pane goes translucent: the beam draws
  over whatever is behind the window. Tint per theme, 1% steps.
  Pairs dangerously well with mini.
- **F11** — fullscreen: nothing but light, zero chrome.

## Exports

- **S** — snapshot PNG (offline re-render of the last frames: exactly
  what you saw, at full quality) → `~/Pictures/Phosphor/`.
- **C** — the last 10 s as mp4 **with sound** (ffmpeg muxes the
  original audio).
- `phosphor render input.flac out.mp4 [--rate N]` — headless
  full-track render; `.phos` files render too.

## Performance & the nerd HUD

`F` cycles: off → **fps** → **the nerd HUD**: cpu frame ms + p99,
**gpu ms** (real timestamp queries), raster ms (the CPU renderer runs
on its own worker thread — a heavy raster never drags the chrome),
segments/s, scope rate, renderer, resolution, dropped frames.
`--fps-log` prints JSON lines including `{"dip":…}` events for any
frame past 2× the pacing period. `phosphor bench` runs the permanent
regression gates.

Settings → Performance: Max FPS (Monitor follows the panel — 165 Hz
means 165), GPU quality (supersampling), CPU renderer resolution.

## Settings reference

`~/.config/phosphor/settings.json` — every key survives round-trips,
foreign keys are preserved, v3 files migrate untouched. Highlights:
`ui_style` (11 ids — see the picker), `theme_name` (beam phosphor
color; `Custom` + RGB), `custom_beam_color_2`/`_3` +
`beam_cycle_count` + `beam_cycle_seconds` (the color cycle — removed
slots keep their picks), `scope_sample_rate` (up to 384 kHz
reconstruction), `renderer` (`gl`/`cairo`), `max_fps` (0 = monitor,
-1 = uncapped), `track_notifications`, `show_fps_detail`,
`glass_tints` (per-style), `vacuum_enabled`, `kit_path`/`kit_enabled`.

## Keys

| Key | Does |
|---|---|
| Space | play/pause · capture toggle (the beam arbiter) |
| O | open audio file |
| M | mini view |
| F11 | fullscreen scope |
| D | compose (draw) |
| S / C | snapshot / 10 s clip |
| G | graticule |
| F | fps → nerd HUD → off |
| L | playlist panel |
| P | pin above |
| ← / → | seek ±5 s · playlist step |
| ↑ / ↓ | volume |
| Esc | leave compose → fullscreen → mini |
| Q | quit |

(One or two others are for you to find.)

## The agent surface

One binary, station-grade: JSON everywhere, exit codes `0` ok ·
`2` unavailable · `3` bad arguments · `4` runtime, every error with a
`fix`. The GUI serves a control socket at
`$XDG_RUNTIME_DIR/phosphor/ctl.sock` (NDJSON).

- `phosphor probe [--json]` — one-shot live state, including
  `source` (what actually feeds the beam: capture/mix/player/silent).
- `phosphor ctl <verb>` — `play pause toggle stop next previous seek
  volume mode theme ui capture target open raise snapshot clip quit`;
  `target mix:app:a+app:b` folds apps; `snapshot`/`clip` replies wait
  for the file and return its path.
- `phosphor tap` — NDJSON beam stream: per-frame mode, segment count,
  bbox, centroid, peak, a decimated polyline; `tick` heartbeats while
  quiet.
- `phosphor feed` — the applet's segment stream (locked protocol).
- `phosphor kit validate|inspect a.phoskit…` — errors small models
  repair in one round-trip; `docs/phoskit.schema.json` ships.
- `phosphor schema` — the whole surface, machine-readable.
- `phosphor --background` — a full headless GUI on a private display
  (Xvfb): renders, serves the socket, steals nothing.

The worked guide: [AGENTS.md](AGENTS.md).

## The applet

Cinnamon 6.x panel vectorscope, engine-free (it spawns
`phosphor feed`). GNOME/KDE can't load it — the main app runs
everywhere regardless. [applet/README.md](../applet/README.md).

## Troubleshooting

- **Black scope, capture on** → read the on-scope label: the source is
  silent. Pick the output that's actually playing (or the app itself).
- **GPU renderer unavailable** → install Vulkan drivers
  (`libvulkan1`/RADV/etc.); the CPU renderer works meanwhile
  (Settings → Renderer).
- **No mp4 clips / no artUrl covers** → install `ffmpeg` / `curl`
  (Recommends in the deb).
- **A vacuumed app lost its sound after a crash** → launch Phosphor
  once; the startup sweep restores every stray stream. That's the law.
- **Media keys fight another player** → both own MPRIS; the desktop
  routes keys to the most recent. Phosphor's transport drives the
  *scoped* player either way.

---

*Phosphor is GPL-3.0-or-later. Built by Ben & Claude — TURTLE VECTOR.
The beam remembers.*
