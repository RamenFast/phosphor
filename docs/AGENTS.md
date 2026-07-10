# Driving Phosphor as an agent

Phosphor is a station-grade instrument: one binary, JSON everywhere,
short directive errors, no pixels needed. This is the worked guide —
the schema itself comes from `phosphor schema`.

## The contract

- Every one-shot emits **one envelope**:
  `{"status":"ok|error","tool":"phosphor","version":…,"ts":…,…}`.
- Errors always carry a `fix` you can act on.
- Exit codes: `0` ok · `2` unavailable (not running / not built yet) ·
  `3` bad arguments · `4` runtime failure.
- Output auto-switches to JSON when stdout is a pipe; `--json` forces
  it on a TTY.
- Streams (`tap`, `feed`) are NDJSON; the first `tap` line is a
  server-emitted `hello` event (protocol version, app version, pid,
  socket path — it names WHICH instance answered), `tick` heartbeats
  prove liveness while the scope is quiet, and the last line is an
  `end` event with a reason (`server-eof` after a clean server exit is
  exit 0; a server that vanished before saying anything is exit 4).

## Read state

```bash
phosphor probe --json
```

`running:false` (exit 0) when no GUI is up. The field to trust is
**`source`** — `{"kind":"capture|mix|player|silent","detail":…}` —
it is *what actually feeds the beam*, reconciled with the engine.
`capture.target_id` is only the remembered preference.

**`beam_cycle`** (4.1+) is `null` unless the Custom-theme color cycle
is animating, else `{"colors":2|3,"seconds":…,"mode":"timer|track",
"current":[r,g,b]}`. `current` is the interpolated beam color this
tick — poll it to watch the color travel without screenshots. `timer`
advances continuously; `track` (4.2+) advances one slot per song
change and rests between songs.

## Drive the scope

```bash
phosphor ctl mode xy45            # modes: see `phosphor schema` enums
phosphor ctl theme "P7 Green"     # beam phosphor color
phosphor ctl ui amber             # chrome look (12 ids)
phosphor ctl target "device:alsa_output.pci-0000_0b_00.4.analog-stereo.monitor"
phosphor ctl target "app:Spotify" # one app, by name key
phosphor ctl target 42            # or a PipeWire node id (integer)
phosphor ctl target "mix:app:one+app:two"   # fold several apps
phosphor ctl capture off
phosphor ctl open /music/song.flac          # load + focus
phosphor ctl play / pause / toggle / next / previous
phosphor ctl seek -- -10          # relative seconds
phosphor ctl volume 0.7
phosphor ctl raise                # focus the window
phosphor ctl snapshot             # BLOCKS until the PNG lands, returns its path
phosphor ctl clip                 # same for the 10 s mp4 with sound
phosphor ctl quit
```

`snapshot`/`clip` use a deferred reply — parse `result.path` from the
envelope; the file exists by the time you see it.

## Watch the beam as numbers

```bash
phosphor tap | jq -c '{mode, segments, bbox, peak}'
```

Per-frame geometry: `bbox` `[minx,miny,maxx,maxy]` in trace px,
`centroid`, `peak`, a ≤64-point `polyline`, `trace_size`. Two laws
worth knowing:

- **Segments arrive in bursts.** At high scope rates the
  reconstruction emits ~8k-segment batches every ~20 ms with zero
  frames between — that's physics, not silence. Judge shape from
  frames where `segments > 0`; judge liveness from `tick`s.
- **A circle is aspect 1.0.** `(bbox[2]-bbox[0])/(bbox[3]-bbox[1])`
  on an L=sin/R=cos tone is the whole rendering-geometry test.

## The beam color cycle (4.1/4.2)

No ctl verb — the cycle is settings-driven (`~/.config/phosphor/
settings.json`, hot-read at launch; the GUI edits it live):
`theme_name:"Custom"` + `beam_cycle_count` (1 = static, 2–3 = cycle) +
`custom_beam_color`/`_2`/`_3` + `beam_cycle_seconds` (leg/fade length,
0.1–60) + `beam_cycle_mode` (`"timer"` continuous · `"track"` one step
per song). Exports follow: snapshots/clips carry the on-screen color,
`phosphor render` animates timer mode on media time (track mode holds
still — one input file is one song). Sub-1 s timer legs prompt the
HUMAN with a photosensitivity confirmation — don't script around it;
respect the pin at 1.0 s.

## Kits (the 7B-model art form)

```bash
phosphor kit validate my.phoskit     # ok | error with the exact key named
phosphor kit inspect my.phoskit      # stages, params, what each op does
```

Errors are one line and directive ("stages[0]: unknown op 'sparkle'
(known: …)") — designed so a small model repairs its kit in one
round-trip. Schema: `docs/phoskit.schema.json`. Multiple files: exit
reflects the worst.

## Headless / background

```bash
phosphor --background &   # full GUI on a private Xvfb display:
                          # renders, serves the socket, steals nothing
phosphor render song.flac out.mp4   # no GUI at all
phosphor bench                      # perf gates, JSON
```

## Gotchas (paid for, so you don't have to)

- **The GUI is the default command.** `phosphor` with no args opens a
  window (or focuses the running one and exits 0 — single instance).
  Scripted runs that *need* a second instance set
  `PHOSPHOR_NO_SINGLE_INSTANCE=1` or use `--background`.
- The control socket lives at `$XDG_RUNTIME_DIR/phosphor/ctl.sock`;
  clients fall back to `/tmp/phosphor-$UID`. With several instances
  up, pin the one you mean with `--socket <path>` (probe/ctl/tap) or
  `PHOSPHOR_CTL_SOCKET` — the tap `hello` echoes the pid and socket so
  you can verify who answered. Isolating a test
  instance = giving it its own `XDG_RUNTIME_DIR` — but then hand the
  process `PIPEWIRE_RUNTIME_DIR=/run/user/$UID` (and `PULSE_SERVER=`
  `unix:/run/user/$UID/pulse/native`) or it goes deaf: PipeWire finds
  its socket through the same variable.
- Socket paths must fit `SUN_LEN` (~108 bytes) — deep temp dirs fail
  to bind, silently costing you the whole surface.
- `ctl` needs a *running* GUI (exit 2 otherwise). `probe` never
  fails on a dead GUI — `running:false` is an answer.
- `feed` speaks the frozen v3 applet protocol (`{"s":[…]}` lines, no
  envelope) — it is the ONE deliberate exception to the contract.
- Sending `Escape` to the window walks the leave-cascade
  (compose → fullscreen → mini → **close**) — in a plain normal
  window Escape QUITS the app. It is not a popup-closer; never send
  it casually.
- `--visitor`, `--exit-after`, `--fps-log` are receipt/dev flags and
  bypass single-instance on purpose.

## Changing the code (not just driving it)

Read **`docs/dev/BUGLOG.md`** FIRST — the regression ledger; every
entry is a shipped bug with the law that prevents its return, and
code comments cite entries as `BUGLOG #N`. Fix a root-caused bug →
append the entry (symptom · root cause · law · receipt). The receipt
must exercise the user's actual gesture (a real click on a real menu
item — v4.0.1 receipted menu *geometry*, never clicked an item, and
shipped the menu broken). The living project map is `HANDOFF.md` at
the repo root; the receipts ledger of the v4 rewrite is
`docs/dev/PARITY.md`; perf gates are `phosphor bench` (BENCH.md laws:
compare under the same machine load — a busy Xvfb test rig once
read as a perf regression).
