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

## #4 — Every dropdown wears a scrollbar despite acres of room (fixed v4.4.0)

**Symptom (Ben):** "Selecting scope art isn't fully expanded when
there's plenty of room… theme selector and scope selector, why are
they squished and needing a scrollbar?? … you need to be testing this
app in a higher resolution. Universal UI principles."

**Root cause:** egui's `Spacing::combo_height` defaults to a fixed
200 px — every ComboBox popup (mode, beam, theme, source, max-fps)
scroll-caged at 200 px regardless of available space. Separately, the
context menu gated content on `is_mini` (a 520 px mini hid what a
280 px mini can't fit) and receipts were taken on a 1600×1000 Xvfb.

**The law:** popups grow until the WINDOW is the limit —
`combo_height` tracks `content_rect().height()` per frame; content
gates key off measured space (`compact = height < threshold`), NEVER
off a window-mode flag. **And receipts run at Ben's resolution:
2560×1440 Xvfb** — a popup that fits a test rig proves nothing about
a squish law.

**Receipt:** mode/beam/theme combos fully expanded at 2560×1440 and
980×640; 520 px mini shows the full menu, 280 px mini still cages.

## #5 — F dead in mini; right-click FPS can't reach the nerd HUD (fixed v4.4.0)

**Symptom (Ben):** "f hotkey doesn't work in Mini player but works in
fullscreen and normal… right click doesn't toggle through nerd mode…
options like FPS are just missing from the right click menu."

**Root cause, three parts:** (1) the mini's left-press handler starts
a WM `drag_window()` on ANY interior press — the click that would
give the undecorated window keyboard focus becomes a move-grab, so
the mini rarely HOLDS focus and plain-character keys land elsewhere;
(2) the menu's FPS entry was a bare `show_fps` checkbox — no path to
`show_fps_detail` (the HUD cycle lived only in the F key);
(3) the entry was `compact`-gated and compact == is_mini (see #4).

**The law:** the mini press handler calls `focus_window()` BEFORE
starting any WM grab; UI affordances that mirror a hotkey share the
hotkey's exact state machine (one `cycle_fps()`, two callers).

**Receipt:** F cycles off→counter→HUD inside a focused mini; the
menu's `FPS: <state> (F)` row walks the same three states.

## #6 — M in fullscreen doesn't reach the mini (fixed v4.4.0)

**Symptom (Ben):** "When clicking M from fullscreen, it doesn't go to
miniplayer, have to exit fullscreen first."

**Root cause:** `MiniToggle` called `set_mini_mode` while the window
was still fullscreen — the WM ignores resize/undecorate requests on a
fullscreen surface, so the mini geometry never landed.

**The law:** mode transitions COMPOSE: entering mini from fullscreen
un-fullscreens first (one gesture, both steps), same as the Escape
cascade walks one layer at a time deliberately.

**Receipt:** F11 → M lands directly in a square mini; M again
restores; fullscreen state fully cleared.

## #7 — Glass scope dead on the CPU renderer (fixed v4.4.0)

**Symptom (Ben):** "Glass scope doesn't work on CPU mode."

**Root cause:** `RasterJob` never carried `scope_alpha`, so the
worker's `CpuRenderer` stayed at its constructor default 1.0 — the
shared composite law (`alpha = scope_alpha + (1−scope_alpha)·b·2`)
always emitted A=255 and the frame painted opaque over the correctly
tinted surface. Two secondary eaters: the frame uploaded as
UNmultiplied (double-premultiply once alpha existed) and the chrome
pass re-cleared with the tinted background under the frame (pane
stacking: `T + T(1−T)` instead of `T`).

**The law:** ONE glass-alpha resolution (`live_glass_alpha()`) feeds
surface clear + GPU compositor + raster jobs; CPU frames premultiply
in gamma space (bit-identical to the GPU glass shader's output form);
offline paths never touch `scope_alpha` — 1.0 is the identity, and
the goldens are the gate that proves it.

**Receipt:** unit tests (bytes identical at 1.0; background alpha
exactly 128 at 0.5 with the beam surviving), goldens 3/3 + snapshots
5/5 byte-held, live smoke on Xvfb. Compositor-visual check on a real
desktop still owed (Xvfb composites nothing).

## #8 — Escape with a dropdown open quit the whole app (fixed v4.4.0)

**Symptom:** pressing Escape to close a combo popup in a plain normal
window fell through to the leave-cascade's last step — Close — and
quit Phosphor. Found when the 4.4.0 receipt rig killed itself.

**The law:** Escape belongs to an open popup first: if
`Memory::any_popup_open()` (combos; ComboBox registers there) or our
`context_menu_open` is true, the window-level handler stands down and
egui closes the popup with that same press. The cascade only ever
sees a CLEAN Escape.

**Receipt:** combo open → Escape → popup closed, `running:true`;
plain window → Escape → quit (cascade intact).

Also fixed in passing (caught BY the hover-card receipt): kits listed
twice when the repo `kits/` and the installed
`/usr/share/phosphor/kits` both resolve — rows now dedupe by file
stem, earliest dir wins.

## #9 — The mini wanders; resize grows past the screen (fixed v4.5.0)

**Symptom (Ben):** "switching in/out of m, the window starts moving
around on its own / not remembering its place… resizing the
miniplayer is a bit glitchy, bottom extends out a bit."

**Root cause, three independent movers:** (1) entering mini FROM
FULLSCREEN banked the fullscreen dims as `normal_geometry`, so
mini-leave "restored" to 2560×1440@0,0; (2) mini-leave applied
`set_outer_position` while the WM was still re-adding decorations —
frame insets shifted the client a few px EVERY round trip;
(3) the re-square used `max(w,h)` (an edge drag could never shrink
the mini) and nothing kept the grown square inside the work area
(bottom extended past the screen near the lower edge).

**The law:** geometry banks at the moment it is TRUE —
FullscreenToggle banks normal geometry on the way in, set_mini_mode
never clobbers an existing bank, plain F11-out drops it (the WM
restores itself); position restores get ONE deferred re-assert
(~160 ms) after the frame is back; the re-square follows the axis
the user dragged and the settle clamps the square inside the work
area. Glass minis wear a dashed hairline so an undecorated
transparent square has visible edges.

**Receipt (1440p):** three M round-trips position-stable to the
pixel (300,200 ↔ 2100,1000); F11→M→M chain restores the banked
normal geometry, not fullscreen dims; dotted outline screenshot.
Frame-inset drift + work-area clamp need a real WM — Ben's final
round covers them.

## #10 — The menu was jailed inside the window (fixed v4.6.0)

**Symptom (Ben):** "the right click should be able to be bigger
than/expand outside of the actual player window" — an egui popup
physically cannot: it draws on the window's own surface.

**The fix:** the context menu is a real OS window now (`MenuPopup`):
X11 `PopupMenu`-typed, override-redirect, always-on-top, transparent
canvas (submenus flare into the spare space), its own egui
context/surface on the shared wgpu device, spawned at the global
cursor after the frame (window creation needs the event loop).
`context_menu_items` is the single menu body, `request_menu_close`
closes both hosts (egui submenu + the popup window).

**Laws learned building it:** (1) winit's per-window RedrawRequested
never arrived for the override-redirect popup — it renders from
about_to_wait on a ~16 ms wake instead; (2) an override-redirect
window NEVER holds focus: winit reports `Focused(false)` at creation
and closing on it killed the menu one frame in — close on main-window
press/focus-loss/Escape/item-click, never on the popup's own focus
events; (3) fullscreen→mini must be STAGED (un-fullscreen, then enter
mini ~140 ms later once the WM lands the restore) — a same-tick
shrink lost the race and cost a second M press.

**Receipt (1440p):** the menu towers over a 280 px mini (screenshot);
submenu click switched mode ring + closed the popup; FPS row clicked
twice walking ■□→■■ with the menu staying open; Escape closed it with
the app alive; F11→one M→mini probe-verified.

## #11 — The mini toggle flaps; the window forgets itself (fixed v4.6.2)

**Symptom (Ben):** "Window behavior is very buggy — not remembering
last location; miniplayer on/off toggle switches location/bugs out."

**Root cause, FOUR independent movers** (each receipted live on a
nested Muffin 6.6.3 rig — see the receipt):

1. **X11 synthetic key events reached the shortcut table.** X11
   synthesizes key events around focus changes; the WM re-decorating
   on a mode switch IS a focus dance, and winit forwards the noise
   flagged `is_synthetic`. A synthetic M-Pressed re-delivered ~7 ms
   after the real press re-toggled the mini in the very next drain —
   leave + instant re-enter, reading as "M does nothing / flashes".
   Non-deterministic (focus-timing dependent), which is why receipts
   kept passing while Ben kept hurting.
2. **Two owners for the double-click-in-mini gesture.** The winit
   press handler detects it (it must — it runs before the WM move
   grab), and the egui scope response ALSO pushed MiniToggle on
   `double_clicked() && is_mini`. One physical double-click = two
   toggles. Latent since #1's fix stopped eating mini presses (the
   egui path could never fire before that).
3. **The settle machinery outlived the mini.** The re-square/snap
   settle had no `is_mini` ownership at fire time: the first click of
   a double-click (or any drag release) armed it, the second click
   left mini, and ~180 ms later the settle SNAPPED THE RESTORED
   NORMAL WINDOW to a work-area edge and stamped its position into
   `mini_x/mini_y`. The next M then opened the mini at the normal
   window's spot — "the toggle switches location".
4. **Persistence recorded half-truths.** `window_width/height` were
   read at launch but never written back (any resize was forgotten on
   quit); `Moved` events during mode-switch convergence recorded the
   WM's transients as "the last location"; the mini's own spot only
   reached settings at snap time, so a quit or a fast M inside the
   180 ms settle window lost the drag.

**The laws:**
- Synthetic key events are state-sync noise, not keystrokes: the
  shortcut table takes only `!is_synthetic` presses.
- ONE owner per gesture — double-click-in-mini belongs to the winit
  press handler alone; egui never mirrors a winit-owned gesture.
- The settle/re-square/snap machinery is mini-ONLY: cleared inside
  `set_mini_mode(false)`, guarded again at fire (`!is_mini` → drop).
- Geometry persistence records only settled user truth: never while
  a GeometryGoal converges or a staged switch is pending; sizes
  persist exactly like positions; the mini's spot follows its drags
  directly. The LAUNCH placement converges through the same
  GeometryGoal as mode switches (a one-shot `with_position` is still
  a timing guess).
- `PHOSPHOR_GEOM_LOG=1` ships in release builds: every geometry
  decision to stderr. When a WM race survives to a real desktop, the
  log IS the receipt — no more guessing.

**Receipt (2026-07-07):** `tests/receipts/w2-wm-geometry.sh` — a
REAL reparenting WM inside the rig (Xvfb 2560×1440 + dbus-run-session
`muffin --x11`, Ben's actual WM), 10/10: relaunch restores client
position AND size to the pixel; 3 mini round-trips under a
SIGSTOP-pulsed (hitching) Muffin, client-stable to the pixel;
double-click restore = exactly one toggle; drag-then-instant-M leaves
the restored window untouched; the next M returns the mini to its
dragged spot; F11 → ONE M → square mini → M → banked normal; quit
from mini relaunches normal with the mini spot surviving. The
synthetic-key delivery was caught live in the geom log
(`state=Released synthetic=true` during a mode switch).

## #12 — Menu-popup clicks act a beat late (fixed v4.6.2)

**Symptom (Ben):** "The ui takes a bit of time to update, slow when
switching fps view."

**Root cause:** the context menu became its own OS window in 4.6.0
(#10) — so a click on the FPS row mutated settings and queued
actions, but NOTHING woke the main window: no input event, no
repaint request. The change sat queued until the main window's next
natural frame — imperceptible with the beam advancing, a visible
beat (or "until I move the mouse") with the scope quiet-asleep. The
F key never lagged because key input repaints by itself; only the
popup path was starved.

**The law:** any action pushed by the menu popup's frame wakes the
main window immediately (`chrome_dirty = true` + `request_redraw()`),
mirroring the key-press path exactly. Every menu row pushes an
action, so the wake needs no per-row wiring — and hover-only popup
frames wake nothing (no idle cost).

**Receipt (2026-07-07, quiet-asleep scope on the Muffin rig):** FPS
row clicked in the popup → the fps plate is on the main window in
the +350 ms screenshot (988 px changed in the corner crop);
two more clicks walked ■■ then □□ with the menu staying open;
Escape closed the popup with the app alive (#8 law re-receipted).
Also in this release: the squares moved to the row's LEFT, sitting
in the same column as the sibling checkbox boxes (Ben: "moved to
the left with the other boxes").

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
- **Escape** walks the leave-cascade (compose → fullscreen → mini →
  **Close**): in a plain normal window Escape QUITS THE APP (v3 law).
  It is not a popup closer — never send it casually in receipts (it
  killed a test instance mid-receipt on 2026-07-07).
- **Version law:** egui 0.33 ↔ egui-wgpu 0.33 ↔ wgpu 27 ↔ winit 0.30.
  The glue defines the set; egui-phosphor 0.11 matches egui 0.33.
- **sRGB double-encode:** prefer a non-sRGB surface; else set the
  `hw_encode` uniform so the shader skips its gamma pow. Offline path
  stays 0/false or goldens break.
- **Repaint law:** honor `viewport_output.repaint_delay` via
  `next_frame_due`; `Resized` must set `chrome_dirty` or exposed
  regions band black.
- **Resolution law:** UI receipts run on a **2560×1440** Xvfb (Ben's
  panel) — space bugs (scroll cages, hidden options, squished popups)
  are invisible on small test screens. Windowed AND the tight cases
  both get receipts; "fits on my rig" is not a receipt.
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
