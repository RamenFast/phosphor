phosphor-studio(1)

# NAME

phosphor-studio - compile scene documents into oscilloscope audio

# SYNOPSIS

*phosphor-studio* [--output text|json] <command> [args...]

# DESCRIPTION

phosphor-studio compiles plain-JSON scene documents into stereo audio
whose waveform IS the picture: play the result on any XY oscilloscope
(hardware or Phosphor) and the scene appears. The file format is the
API — agents, humans, and GUIs all read and write the same documents.

A minimal scene:

```
{"scene": 1, "name": "breathing dot", "seconds": 4, "loop_hz": 100,
 "shape": {"kind": "polygon", "sides": 24},
 "animate": {"scale": {"from": 0.03, "to": 0.09, "lfo_hz": 0.4}}}
```

Shape kinds: *polygon* (sides), *lissajous* (a, b, phase, points),
*path* (points as [[x, y], ...]), *turtle* (the tutorial shape).
Animation: *scale* {from, to, lfo_hz} and *rotate_hz*. Paths are
traversed at constant speed so the drawn shape is evenly lit.

# COMMANDS

*render* <scene.json> *-o* <out> [--rate N]
	Compile to an audio file. .wav is written directly; other
	extensions encode through ffmpeg; '-' pipes raw s16le to stdout.

*validate* <scene.json>
	Check a document without rendering. Errors name the offending
	JSON key.

*inspect* <scene.json> [--rate N]
	Compile in memory; report name, duration, frames, and the
	sha256 of the samples (scenes compile deterministically).

*preview* <scene.json> [--rate N]
	Loop the scene out loud until interrupted; any scope watching
	the output draws it.

# OPTIONS

*--output* text|json
	json emits machine-readable one-line results and errors
	(objects with a message and a JSON path), for agents.

# EXIT CODES

0 success · 2 usage error · 3 invalid scene · 4 I/O or encoding failure

# SEE ALSO

Starter scenes live in /usr/share/phosphor/scenes. The turtle is the
tutorial.
