phosphor(1)

# NAME

phosphor - CRT-style XY oscilloscope for desktop audio

# SYNOPSIS

*phosphor* [_file_]++
*phosphor* *render* _input_ _output.mp4_ [*--rate* _N_]++
*phosphor* *bench*++
*phosphor* [*--mini*] [*--visitor*]

# DESCRIPTION

Phosphor draws what your PC plays the way an analog scope would: XY
mode for oscilloscope music, a goniometer for stereo width, 3D
delay-embedding views, waveform and spectrum. One Rust engine
simulates P7 phosphor decay on the GPU (wgpu) or CPU (SIMD), captures
whole outputs, single applications, or microphones natively over
PipeWire, plays audio files and signal postcards (_.phos_), wears
transform kits (_.phoskit_), lets you draw shapes the scope plays
back (compose mode), and exports snapshots and mp4 clips with sound.

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

*probe*, *tap*, *ctl*, *feed*, *kit*, *studio*
	Not built yet: these exit 2 with a short message. The agent
	control surface (probe/tap/ctl/feed, kit validate|inspect)
	lands in wave 3; the studio scene compiler returns in wave 4
	(its v3 Python form was retired with v3). Track progress in
	the GitHub issues.

# KEYS

Space: capture on/off (play/pause while a track is loaded) · O: open
file · D: compose (draw on the scope) · M: mini view · S: snapshot ·
C: save the last 10 s as mp4 · P: pin above · G: grid · L: playlist ·
F: fps overlay · F11: fullscreen scope · Escape: leave
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
returns (in Rust) in wave 4. Precompute was retired deliberately: the
v4 engine reconstructs in realtime on the GPU path; the decision and
its revisit criteria are recorded in PARITY.md. The Cinnamon panel
applet still runs its bundled v3-era engine until the wave-3 rewrite.

# LICENSE

GPL-3.0-or-later. Ben <2bmillerb@gmail.com>.
