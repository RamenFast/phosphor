# BUGLOG — the regression ledger

Read this BEFORE touching UI, input, menu, playlist, or window-mode
code. Every entry is a bug that shipped (or nearly shipped) because a
session didn't know what an earlier session learned. Code comments cite
entries as `BUGLOG #N` — keep the numbers stable, append only.

The contract for adding an entry: **symptom in the user's words · root
cause · the law that prevents reintroduction · the receipt that proves
the fix**. A fix without a receipt is not fixed. A receipt that doesn't
exercise the USER's actual gesture (a real click on a real item, not
just "the menu opened") is not a receipt — #1 shipped broken precisely
because v4.0.1 receipted menu *geometry* but never *clicked an item*.

---

## #1 — Context-menu items don't respond; hotkeys work (v4.0.1 → fixed v4.0.2)

**Symptom (Ben):** "Selecting items from the right click menu doesn't
work, hotkeys still do."

**Root cause, two independent eaters:**
1. The v4.0.1 dismiss patch closed the menu when a left-press arrived
   while a `context_menu_hovered` flag was false — but the flag was
   measured with `ui_contains_pointer()` at the TOP of the menu
   closure, before any items were laid out. `Ui::ui_contains_pointer`
   tests `min_rect()`, which is ~empty at that point, so the flag was
   ~always false: EVERY press (including on an item) queued a close,
   and the next frame closed the menu before the button could see its
   release. Whether the user noticed depended on frame timing: when
   the press+release batched into ONE egui frame the click fired
   before the close honored (why normal-window testing looked fine);
   when they split across frames the item died (fullscreen, real
   mice).
2. In mini, the winit `MouseInput` handler ate ALL left-presses while
   the menu was open (dismiss + return), and otherwise started a WM
   drag whose grab swallows the release egui needs — menu items could
   never fire in mini at all.

**The law:** any manual "close on press outside" logic must test the
press position against the layer under it: the menu AND its submenus
live on `egui::Order::Foreground` (`ctx.layer_id_at(pos)`); close only
when the press lands on a lower layer. Never a hovered-flag, never a
rect captured pre-layout, never an unconditional winit-side eat. And
`pointer.press_origin()` is `None` when the release arrived in the
same input batch (fast clicks, xdotool) — always fall back to
`interact_pos()`.

**Receipts (2026-07-07, Xvfb :99, isolated instance):** submenu click
changed mode xy→helix (probe); Grid root-item toggled (screenshot);
fullscreen "Next track" advanced Acid Rain→Artifact (probe); mini
submenu click set xy45 (probe); outside-press dismissed in all three
modes (screenshots).

## #2 — Fullscreen playlist opens as a floating window (fixed v4.0.2)

**Symptom (Ben):** "Playlist in fullscreen view pops out into a
window, not the left pane slide out."

**Root cause:** `ui_playlist_panel` gated the docked `SidePanel` on
`!(is_mini || is_fullscreen)` — fullscreen took mini's floating-window
branch.

**The law:** fullscreen is a CHROME-hiding mode, not a
LAYOUT-shrinking mode — panels the user summons explicitly (L) dock
exactly as they do windowed. Only mini (200–520 px square) is
physically too small to dock and floats.

**Receipt:** fullscreen + L → docked left pane, 53 folder tracks,
current highlighted; row click switched the playing track (probe).

## #3 — External open leaves the playlist empty (fixed v4.0.2)

**Symptom:** `phosphor ctl open x.wav` (also file-manager forwards and
MPRIS OpenUri) played the track but the playlist stayed empty — bare
panel, dead next/previous, no gapless.

**Root cause:** those paths pushed `UiAction::PlayPath` →
`play_file(path, rebuild_playlist: false)`, and the not-in-playlist
case did nothing.

**The law:** in `play_file`, a path NOT found in the current playlist
means it came from outside the list — build the folder-sibling
playlist (file-dialog law). Drag-drop stays single-track by seeding
its list BEFORE calling (its path is always found). Don't "fix" the
`false` at the playlist-row/Tracks-submenu call sites — rebuilding
there would destroy a seeded list.

**Receipt:** ctl open Acid Rain → 53-row playlist, gapless next
queued, fullscreen pane populated.

---

## Standing laws (older repeat families — one line each, don't relearn)

- **Focus trap:** egui 0.33 `wants_keyboard_input()` ==
  `focused().is_some()` and clicked buttons KEEP focus. Keyboard gate
  is OUR `text_focus_ids` registry; every text-capable widget must
  register each frame or typing s/g/f in it fires shortcuts.
- **Scope wheel gate:** `wants_pointer_input()` counts the
  CentralPanel itself — gate wheels on the scope response's
  occlusion-aware `hovered()`, stored per frame.
- **Menu vs mini settle:** never re-square/snap the mini while
  `context_menu_open` — the window sliding under an open menu was
  "right click glitches out a ton".
- **Escape** walks the leave-cascade (compose → fullscreen → mini);
  it is NOT a popup closer. Don't send it casually in receipts.
- **Version law:** egui 0.33 ↔ egui-wgpu 0.33 ↔ wgpu 27 ↔ winit 0.30.
  The glue defines the set; egui-phosphor 0.11 matches egui 0.33.
- **sRGB double-encode:** prefer a non-sRGB surface; else set the
  `hw_encode` uniform so the shader skips its gamma pow. Offline path
  stays 0/false or goldens break.
- **Repaint law:** honor `viewport_output.repaint_delay` via
  `next_frame_due`; `Resized` must set `chrome_dirty` or exposed
  regions band black.
- **Receipt tooling:** winit DROPS xdotool `--window` synthetic keys —
  focus the window (`windowfocus` on WM-less Xvfb, `windowactivate`
  otherwise) and send XTEST. `xdotool click 4` = TWO LineDelta events.
  Keep synthetic-input windows short on Ben's real display.
- **Process hygiene:** `pkill -x phosphor` kills the applet's feed
  (comm is "phosphor"); `pkill -f` self-matches the shell — use
  patterns like `phospho[r]`.
- **Isolated instances:** own `XDG_RUNTIME_DIR` (short path, SUN_LEN
  ~108B) + hand back `PIPEWIRE_RUNTIME_DIR=/run/user/1000` +
  `PULSE_SERVER` or it goes deaf; `PHOSPHOR_NO_SINGLE_INSTANCE=1`;
  fakehome so Ben's settings.json is untouched; `ctl volume 0` before
  loading tracks or the test plays on his speakers.
- **Bench fixtures:** `noise`/`scene` cuts are NOT in the repo and not
  Rust-regenerable; after a /tmp wipe recover
  `tests/bench/signals.py` from git history (`git show
  5a33b59^:tests/bench/signals.py`) and regenerate the 240 s cut into
  /tmp/phosphor-bench (SHA-verified against v3-baseline.json).
  "signal unavailable" is environmental, not a perf regression.
