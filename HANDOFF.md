# Handoff — v3.1 branch, ready for Ben's light testing

Branch `v3.1` holds everything from the polish round, unmerged so you can
test first. When you're happy: merge to master, tag, release (the .deb
build now picks up the icon + native core automatically).

## Test me (5 minutes)

1. **Taskbar minimize/restore ×6 while playing** — the blank-screen bug's
   root cause (silent GPU buffer allocation failure at 3× supersampling)
   is fixed: allocations are verified, retried, and quality sheds itself
   under VRAM pressure instead of going black. Also: consider GPU quality
   **2×** — 3× at your window size is ~100 MB of energy buffers and most
   of your GPU struggle.
2. **Pick a source while a track plays** (your Spotify case) — the picker
   now switches to live capture; the re-scan button no longer touches
   playback.
3. **AMOLED scope switch** on Custom and White themes — now a true pixel
   black (Custom honors the switch; dither is gated at black).
4. **Settings gear** — the scope scoots left while the popover is open.
5. **New modes** — XY · swirl, Ring · oscillogram, Spectrum · tunnel (app
   + panel applet). Try brakence on swirl.
6. **New styles** — Bloom · neon, Stonework 95, Aero glass, Ice Blue ❄
   (file picker icons now forced-visible). New icon installed too.
7. There are two easter eggs. One listens for a famous sequence of keys
   while the scope has focus. The other knows which artists matter here.

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
embed+project per frame easily at 384 kHz.

### Smaller deferred items
- Theme config popout (per-style options in a pinned submenu that doesn't
  dismiss on click) — wanted once styles grow options.
- Applet: ship new modes needs a reinstall (`applet/install.sh` done on
  this machine already).
- AMD/GPU note: the beam shader already runs on the stream processors;
  the honest wins are quality 2×, the FBO fix (done), and possibly a
  half-res energy buffer option ("Performance · 0.5×") — a compute-shader
  port would not beat the current instanced path. Ballpark: 0.5× option ≈
  1 hour, ~4× less fragment work; ROCm/compute ≈ days, for ~nothing.

## Answers logged elsewhere
- Precompute is **frame-rate-free**: the cache stores the reconstructed
  sample stream, not frames — display frames are cut at draw time by the
  playback clock, so one cache serves any fps/gain/window/mode. "Done"
  means the whole track's stream is on disk (progress % is decode
  progress), never "done up to some framerate".
- mmx/M3 field notes → `.claude/skills/mmx-playbook/` (+ copy for Nexus
  in `~/.hermes/skills/claude-fable-mmx-playbook/`).
