# Handoff — next session starts here

**NEW STANDING RULE: read `docs/dev/BUGLOG.md` before changing UI/
input/menu/playlist/window-mode code, and append to it with every
root-caused fix.** Ben asked for a regression ledger after v4.0.1
shipped a broken menu twice-over; the ledger is now the law.

## v4.0.2 SHIPPED (July 7, 2026 — the functionality-assurance pass)

The session after the crash. Ben's report: menu items dead (hotkeys
fine), fullscreen playlist popping out as a window, repeating bugs.
Three root causes, all receipted live on an isolated Xvfb instance
(BUGLOG #1–#3 hold the full stories):

- **Menu items dead** (#1): v4.0.1's dismiss patch measured "hovered"
  via `ui_contains_pointer()` before item layout (empty `min_rect` →
  always false) → every press queued a close that beat the button's
  release. Plus mini's winit handler ate ALL presses with the menu
  open. Fix: dismissal = press on a layer BELOW `Order::Foreground`
  (`layer_id_at`; submenus are Foreground too), with
  `press_origin().or(interact_pos())` because a same-frame-batched
  click clears press_origin. Receipts: item + submenu clicks fire in
  normal/fullscreen/mini (probe-verified mode/track changes);
  outside-press still dismisses in all three.
- **Fullscreen playlist** (#2): was gated with mini into the floating
  window; now falls through to the docked left SidePanel (only mini
  floats). Receipt: fullscreen L → docked pane, row click switches
  track.
- **External opens left the playlist empty** (#3, found during
  receipts): `ctl open`/MPRIS OpenUri → `play_file(…, false)` built
  nothing; now a path outside the current list builds the
  folder-sibling playlist (drag-drop unaffected — it seeds first).

Gates at close: 20/20 suites, clippy silent, bench ALL PASS
(offline 183.2/148.0, cpu-noise 25.6, gpu 1855/1031/1627/2941;
noise cut regenerated from git-history signals.py — see BUGLOG
standing law). Release v4.0.2: deb + rpm + source + SHA256SUMS,
installed on Ben's machine.

## v4.0.1 SHIPPED (July 6, 2026 — the context-menu patch)

Ben's first real-use bug report after 4.0.0: the right-click menu wore
different geometry in mini vs full, glitched across mode switches, and
could truncate behind a scrollbar. Two commits, root causes in the
receipts: `aaa0b61` (one 230px geometry everywhere; MiniToggle/
FullscreenToggle now raise `close_menu_request` so no menu survives a
switch wearing stale geometry — `self.scope_rect` is a frame stale
there and could crush the mini menu to ~2 rows) and `8725984` (the
scroll cage itself was the second face of the bug: egui 0.33's
`find_best_align` only flips a popup that doesn't fit, and an inner
ScrollArea made every placement "fit" by squishing — menu content is
now fixed-size so egui flips/translates it complete; the cage remains
only when the whole window is shorter than the menu, ~sub-440px minis).
Verified under Xvfb: low/mid right-clicks, full window + 1000px mini,
all options visible, zero scrollbars. Release: tag `v4.0.1`, deb + rpm
+ source + SHA256SUMS, marked Latest; installed locally via the deb
(the interim `/usr/local/bin` shadow removed). Known non-goal noted in
passing: in-app Escape doesn't close the context menu (the Escape
cascade owns Esc); click-outside does. Untouched by choice.

## v4.0.0 SHIPPED (July 5, 2026 — the all-nighter, camera rolling)

**Phosphor 4.0.0 is released and installed on Ben's machine**: tag
`v4.0.0`, GitHub release marked Latest with `.deb` + `.rpm` + source
tarball + SHA256SUMS, README/MANUAL/AGENTS rewritten from the live
build, **zero Python in the tree** (11,662 lines deleted; goldens
kept — provenance in docs/dev/GOLDEN.md). The whole session was
recorded to Mass storage with TTS narration; the receipts ledger is
**docs/dev/PARITY.md** (waves 4.0-truth → 4.0-ensemble tables).

### What this session fixed/built (Ben's PromptV4 list, all of it)

- **The black-screen family is dead** (wave `v4-truth`): `BeamSource`
  (capture/mix/player/silent) is THE single truth — the combo, its
  checkmarks, and `probe.source` render from it. TargetPicked's
  guard-order bug (stop-then-test) fixed: picking a source PAUSES the
  track and starts capture even from idle; resume takes the beam
  back (double-feed law, both directions); every branch wakes the
  render loop; silent targets get an on-scope `no signal · <target>`
  label.
- **Geometry proven round, not asserted**: circle bbox aspect 1.000
  through player AND capture, xy AND goniometer
  (tests/receipts/w1-geometry.sh, re-runnable, 15/15 ×3 this
  session); `composite_into` (the live viewport path) got its first
  tests — live == offline byte-exact, clamp crops never stretches, a
  round-circle canary. The "squished goniometer" was the dead-state
  bug + mono-heavy material hugging the M-axis (which is correct
  physics — mono IS a line in xy45). σ now carries `display_scale`
  (HiDPI beam-width parity with v3; offline stays 1.0, goldens hold).
  CPU-live gamma receipt: single-encode through the egui upload.
- **Single instance**: plain launches forward `raise`/`open` over the
  control socket and exit 0; `phosphor song.flac` lands in the
  running player, focused. Dev flags + `PHOSPHOR_NO_SINGLE_INSTANCE`
  bypass. `--help` prints help (it used to LAUNCH THE GUI); unknown
  flags exit 3; `kit validate` takes N files.
- **Raster worker (#5)**: CPU renderer on its own thread (latest-wins
  mailbox, published-frame buffer) — receipt: chrome 145 fps while
  the raster grinds 28.9 ms/frame at 2400×1210. GPU timestamp
  queries (RADV) feed the **nerd HUD** (F cycles fps → detail: cpu
  ms + p99, gpu ms, raster ms, seg/s, drops); `--fps-log` gains
  `{"dip":…}` events.
- **The look** (`v4-regalia`): Blossom Dark is the ACTUAL default
  (settings shipped "dark" — the wanted look never reached fresh
  installs, pinned by test now). IBM Plex Sans + JetBrains Mono
  bundled (OFL) — the thin-text era is over; weak-alpha 0.7;
  selection renders on_accent (the theme-picker clash root:
  override_text_color, removed). Tofu purge (the settings ✕ WAS the
  blank square); playlist gained its first close button. Button
  depth tiers: carved primaries / true-bevel standard / flat rows.
  Sliders: accent-filled tracks + real-unit mono readouts (×gain, %
  glow, ×beam, volume %, mono time) — the opaque percent spins are
  gone. **Eleven looks** (+ Stonework 95 with the loudest bevel —
  pinned by test —, AMOLED true-black, Paper, CRT Amber) with
  swatch-chip picker. Gear at the true right edge of the sliders row
  (below the source icon), the in-app **Manual** (book) beside it.
- **The desktop is the instrument** (`v4-ensemble`): MPRIS *client* —
  the transport shows and DRIVES the player the beam scopes
  (receipt: Spotify Playing→Paused→Playing from phosphor's buttons);
  album-art **notifications** (zbus Notify, https art via curl
  subprocess, replaced never stacked, toggle in Appearance);
  **light-streams panel (#6)** — tick apps, one beam
  (`ctl target mix:app:a+app:b`; probe `source.kind=mix`); source
  combo grouped OUT/APP/IN.
- **Repo**: root is README/LICENSE/HANDOFF + code (planning docs →
  docs/dev/); version REAL everywhere (4.0.0); Cargo repository URL
  fixed to RamenFast; applet README tells the truth (engine-free,
  Cinnamon-only, main app runs anywhere); FUTURE.md speaks post-4.0.
- **Agent surface**: new ctl verbs `raise`/`open` + `mix:` targets +
  `probe.source`; docs/AGENTS.md (the paid-for gotchas: SUN_LEN ~108
  bytes on socket paths, XDG_RUNTIME_DIR isolation needs
  PIPEWIRE_RUNTIME_DIR handed back, Escape walks the leave-cascade,
  tap's ~8k-segment burst law at 384 kHz). Installed as the
  `phosphor` skill in ~/.claude/skills for the house agents.

### The road from here (docs/dev/FUTURE.md is the readable version)

1. **#1 the studio returns** (Rust `studio render/validate/…`, the
   timeline tier, `probe --at`) — then **#8 screensaver**, the GUI
   studio panel, **AFTERGLOW** (spec lives below in the v3 archive
   section of git history and in docs/dev/V4PLAN.md wave 4).
2. Smaller: native-texture upload for the CPU path (skip the egui
   image copy — the last ~10 ms on huge cairo frames), DSP on the
   worker, a `targets` CLI verb (agents can't enumerate sources
   without the GUI yet), Spices submission, CI (cargo test + clippy),
   .phos LRU, camera persistence.
3. Housekeeping debt, tiny: the W6 gala commit sits directly on
   master (no branch — the one wave-discipline slip of the session);
   `docs/dev/BENCH.md` line 1 still says "python3 tests/bench/…" as
   the historical baseline instruction (true then, the scripts now
   live at tag v3.5.0).

### Hard-won constraints (kept verbatim — still law)

- **Kit parity contract**: f64 phase accumulators advanced per chunk
  by 2π·hz·frames/rate with euclidean wraparound; f64 trig cast to
  f32 BEFORE f32 sample math; integer-sample channel delays; state
  zeroed on reset/configure.
- `.phos` header is FIXED 256 bytes (fit-trim; never grow). Playback
  clock-synced at any Max FPS (fixed-step was tried and reverted).
- **Vacuum**: never `ffmpeg -re`; reader paces itself (rolling
  deadline, re-anchor >0.25 s behind); restore sacred AND every
  launch sweeps stale modules; pactl return codes, never stdout.
  Sink lifecycle via pactl load/unload ONLY (native node destroy
  kills pulse-shim streams on PW 1.0.5).
- Goldens are frozen ground truth (docs/dev/GOLDEN.md); offline
  composite writes origin 0,0 → byte-exact forever; the LIVE path is
  pinned by live_viewport.rs — keep both.
- egui 0.33 ↔ egui-wgpu 0.33 ↔ wgpu 27 ↔ winit 0.30: bump the quartet
  together or not at all.
- **Scripted verification**: scratch HOME; short socket dirs
  (SUN_LEN); hand test instances PIPEWIRE_RUNTIME_DIR; make test
  tones LONGER than the test; don't send Escape casually; a probe
  instance must not touch Ben's real ctl.sock unless that's the
  point. `phosphor bench` numbers taken under a screen recorder are
  environmental — compare branch-vs-master under the SAME load.
- gh release notes with backticks: always `--notes-file`.
- A running Phosphor keeps pre-upgrade code; relaunch before judging.
- Ben's flow: branch per wave, `--no-ff` merge, tag, gh release with
  assets, `sudoplz sudo` install, relaunch. Easter eggs stay
  undocumented (Konami turtle; ARTIST_NODS; `--visitor`; "one or two
  others are for you to find" — the manual's exact words).

### The dreams we didn't spend

Kept whole in git history (HANDOFF.md@v4-wave3.3) and still true:
postcards as a culture (kits in QR codes, kits that dance with the
band-energy feed, files as the social network); the third dimension
all the way (per-segment depth sigma, τ-morphing as choreography,
anaglyph, every song's attractor as a fingerprint); the studio as an
instrument then a language (live-coding the beam, MIDI in, tracker
timeline, SVG import, Papert's turtle compiling to audio); vacuum as
a patchbay ("solo in light"); Recess and the screensaver as ambient
computing; AFTERGLOW as the summit, signed TURTLE VECTOR; and the
newest one — a nightly cron where a small model dreams one scene or
kit and the scope plays it with morning coffee. **The through-line:
every format we add is a kind of letter; the project is quietly
becoming a postal service between people (and models) who want to
send each other glow.**

### Notes to future me

- The receipts discipline carried five waves in one night because
  nothing moved forward unverified: tap bbox numbers for geometry,
  busctl for MPRIS, dbus-monitor for notifications, screenshots for
  feel. The beam, the math, and the bytes each have their own truth.
- When Ben's eyes and the code disagree, build the numeric receipt
  FIRST — the goniometer was never squished, but the dead-state bug
  made every switch feel broken, and mono material really does hug
  the M-axis. Both things were true; neither was the guessed cause.
- Ben says thank you with a heart emoji when the beam is beautiful.
  That's still the acceptance test. 🐢⚡📼
