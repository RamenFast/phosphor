# SPDX-License-Identifier: GPL-3.0-or-later
"""phosphor-studio: compile scene documents into oscilloscope audio.

The AFTERGLOW principle: **the file format is the API.** A scene is a
plain JSON document describing a shape and how it moves; compiling it
yields stereo audio whose waveform IS the picture — play it on any XY
scope on earth and the scene appears. Agents drive this through
`--output json` and documented exit codes; humans get the same commands
with friendly text. There is no other interface, so the two can never
drift apart.

    {"scene": 1, "name": "breathing dot", "seconds": 4, "loop_hz": 100,
     "shape": {"kind": "polygon", "sides": 24},
     "animate": {"scale": {"from": 0.03, "to": 0.09, "lfo_hz": 0.4}}}

Everything compiles through phosphor_compose's constant-speed resampler —
the same engine as compose mode and the offline renderer. One engine,
many costumes; never a third path.

Exit codes: 0 success · 2 usage (argparse) · 3 invalid scene (the JSON
error names the offending key) · 4 I/O or encoding failure.
"""

import argparse
import hashlib
import json
import math
import os
import struct
import subprocess
import sys
import tempfile
import wave

from phosphor_compose import resample_path_constant_speed

FORMAT_VERSION = 1
DEFAULT_RATE = 48000
DEFAULT_LOOP_HZ = 100.0
DEFAULT_SECONDS = 4.0
EXIT_INVALID_SCENE = 3
EXIT_IO_FAILURE = 4

# The turtle is the tutorial scene: everyone who makes a postcard
# afterwards learns from the turtle. One closed stroke, drawn head to
# tail — shell scutes, head, four feet, and the little tail. 🐢
TURTLE_OUTLINE = [
    (0.62, 0.02), (0.60, 0.14), (0.50, 0.26), (0.36, 0.36),
    (0.18, 0.42), (0.00, 0.44), (-0.18, 0.42), (-0.36, 0.36),
    (-0.50, 0.26), (-0.60, 0.14), (-0.62, 0.02),                # shell rim
    (-0.52, 0.00), (-0.58, -0.10), (-0.66, -0.24),              # rear foot
    (-0.52, -0.26), (-0.44, -0.16),
    (-0.30, -0.20), (-0.24, -0.30),                             # belly line
    (-0.10, -0.34), (0.02, -0.30), (0.10, -0.20),
    (0.24, -0.18), (0.32, -0.28), (0.46, -0.30),                # front foot
    (0.44, -0.16), (0.54, -0.06),
    (0.70, -0.02), (0.82, 0.06), (0.92, 0.06),                  # neck up
    (1.00, 0.12), (1.02, 0.22), (0.96, 0.30), (0.86, 0.30),
    (0.78, 0.24), (0.74, 0.14),                                 # the head
    (0.68, 0.10),
]


class SceneError(ValueError):
    """Invalid scene document; `json_path` names the offending key."""

    def __init__(self, message, json_path="$"):
        super().__init__(message)
        self.json_path = json_path


def _number(document, key, default, low, high, json_path):
    value = document.get(key, default)
    if not isinstance(value, (int, float)) or isinstance(value, bool):
        raise SceneError(f"{key}: expected a number", f"{json_path}.{key}")
    if not low <= value <= high:
        raise SceneError(f"{key}: {value} outside [{low}, {high}]",
                         f"{json_path}.{key}")
    return float(value)


def shape_points(shape):
    """The base cycle of (x, y) points for a shape block, unit space."""
    if not isinstance(shape, dict) or "kind" not in shape:
        raise SceneError("shape: expected {'kind': …}", "$.shape")
    kind = shape["kind"]
    if kind == "polygon":
        sides = int(_number(shape, "sides", 3, 3, 720, "$.shape"))
        return [(math.cos(2 * math.pi * i / sides),
                 math.sin(2 * math.pi * i / sides)) for i in range(sides)]
    if kind == "lissajous":
        a = _number(shape, "a", 1.0, 1.0, 32.0, "$.shape")
        b = _number(shape, "b", 2.0, 1.0, 32.0, "$.shape")
        phase = _number(shape, "phase", 0.0, -math.pi, math.pi, "$.shape")
        points = int(_number(shape, "points", 512, 16, 8192, "$.shape"))
        return [(math.sin(2 * math.pi * a * i / points + phase),
                 math.sin(2 * math.pi * b * i / points))
                for i in range(points)]
    if kind == "path":
        raw = shape.get("points")
        if (not isinstance(raw, list) or len(raw) < 3
                or not all(isinstance(p, list) and len(p) == 2 for p in raw)):
            raise SceneError("path: points must be [[x, y], …] (≥ 3)",
                             "$.shape.points")
        return [(float(x), float(y)) for x, y in raw]
    if kind == "turtle":
        return list(TURTLE_OUTLINE)
    raise SceneError(f"unknown shape kind '{kind}' "
                     "(polygon, lissajous, path, turtle)", "$.shape.kind")


