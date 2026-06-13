# SPDX-License-Identifier: GPL-3.0-or-later
"""Compose mode: draw a shape on the scope, hear it.

This is the inverse of everything else in Phosphor. Instead of audio
becoming a picture, a picture becomes audio: a path drawn with the mouse is
resampled into a closed loop traversed at constant speed — left channel = X,
right channel = Y — and played at a chosen loop frequency. Feed that audio
to any XY oscilloscope (including Phosphor itself, which is exactly what
compose mode does) and it draws the shape. Draw a mushroom, hear the
mushroom.

Constant *speed* matters, not constant parameter: the beam's brightness is
dwell time, so traversing the path at uniform velocity keeps the drawn
shape evenly lit instead of bunching light where the mouse moved slowly.

The loop is written as a seamless WAV (an exact whole number of cycles), so
playback can simply repeat the file forever, and an export of the same
audio works on real scopes and in other software.
"""

import math
import os
import struct
import time
import wave

try:
    import numpy
except ImportError:
    numpy = None

LOOP_WAVE_PATH = os.path.expanduser("~/.cache/phosphor/compose-loop.wav")
EXPORT_DIRECTORY = os.path.expanduser("~/Music/Phosphor")
LOOP_FILE_SECONDS = 1.0      # short file looped forever by the player
EXPORT_SECONDS = 10.0
MINIMUM_FREQUENCY_HZ = 20.0
MAXIMUM_FREQUENCY_HZ = 400.0
SMOOTHING_FRACTION = 1 / 150  # of a cycle; rounds off pen jitter, not corners


def clamp_frequency(frequency_hz):
    return max(MINIMUM_FREQUENCY_HZ, min(MAXIMUM_FREQUENCY_HZ, frequency_hz))


def _close_path(points):
    """The loop must end where it began or every cycle gets a retrace flash."""
    if points[0] != points[-1]:
        return list(points) + [points[0]]
    return list(points)


def resample_path_constant_speed(points, sample_count):
    """Resample a drawn path into `sample_count` (x, y) pairs that traverse
    the closed path at constant speed.

    Walks the cumulative arc length of the polyline and places samples at
    uniform distance steps, interpolating linearly within each edge.
    """
    closed = _close_path(points)
    edge_lengths = [
        math.hypot(closed[i + 1][0] - closed[i][0],
                   closed[i + 1][1] - closed[i][1])
        for i in range(len(closed) - 1)
    ]
    total_length = sum(edge_lengths)
    if total_length <= 0.0:
        raise ValueError("path has no length")

    resampled = []
    edge_index = 0
    distance_into_edge = 0.0
    step = total_length / sample_count
    for _ in range(sample_count):
        while (edge_index < len(edge_lengths) - 1
               and distance_into_edge >= edge_lengths[edge_index]):
            distance_into_edge -= edge_lengths[edge_index]
            edge_index += 1
        edge_length = edge_lengths[edge_index]
        fraction = (distance_into_edge / edge_length) if edge_length > 0 else 0.0
        start, end = closed[edge_index], closed[edge_index + 1]
        resampled.append((start[0] + (end[0] - start[0]) * fraction,
                          start[1] + (end[1] - start[1]) * fraction))
        distance_into_edge += step
    return resampled


def _smooth_closed_loop(samples, window):
    """Circular moving average: takes the buzz out of hand-drawn jitter
    while leaving deliberate corners essentially intact."""
    if window < 3:
        return samples
    count = len(samples)
    half = window // 2
    if numpy is not None:
        loop = numpy.asarray(samples, dtype=numpy.float64)
        padded = numpy.concatenate((loop[-half:], loop, loop[:half]))
        kernel = numpy.ones(2 * half + 1) / (2 * half + 1)
        smoothed_x = numpy.convolve(padded[:, 0], kernel, mode="valid")
        smoothed_y = numpy.convolve(padded[:, 1], kernel, mode="valid")
        return list(zip(smoothed_x.tolist(), smoothed_y.tolist()))
    smoothed = []
    for index in range(count):
        x_total = y_total = 0.0
        for offset in range(-half, half + 1):
            x, y = samples[(index + offset) % count]
            x_total += x
            y_total += y
        smoothed.append((x_total / (2 * half + 1), y_total / (2 * half + 1)))
    return smoothed


def loop_samples(points, frequency_hz, sample_rate):
    """One seamless cycle of the drawn shape as (x, y) pairs in -1..1."""
    samples_per_cycle = max(16, int(round(sample_rate / frequency_hz)))
    cycle = resample_path_constant_speed(points, samples_per_cycle)
    smoothing_window = int(samples_per_cycle * SMOOTHING_FRACTION)
    return _smooth_closed_loop(cycle, smoothing_window)


def _write_wav(path, cycle, cycle_count, sample_rate):
    """Tile `cycle_count` repeats of one cycle into a 16-bit stereo WAV."""
    os.makedirs(os.path.dirname(path), exist_ok=True)
    frames = bytearray()
    for x, y in cycle:
        left = max(-1.0, min(1.0, x))
        right = max(-1.0, min(1.0, y))
        frames += struct.pack("<hh", int(left * 32767), int(right * 32767))
    with wave.open(path, "w") as wav_file:
        wav_file.setnchannels(2)
        wav_file.setsampwidth(2)
        wav_file.setframerate(sample_rate)
        wav_file.writeframes(bytes(frames) * cycle_count)
    return path


def write_loop_wav(points, frequency_hz, sample_rate):
    """The playback loop: ~1 s of whole cycles, seamless when repeated."""
    cycle = loop_samples(points, frequency_hz, sample_rate)
    cycle_count = max(1, int(round(LOOP_FILE_SECONDS * frequency_hz)))
    return _write_wav(LOOP_WAVE_PATH, cycle, cycle_count, sample_rate)


def export_drawing_wav(points, frequency_hz, sample_rate):
    """A shareable take of the drawing: EXPORT_SECONDS of audio that draws
    the shape on any XY oscilloscope. Saved beside the other exports."""
    cycle = loop_samples(points, frequency_hz, sample_rate)
    cycle_count = max(1, int(round(EXPORT_SECONDS * frequency_hz)))
    output_path = os.path.join(
        EXPORT_DIRECTORY, time.strftime("phosphor-drawing-%Y%m%d-%H%M%S.wav"))
    return _write_wav(output_path, cycle, cycle_count, sample_rate)
