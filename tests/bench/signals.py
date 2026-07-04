# SPDX-License-Identifier: GPL-3.0-or-later
"""Deterministic bench workloads, shared between the v3 baseline and the
future v4 `phosphor bench` — the comparison is only honest if both eras
trace byte-identical audio.

Three stress signals (Ben: "very basic shapes don't tell us much"):

  sweep — the original parity-test sweep; kept as the light/basic
          reference point.
  chaos — an 8-oscillator FM stack (4 per channel, detuned right),
          phases in closed form (no cumulative drift): a dense, harsh,
          endlessly-evolving figure at ~90 % deflection.
  noise — seeded uniform noise at 0.85 amplitude: every sample jumps
          anywhere on screen, so segment length ≈ screen diagonal.
          This is the fill-rate worst case; it is what "max out the
          GPU" means for a beam renderer.
  scene — scenes/stress-knot.scene.json through the real studio
          compiler (31:29 lissajous, 8192 points, rotating, breathing),
          tiled seamlessly (it is a loop by construction). Complex
          *drawn* geometry, the AFTERGLOW-shaped workload.

All wavs are 48 kHz s16 stereo. Regeneration is bit-identical: chaos is
closed-form math, noise uses numpy's stability-guaranteed PCG64 stream,
the scene renders through the deterministic studio pipeline. SHA-256 of
every generated file is recorded in the bench results.
"""

import math
import os
import struct
import sys
import wave

import numpy

REPO = os.path.dirname(os.path.dirname(os.path.dirname(
    os.path.abspath(__file__))))
sys.path.insert(0, REPO)

RATE = 48000
NOISE_SEED = 0x9805F0

SIGNAL_NAMES = ("sweep", "chaos", "noise", "scene")


def _write_wav(path, frames):
    """frames: float array shape (n, 2) in [-1, 1] -> s16 stereo wav."""
    clipped = numpy.clip(frames, -1.0, 1.0)
    data = (clipped * 32767.0).astype("<i2").tobytes()
    with wave.open(path, "w") as wav_file:
        wav_file.setnchannels(2)
        wav_file.setsampwidth(2)
        wav_file.setframerate(RATE)
        wav_file.writeframes(data)


def sweep(seconds):
    """The parity test's sweep + detuned right, cycling every 8 s."""
    t = numpy.arange(seconds * RATE, dtype=numpy.float64) / RATE
    frequency = 220.0 + 400.0 * ((t % 8.0) / 8.0)
    left = 0.6 * numpy.sin(2 * math.pi * frequency * t)
    right = 0.6 * numpy.sin(2 * math.pi * frequency * 1.5 * t + 0.7)
    return numpy.stack([left, right], axis=1)


def chaos(seconds):
    """8-oscillator FM stack. Phase in closed form:
    integral of (f + d·sin(2π·g·t + φ)) dt = f·t − d/(2π·g)·cos(…)."""
    t = numpy.arange(seconds * RATE, dtype=numpy.float64) / RATE
    channels = []
    for channel in (0, 1):
        total = numpy.zeros_like(t)
        weight_sum = 0.0
        for k in range(4):
            base = 110.0 * (k + 1) * (1.0 + 0.011 * channel * (k + 1))
            lfo = 0.13 * (k + 1)
            depth = 55.0 * (k + 1)
            phase = 2 * math.pi * (
                base * t - depth / (2 * math.pi * lfo)
                * numpy.cos(2 * math.pi * lfo * t + channel))
            weight = 1.0 / (k + 1)
            total += weight * numpy.sin(phase)
            weight_sum += weight
        channels.append(0.9 * total / weight_sum)
    return numpy.stack(channels, axis=1)


def noise(seconds):
    """Seeded uniform noise: maximal beam jumps, the fill-rate killer."""
    generator = numpy.random.default_rng(NOISE_SEED)
    return generator.uniform(-0.85, 0.85, size=(seconds * RATE, 2))


def scene(seconds):
    """scenes/stress-knot.scene.json through the real studio compiler,
    tiled to length (seamless: the scene is a loop by construction)."""
    import json
    import phosphor_studio
    with open(os.path.join(REPO, "scenes",
                           "stress-knot.scene.json")) as handle:
        document = json.load(handle)
    compiled, _rate = phosphor_studio.compile_scene(document,
                                                    sample_rate=RATE)
    frames = numpy.asarray(compiled, dtype=numpy.float64)  # (n, 2)
    repeats = max(1, math.ceil(seconds * RATE / len(frames)))
    return numpy.tile(frames, (repeats, 1))[:seconds * RATE]


GENERATORS = {"sweep": sweep, "chaos": chaos, "noise": noise,
              "scene": scene}


def ensure_wav(name, seconds, directory):
    """Generate (once) and return the path to a bench signal wav."""
    path = os.path.join(directory, f"signal-{name}-{seconds}s.wav")
    if not os.path.exists(path):
        _write_wav(path, GENERATORS[name](seconds))
    return path


if __name__ == "__main__":
    import hashlib
    target = sys.argv[1] if len(sys.argv) > 1 else "/tmp"
    for name in SIGNAL_NAMES:
        path = ensure_wav(name, 240, target)
        digest = hashlib.sha256(open(path, "rb").read()).hexdigest()
        print(f"{name}: {path} sha256={digest[:16]}…")
