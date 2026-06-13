#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Headless scope feed for the Phosphor Cinnamon applet.

The applet can't run a GTK window inside the Cinnamon process, so this
lightweight helper does the audio work outside it: it captures the default
output monitor with the very same `parec` path the app uses, runs the same
`SegmentComputer`, and writes one compact line of beam segments per frame to
stdout. The applet reads those lines and draws them with Cairo — so the panel
scope and the full app trace the signal identically.

It auto-gains the trace (the app's "autosize to screen" idea: instant attack,
slow release, with a noise floor) so a quiet stream still fills the panel
without amplifying silence into jitter.

Protocol
--------
stdout: one JSON object per line, ``{"s": [x0, y0, x1, y1, i, x0, y0, ...]}``
        — a flat run of 5-int segments (coordinates in a 0..1000 box, the
        beam intensity 0..255). ``{"error": "..."}`` if capture can't start.
stdin : optional one-per-line commands the applet sends —
        ``mode <xy|xy45|xy_dots|waveform|spectrum|spectrum_radial>`` or
        ``quit``.

Run standalone to watch it work:  (sleep 3) | python3 phosphor_applet_feed.py
"""

import json
import os
import select
import sys
import time

# Find the Phosphor modules whether we're bundled in the applet directory,
# installed by the .deb, or sitting in a source checkout.
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
if "/usr/lib/phosphor" not in sys.path:
    sys.path.append("/usr/lib/phosphor")

import phosphor_audio
import phosphor_signal


def set_process_name(name):
    """Label this process so it's identifiable in task managers (Linux)."""
    try:
        import ctypes
        libc = ctypes.CDLL("libc.so.6", use_errno=True)
        buffer = ctypes.create_string_buffer(name.encode()[:15])
        libc.prctl(15, ctypes.byref(buffer), 0, 0, 0)  # 15 = PR_SET_NAME
    except Exception:
        pass


COORDINATE_BOX = 1000.0      # segments are emitted in a 0..COORDINATE_BOX square
FRAMES_PER_SECOND = 30
# A panel scope is tiny, so a modest feed rate is plenty and keeps each line
# small (fewer samples per frame -> fewer segments to serialize and parse).
CAPTURE_SAMPLE_RATE = 16000
MAX_SEGMENTS_PER_FRAME = 500  # bound the line length if audio ever backs up

# Auto-gain: scale the trace so its loudest recent peak fills most of the box.
AGC_TARGET_FILL = 0.9        # peak deflection should reach 90% of full scale
AGC_NOISE_FLOOR = 0.005      # below this, treat as silence and ease back to 1x
AGC_MAX_GAIN = 40.0          # generous, so a low system volume still fills the panel
AGC_RELEASE = 0.92           # how slowly the tracked peak falls back each frame

VALID_MODES = {"xy", "xy45", "xy_dots", "waveform", "spectrum", "spectrum_radial"}


def find_default_monitor():
    """A CaptureTarget for the default output's monitor, or None."""
    target_id = phosphor_audio.default_monitor_target_id()
    if target_id:
        monitor_device = target_id.split(":", 1)[1]
        return phosphor_audio.CaptureTarget("device", monitor_device, "OUT")
    for target in phosphor_audio.list_capture_targets():
        if target.kind == "device" and target.identifier.endswith(".monitor"):
            return target
    return None


def apply_command(line, computer):
    """Update the computer from one stdin command line. Returns False on quit."""
    parts = line.split()
    if not parts:
        return True
    if parts[0] == "quit":
        return False
    if parts[0] == "mode" and len(parts) > 1 and parts[1] in VALID_MODES:
        if parts[1] != computer.mode:
            computer.mode = parts[1]
            computer.reset()
    return True


def encode_segments(segments):
    """Flatten beam segments to the compact int run the applet expects."""
    if segments is None or len(segments) == 0:
        return []
    if hasattr(segments, "tolist"):       # numpy array from the fast path
        segments = segments.tolist()
    if len(segments) > MAX_SEGMENTS_PER_FRAME:
        step = len(segments) / MAX_SEGMENTS_PER_FRAME
        segments = [segments[int(index * step)]
                    for index in range(MAX_SEGMENTS_PER_FRAME)]
    flat = []
    for x0, y0, x1, y1, intensity in segments:
        flat.append(int(round(x0)))
        flat.append(int(round(y0)))
        flat.append(int(round(x1)))
        flat.append(int(round(y1)))
        flat.append(max(0, min(255, int(round(intensity * 255)))))
    return flat


def write_line(payload):
    """Emit one JSON line; returns False once the reader has gone away."""
    try:
        sys.stdout.write(json.dumps(payload))
        sys.stdout.write("\n")
        sys.stdout.flush()
        return True
    except (BrokenPipeError, OSError):
        return False


def main():
    set_process_name("phosphor-feed")
    computer = phosphor_signal.SegmentComputer()
    computer.set_sample_rate(CAPTURE_SAMPLE_RATE)

    target = find_default_monitor()
    if target is None:
        write_line({"error": "no output monitor source found"})
        return

    stream = phosphor_audio.AudioCaptureStream(
        on_stream_ended=lambda: None, sample_rate=CAPTURE_SAMPLE_RATE)
    stream.start(target)

    fps = FRAMES_PER_SECOND
    for index, argument in enumerate(sys.argv):
        if argument == "--fps" and index + 1 < len(sys.argv):
            try:
                fps = max(5, min(60, int(sys.argv[index + 1])))
            except ValueError:
                pass
    frame_interval = 1.0 / fps
    tracked_peak = 0.0
    try:
        while True:
            frame_start = time.monotonic()

            # Drain any commands the applet has sent without blocking. A
            # readable-but-empty stdin means the applet closed the pipe: quit.
            while select.select([sys.stdin], [], [], 0)[0]:
                command = sys.stdin.readline()
                if command == "":
                    return
                if not apply_command(command.strip(), computer):
                    return

            samples = stream.take_stereo_samples()

            frame_peak = max((abs(value) for value in samples), default=0.0)
            tracked_peak = (frame_peak if frame_peak > tracked_peak
                            else tracked_peak * AGC_RELEASE + frame_peak * (1 - AGC_RELEASE))
            if tracked_peak > AGC_NOISE_FLOOR:
                computer.gain = max(1.0, min(AGC_MAX_GAIN, AGC_TARGET_FILL / tracked_peak))
            else:
                computer.gain = computer.gain * 0.9 + 0.1   # ease back to unity when quiet

            segments = computer.compute(samples, COORDINATE_BOX, COORDINATE_BOX)
            if not write_line({"s": encode_segments(segments)}):
                return

            time.sleep(max(0.0, frame_interval - (time.monotonic() - frame_start)))
    finally:
        stream.stop()


if __name__ == "__main__":
    try:
        main()
    except (KeyboardInterrupt, BrokenPipeError):
        pass