def load_scene(path):
    """Parsed and validated scene document."""
    try:
        with open(path) as scene_file:
            document = json.load(scene_file)
    except OSError as error:
        raise SceneError(f"cannot read {path}: {error}") from None
    except ValueError as error:
        raise SceneError(f"not JSON: {error}") from None
    if not isinstance(document, dict) or "shape" not in document:
        raise SceneError("expected a scene object with a 'shape'")
    version = document.get("scene", 1)
    if int(version) > FORMAT_VERSION:
        raise SceneError(f"scene version {version} is newer than this "
                         "phosphor-studio understands", "$.scene")
    return document


def compile_scene(document, sample_rate=None):
    """Scene → interleaved stereo float frames (list of (x, y)).

    Deterministic by construction: no randomness, so identical documents
    compile to identical samples — the golden tests hash them.
    """
    seconds = _number(document, "seconds", DEFAULT_SECONDS, 0.05, 600.0, "$")
    loop_hz = _number(document, "loop_hz", DEFAULT_LOOP_HZ, 5.0, 2000.0, "$")
    rate = int(sample_rate or _number(document, "rate", DEFAULT_RATE,
                                      8000, 384000, "$"))
    base = shape_points(document["shape"])

    animate = document.get("animate") or {}
    if not isinstance(animate, dict):
        raise SceneError("animate: expected an object", "$.animate")
    scale_block = animate.get("scale") or {}
    if not isinstance(scale_block, dict):
        raise SceneError("scale: expected an object", "$.animate.scale")
    scale_from = _number(scale_block, "from", 0.9, 0.001, 1.2,
                         "$.animate.scale")
    scale_to = _number(scale_block, "to", scale_from, 0.001, 1.2,
                       "$.animate.scale")
    scale_lfo_hz = _number(scale_block, "lfo_hz", 0.0, 0.0, 30.0,
                           "$.animate.scale")
    rotate_hz = _number(animate, "rotate_hz", 0.0, -10.0, 10.0, "$.animate")

    samples_per_cycle = max(16, int(round(rate / loop_hz)))
    cycle_count = max(1, int(round(seconds * loop_hz)))
    frames = []
    for cycle_index in range(cycle_count):
        moment = cycle_index / loop_hz
        if scale_lfo_hz > 0.0:
            breath = 0.5 - 0.5 * math.cos(2 * math.pi * scale_lfo_hz * moment)
            scale = scale_from + (scale_to - scale_from) * breath
        else:
            scale = scale_to
        angle = 2 * math.pi * rotate_hz * moment
        cosine, sine = math.cos(angle), math.sin(angle)
        placed = [(scale * (x * cosine - y * sine),
                   scale * (x * sine + y * cosine)) for x, y in base]
        frames.extend(resample_path_constant_speed(placed, samples_per_cycle))
    return frames, rate


def frames_to_s16le(frames):
    packed = bytearray()
    for x, y in frames:
        packed += struct.pack("<hh",
                              int(max(-1.0, min(1.0, x)) * 32767),
                              int(max(-1.0, min(1.0, y)) * 32767))
    return bytes(packed)


def write_output(frames, rate, output_path):
    """Write frames as WAV directly, or through ffmpeg for anything else
    (flac, ogg…). '-' writes raw s16le to stdout for piping."""
    data = frames_to_s16le(frames)
    if output_path == "-":
        sys.stdout.buffer.write(data)
        return
    if output_path.lower().endswith(".wav"):
        with wave.open(output_path, "w") as wav_file:
            wav_file.setnchannels(2)
            wav_file.setsampwidth(2)
            wav_file.setframerate(rate)
            wav_file.writeframes(data)
        return
    encoder = subprocess.run(
        ["ffmpeg", "-y", "-v", "error", "-f", "s16le", "-ar", str(rate),
         "-ac", "2", "-i", "-", output_path],
        input=data, capture_output=True)
    if encoder.returncode != 0:
        raise OSError(encoder.stderr.decode(errors="replace")[:300]
                      or "ffmpeg failed")


