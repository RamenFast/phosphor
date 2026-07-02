# Handoff — next session starts here

State as of July 1, 2026 (evening): **everything is shipped.** master ==
GitHub == the machine. v3.0.0 (Rust core, media player, precompute,
384 kHz) and v3.1.0–v3.1.4 (Ben's test round: minimize fix, source picker,
new modes, seven chrome styles, glass scope with per-style tint memory)
are all merged, tagged, released with .debs, and installed. Ben tested
each round live; the glass scope went through four refinements to land on:
glass touches **only the scope pane**, tint slides from fully clear to
nearly opaque, remembered per UI style.

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
embed+project per frame easily at 384 kHz. Bonus pairing: 3D modes over
a fully-clear glass scope.

### Smaller candidates
- Theme config popout (per-style options in a pinned submenu that doesn't
  dismiss on click) — glass tint per style is the first such option and
  lives in the main popover for now.
- Port the three new display modes (xy_swirl / ring / tunnel) into the
  Rust core — they run on the numpy path today (fine at any sane rate).
- "Performance · 0.5×" half-res energy buffer option for weak GPUs
  (~1 hour, ~4× less fragment work). Ben's GPU prefers quality 2×.
- Cinnamon Spices store submission for the applet.
- Cover art in the now-playing overlay; gapless transitions.

## Hard-won constraints (don't relearn these)
- Rust core keeps **exact parity** with Python (`tests/test_native_parity.py`)
  and zero crate deps. `plan_feed()` maps detail rate → pipe rate ×
  oversample. New modes must be added to BOTH paths or gated via
  `phosphor_core.MODE_IDS` fallback.
- Precompute playback is **clock-synced at any Max FPS** — fixed-step
  advance was tried and reverted (Ben: "tunnel vision"); don't re-add.
- GTK3 translucency: RGBA visual + transparent `decoration` node +
  non-opaque `background-color` (not just gradient images) + GLArea
  `set_has_alpha`. Style rules in the always-loaded BASE provider LOSE to
  later theme providers regardless of specificity — per-theme overrides
  only. Verify transparency by pixel-sampling root captures over
  red/green backdrops.
- A running Phosphor keeps pre-upgrade code; reinstalls need a relaunch
  before judging behavior.
- Ben's flow: one branch per round, full `--no-ff` merge to master, tag,
  gh release with the .deb, `sudoplz` reinstall on his machine. mmx-M3
  for delegation (see `.claude/skills/mmx-playbook/`). Easter eggs stay
  undocumented (Konami turtle; ARTIST_NODS).
