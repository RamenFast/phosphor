phosphor(1)

# NAME

phosphor - CRT-style XY oscilloscope for desktop audio

# SYNOPSIS

*phosphor* [_file_]++
*phosphor* *render* _input_ _output.mp4_ [*--rate* _N_]++
*phosphor* *bench*++
*phosphor* *probe* [*--json*] [*--at* _ts_]++
*phosphor* *ctl* _verb_ [_args_]++
*phosphor* *tap*++
*phosphor* *feed*++
*phosphor* *kit* *validate*|*inspect* _file_++
*phosphor* *schema*++
*phosphor* [*--mini*] [*--visitor*] [*--background*]

# DESCRIPTION

Phosphor draws what your PC plays the way an analog scope would: XY
mode for oscilloscope music, a goniometer for stereo width, 3D
delay-embedding views, waveform and spectrum. One Rust engine
simulates P7 phosphor decay on the GPU (wgpu) or CPU (SIMD, on its
own worker thread), captures whole outputs, single applications,
microphones, or several apps folded into one beam, natively over
PipeWire, plays audio files and signal postcards (_.phos_), wears
transform kits (_.phoskit_), lets you draw shapes the scope plays
back (compose mode), posts systemwide track notifications with album
art, and exports snapshots and mp4 clips with sound. When the beam
scopes another MPRIS player (Spotify, a browser), Phosphor's
transport drives that player.

Phosphor is single-instance: a plain launch while one runs focuses
the existing window (a _file_ argument is forwarded to it). *--help*
prints usage; unknown options exit 3 rather than opening a window.

Launching with a _file_ argument opens it like a file manager would.
Dropping audio files on the window queues them; dropping a _.phoskit_
installs and activates it.

# SUBCOMMANDS

*render* _input_ _output.mp4_ [*--rate* _N_]
	Headless full-track render: decode _input_ (audio or _.phos_),
	trace it offline, mux the source audio. _--render_ is accepted
	as the v3 spelling.

*bench*
	The performance regression gate: uncapped fixture renders,
	JSON results, exit 1 on a miss.

*probe* [*--json*] [*--at* _ts_]
	One-shot live status of a running instrument: theme, mode,
	capture state, beam. Prints *running: false* (exit 0) when no
	GUI is up. Output is JSON on a pipe or under *--json*, human
	text on a terminal. *--at* a past timestamp is a stub (exit 2)
	until the studio wave.

*ctl* _verb_ [_args_]
	Drive a running instrument over its control socket
	(_$XDG_RUNTIME_DIR/phosphor/ctl.sock_, NDJSON). Verbs:
	*play*, *pause*, *toggle*, *stop*, *next*, *previous*, *seek*,
	*volume*, *mode*, *theme*, *ui*, *capture* *on*|*off*,
	*target*, *open* _path_, *raise*, *snapshot*, *clip*, *quit*.
	*target mix:app:a+app:b* folds several app streams into one
	beam. *open* loads a file into the player and focuses the
	window; *raise* just focuses. *snapshot*/*clip* return the
	written file path.

*tap*
	Stream the live beam as NDJSON: a *hello* greeting, *frame*
	objects (ts, mode, segments, bbox, centroid, peak, polyline,
	trace size), and a *tick* heartbeat. Pipe it to _jq_.

*feed*
	The applet feed: stdio NDJSON, protocol verbatim from v3 (the
	one documented exception to the envelope — no *event* field).
	The engine-free Cinnamon applet draws from it.

*kit* *validate*|*inspect* _file_
	Check or introspect a _.phoskit_ without launching the GUI.
	Errors are short and name the fix so a small model can repair
	its kit in one round-trip.

*schema*
	Print the strict JSON schemas and enums for the agent surface
	(also shipped as _docs/phoskit.schema.json_).

*studio*
	Not built yet: exits 2 with a short message. The studio scene
	compiler returns in wave 4 (its v3 Python form was retired
	with v3). Track progress in the GitHub issues.

All agent verbs share one envelope (*status*, *tool*, *version*,
*ts*), errors that carry a *fix*, and exit codes 0/2/3/4.

# BACKGROUND MODE

*phosphor --background* runs the full instrument on a private Xvfb
display: it renders, captures, and serves the control socket, tap,
and feed, but never maps a window on your screen and never steals
focus (game-safe). Drive it with *ctl*/*probe*/*tap* and stop it with
*phosphor ctl quit*. Needs _xvfb-run_ (package *xvfb*); exits 2 with
that hint when missing. Works the same launched by hand or by an
agent.

# KEYS

Space: capture on/off (play/pause while a track is loaded) · O: open
file · D: compose (draw on the scope) · M: mini view · S: snapshot ·
C: save the last 10 s as mp4 · P: pin above · G: grid · L: playlist ·
F: fps overlay, again for the nerd HUD (frame ms, gpu ms, p99,
drops, segments/s) · F11: fullscreen scope · Escape: leave
compose/fullscreen/mini, else quit · Q: quit. The mouse wheel over
the scope is gain (pitch while composing; dolly in 3D modes).

# FILES

_~/.config/phosphor/settings.json_
	Settings (v3 files are read and migrated; unknown keys are
	preserved on write-back).

_~/.local/share/phosphor/kits_, _/usr/share/phosphor/kits_
	Signal kits; user kits shadow the shipped starters.

_~/Pictures/Phosphor_, _~/Music/Phosphor_
	Snapshot/clip exports and compose drawings.

# NOT YET IN V4 (VS V3.5)

The v3 Python app shipped a *phosphor-studio* scene compiler and a
render-ahead precompute cache; neither is in this build. The studio
is the one remaining stub — it returns (in Rust) in wave 4.
Precompute was retired deliberately: the v4 engine reconstructs in
realtime on the GPU path; the decision and its revisit criteria are
recorded in PARITY.md. The Cinnamon panel applet is now engine-free
(2.0.0): it draws from *phosphor feed* and bundles no engine code.

# LICENSE

GPL-3.0-or-later. Ben <2bmillerb@gmail.com>.