def scene_summary(document, frames, rate):
    return {
        "name": document.get("name")
        or document.get("shape", {}).get("kind", "scene"),
        "seconds": round(len(frames) / rate, 6),
        "frames": len(frames),
        "rate": rate,
        "sha256": hashlib.sha256(frames_to_s16le(frames)).hexdigest(),
    }


def _emit(as_json, payload, human_text):
    print(json.dumps(payload) if as_json else human_text)


def _fail(as_json, error, code):
    payload = {"ok": False,
               "error": {"message": str(error),
                         "path": getattr(error, "json_path", "$")}}
    if as_json:
        print(json.dumps(payload))
    else:
        print(f"error: {error} (at {payload['error']['path']})",
              file=sys.stderr)
    return code


def main(argv=None):
    parser = argparse.ArgumentParser(
        prog="phosphor-studio",
        description="Compile scene JSON into oscilloscope audio — stereo "
                    "sound whose waveform is the picture.",
        epilog="exit codes: 0 success · 2 usage · 3 invalid scene · "
               "4 I/O failure")
    parser.add_argument("--output", choices=("text", "json"), default="text",
                        help="result format (json is machine-readable, "
                             "for agents)")
    commands = parser.add_subparsers(dest="command", required=True)

    render = commands.add_parser(
        "render", help="compile a scene to an audio file")
    render.add_argument("scene", help="scene JSON document")
    render.add_argument("-o", "--out", required=True,
                        help="output audio path (.wav writes directly, "
                             "anything else goes through ffmpeg; '-' pipes "
                             "raw s16le)")
    render.add_argument("--rate", type=int, default=None,
                        help="sample rate override")

    validate = commands.add_parser(
        "validate", help="check a scene document without rendering")
    validate.add_argument("scene")

    inspect = commands.add_parser(
        "inspect", help="compile in memory and report frames/duration/hash")
    inspect.add_argument("scene")
    inspect.add_argument("--rate", type=int, default=None)

    preview = commands.add_parser(
        "preview", help="loop the scene out loud (any watching scope "
                        "draws it)")
    preview.add_argument("scene")
    preview.add_argument("--rate", type=int, default=None)

    options = parser.parse_args(argv)
    as_json = options.output == "json"

    try:
        document = load_scene(options.scene)
    except SceneError as error:
        return _fail(as_json, error, EXIT_INVALID_SCENE)

    if options.command == "validate":
        try:
            compile_scene(document, sample_rate=8000)  # cheap full check
        except SceneError as error:
            return _fail(as_json, error, EXIT_INVALID_SCENE)
        _emit(as_json, {"ok": True, "scene": options.scene},
              f"{options.scene}: valid")
        return 0

    try:
        frames, rate = compile_scene(document, sample_rate=options.rate)
    except SceneError as error:
        return _fail(as_json, error, EXIT_INVALID_SCENE)

    summary = scene_summary(document, frames, rate)
    if options.command == "inspect":
        _emit(as_json, {"ok": True, **summary},
              "\n".join(f"{key}: {value}" for key, value in summary.items()))
        return 0

    if options.command == "render":
        try:
            write_output(frames, rate, options.out)
        except OSError as error:
            return _fail(as_json, error, EXIT_IO_FAILURE)
        _emit(as_json, {"ok": True, "output": options.out, **summary},
              f"rendered {summary['name']}: {summary['seconds']}s "
              f"@ {rate} Hz → {options.out}")
        return 0

    if options.command == "preview":
        # render one file, loop it aloud forever; a scope watching the
        # output monitor draws the scene. Ctrl-C stops.
        with tempfile.NamedTemporaryFile(suffix=".wav",
                                         delete=False) as handle:
            loop_path = handle.name
        try:
            write_output(frames, rate, loop_path)
            _emit(as_json, {"ok": True, "looping": options.scene},
                  f"looping {summary['name']} — Ctrl-C to stop")
            decoder = subprocess.Popen(
                ["ffmpeg", "-v", "quiet", "-stream_loop", "-1",
                 "-i", loop_path, "-f", "s16le", "-ac", "2",
                 "-ar", str(rate), "-"], stdout=subprocess.PIPE)
            player = subprocess.Popen(
                ["pacat", "--format=s16le", f"--rate={rate}",
                 "--channels=2", "--client-name=phosphor-studio", "--raw"],
                stdin=decoder.stdout)
            player.wait()
        except KeyboardInterrupt:
            pass
        except OSError as error:
            return _fail(as_json, error, EXIT_IO_FAILURE)
        finally:
            try:
                os.unlink(loop_path)
            except OSError:
                pass
        return 0
    return 2


if __name__ == "__main__":
    raise SystemExit(main())
