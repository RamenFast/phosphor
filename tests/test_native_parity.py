# SPDX-License-Identifier: GPL-3.0-or-later
"""Parity: the Rust core must trace exactly what the Python path traces.

Runs every display mode over the same streamed audio through both engines
(oversample=1, where their outputs are defined to be identical) and compares
segment-by-segment. Run directly:  python3 tests/test_native_parity.py
"""

import math
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

import phosphor_signal
from phosphor_signal import SegmentComputer

COORDINATE_TOLERANCE = 0.05    # px, f32 vs f64 rounding across the paths
INTENSITY_TOLERANCE = 5e-3


def make_audio(frame_count, sample_rate=48000):
    """A deliberately awkward stereo signal: sweep + detuned right channel."""
    samples = []
    for i in range(frame_count):
        t = i / sample_rate
        frequency = 220.0 + 400.0 * t
        samples.append(0.6 * math.sin(2 * math.pi * frequency * t))
        samples.append(0.6 * math.sin(2 * math.pi * (frequency * 1.5) * t + 0.7))
    return samples


def build_pair(mode, sample_rate):
    native = SegmentComputer()
    assert native._native is not None, "native core not loaded"
    reference = SegmentComputer()
    reference._native = None            # force the Python path
    for computer in (native, reference):
        computer.mode = mode
        computer.gain = 1.3
        computer.beam_energy = 8.0
        computer.frame_glow_keep = 0.82
        computer.set_sample_rate(sample_rate)
    return native, reference


def as_rows(segments):
    return [tuple(float(value) for value in row) for row in segments]


def compare_mode(mode, sample_rate=48000, width=800, height=600, kit=None):
    native, reference = build_pair(mode, sample_rate)
    if kit is not None:
        native.set_kit(kit)
        reference.set_kit(kit)
    audio = make_audio(6000, sample_rate)
    chunk = 1600                        # stream in uneven pieces
    worst = 0.0
    for call_index, start in enumerate(range(0, len(audio) - chunk, chunk)):
        piece = audio[start:start + chunk]
        native_rows = as_rows(native.compute(piece, width, height))
        reference_rows = as_rows(reference.compute(piece, width, height))
        assert len(native_rows) == len(reference_rows), (
            f"{mode} call {call_index}: {len(native_rows)} native segments "
            f"vs {len(reference_rows)} python")
        for row_index, (native_row, reference_row) in enumerate(
                zip(native_rows, reference_rows)):
            for column in range(4):
                delta = abs(native_row[column] - reference_row[column])
                worst = max(worst, delta)
                assert delta <= COORDINATE_TOLERANCE, (
                    f"{mode} call {call_index} segment {row_index} "
                    f"column {column}: {native_row} vs {reference_row}")
            delta = abs(native_row[4] - reference_row[4])
            assert delta <= INTENSITY_TOLERANCE, (
                f"{mode} call {call_index} segment {row_index} intensity: "
                f"{native_row[4]} vs {reference_row[4]}")
    return worst


def test_all_modes_match_python():
    assert phosphor_signal.native_available(), "native core not built?"
    for mode in ("xy", "xy45", "xy_dots", "waveform", "spectrum",
                 "spectrum_radial"):
        for sample_rate in (48000, 96000):
            worst = compare_mode(mode, sample_rate)
            print(f"  {mode:16s} @ {sample_rate}: worst coordinate delta "
                  f"{worst:.5f}px")


KIT_CASES = {
    "rotate": [("rotate", [0.8, 0.4, 0.0, 0.0])],
    "midside": [("midside", [1.7, 0.0, 0.0, 0.0])],
    "ringmod": [("ringmod", [3.0, 0.6, 0.0, 0.0])],
    "wobble": [("wobble", [1.3, 0.9, 0.0, 0.0])],
    "matrix": [("matrix", [0.9, 0.3, -0.2, 1.1])],
    "chandelay": [("chandelay", [7.0, 1.0, 0.0, 0.0])],
    "chain": [("rotate", [0.05, 0.0, 0.0, 0.0]),
              ("midside", [1.4, 0.0, 0.0, 0.0]),
              ("ringmod", [3.0, 0.2, 0.0, 0.0])],
}


def test_kit_chains_match_python():
    """Stateful kit ops must stay in step across engines over many calls
    (phase accumulation and delay lines are where parity usually dies)."""
    for kit_name, stages in KIT_CASES.items():
        for mode in ("xy", "spectrum"):
            for sample_rate in (48000, 96000):
                worst = compare_mode(mode, sample_rate, kit=stages)
                print(f"  kit {kit_name:10s} over {mode:8s} @ {sample_rate}:"
                      f" worst delta {worst:.5f}px")


def test_plan_feed_mapping():
    assert phosphor_signal.plan_feed(48000) == (48000, 1)
    assert phosphor_signal.plan_feed(96000) == (48000, 2)
    assert phosphor_signal.plan_feed(192000) == (96000, 2)
    assert phosphor_signal.plan_feed(384000) == (96000, 4)


def test_oversample_output_density():
    computer = SegmentComputer()
    assert computer._native is not None
    computer.mode = "xy"
    computer.set_sample_rate(48000, oversample=4)
    audio = make_audio(4000)
    base_frames = 0
    total = 0
    for start in range(0, len(audio) - 1000, 1000):
        piece = audio[start:start + 1000]
        base_frames += len(piece) // 2
        total += len(computer.compute(piece, 800, 600))
    # ~4x the segments of the plain feed (minus sinc latency at the head)
    assert 3 * base_frames < total <= 4 * base_frames + 8, total


if __name__ == "__main__":
    test_plan_feed_mapping()
    print("plan_feed mapping ok")
    test_all_modes_match_python()
    test_kit_chains_match_python()
    test_oversample_output_density()
    print("oversample density ok")
    print("PARITY OK")
