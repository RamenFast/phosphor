# SPDX-License-Identifier: GPL-3.0-or-later
"""Snapshot, clip, and full-track export for Phosphor.

Rather than screen-grabbing, every export re-renders audio through the
same signal/renderer code the live scope uses — deterministic, works no
matter which renderer is on screen, and the clip gets the actual audio
muxed in. Snapshots go to ~/Pictures/Phosphor/, clips (mp4 with sound,
via ffmpeg) to ~/Videos/Phosphor/.

`phosphor --render in.flac out.mp4` runs the same pipeline headless over
a whole file (render_main below): no window, no PulseAudio — decode,
trace, encode. Signal postcards (.phos) render too.
"""

import os
import struct
import subprocess
import sys
import tempfile
import threading
import time
import wave
from array import array

from phosphor_audio import (DEFAULT_SAMPLE_RATE, phos_header,
                            probe_duration_seconds)
from phosphor_render_cairo import OfflineFrameRenderer
from phosphor_settings import grid_spacing_fraction
from phosphor_signal import SegmentComputer

EXPORT_FPS = 60
SNAPSHOT_DIRECTORY = os.path.expanduser("~/Pictures/Phosphor")
CLIP_DIRECTORY = os.path.expanduser("~/Videos/Phosphor")
SNAPSHOT_WARMUP_SECONDS = 1.2   # enough audio to rebuild the glow trails


def _timestamped_path(directory, extension):
    os.makedirs(directory, exist_ok=True)
    return os.path.join(directory, time.strftime(f"phosphor-%Y%m%d-%H%M%S.{extension}"))


def _build_offline_pipeline(settings, width, height, sample_rate, gain=None,
                            oversample=1):
    gain = settings.gain if gain is None else gain   # auto-gain passes its own
    computer = SegmentComputer()
    computer.mode = settings.display_mode
    computer.gain = gain
    computer.beam_energy = settings.beam_energy
    computer.set_sample_rate(sample_rate, oversample)
    if getattr(settings, "kit_enabled", False) and settings.kit_path:
        try:
            import phosphor_kit
            _name, _author, stages = phosphor_kit.load(settings.kit_path)
            computer.set_kit(stages)
        except (OSError, ValueError):
            pass    # a broken kit shouldn't kill an export; render it plain
    renderer = OfflineFrameRenderer(width, height, settings.current_theme(),
                                    settings.persistence, settings.grid_enabled,
                                    beam_focus=settings.beam_focus,
                                    grid_spacing_fraction=grid_spacing_fraction(
                                        gain))
    return computer, renderer


def _frames_from_audio(audio_bytes, settings, width, height, sample_rate,
                       gain=None, oversample=1):
    """Yield composited frame surfaces, one per 1/EXPORT_FPS of audio."""
    computer, renderer = _build_offline_pipeline(settings, width, height,
                                                 sample_rate, gain, oversample)
    samples = array("f")
    samples.frombytes(audio_bytes[:len(audio_bytes) - len(audio_bytes) % 8])
    stereo_per_frame = 2 * (sample_rate // EXPORT_FPS)
    for start in range(0, len(samples) - stereo_per_frame + 1, stereo_per_frame):
        chunk = samples[start:start + stereo_per_frame]
        segments = computer.compute(chunk, width, height)
        yield renderer.render_frame(segments), renderer


def export_size(settings):
    if settings.display_mode in ("xy", "xy45", "xy_swirl", "ring", "tunnel",
                                 "xyz_takens", "helix"):
        return 720, 720
    return 1080, 720


def save_snapshot(audio_bytes, settings, sample_rate=DEFAULT_SAMPLE_RATE,
                  gain=None, oversample=1):
    """Re-render the last moment of audio and save the final frame as PNG."""
    width, height = export_size(settings)
    warmup_bytes = int(SNAPSHOT_WARMUP_SECONDS * sample_rate) * 8
    surface = None
    for surface, _renderer in _frames_from_audio(audio_bytes[-warmup_bytes:],
                                                 settings, width, height,
                                                 sample_rate, gain, oversample):
        pass
    if surface is None:
        raise RuntimeError("not enough captured audio yet")
    output_path = _timestamped_path(SNAPSHOT_DIRECTORY, "png")
    surface.write_to_png(output_path)
    return output_path


def _write_wav(audio_bytes, wav_path, sample_rate):
    """float32 stereo -> 16-bit WAV for muxing."""
    samples = array("f")
    samples.frombytes(audio_bytes[:len(audio_bytes) - len(audio_bytes) % 8])
    with wave.open(wav_path, "w") as wav_file:
        wav_file.setnchannels(2)
        wav_file.setsampwidth(2)
        wav_file.setframerate(sample_rate)
        clipped = (max(-1.0, min(1.0, value)) for value in samples)
        wav_file.writeframes(b"".join(
            struct.pack("<h", int(value * 32767)) for value in clipped))


def save_clip(audio_bytes, settings, progress_callback=None,
              sample_rate=DEFAULT_SAMPLE_RATE, gain=None, oversample=1):
    """Render audio to an mp4 (video + the audio itself). Blocking; run in a
    thread. progress_callback(fraction) is called from that thread."""
    width, height = export_size(settings)
    output_path = _timestamped_path(CLIP_DIRECTORY, "mp4")
    total_frames = max(1, (len(audio_bytes) // 8) // (sample_rate // EXPORT_FPS))

    with tempfile.NamedTemporaryFile(suffix=".wav", delete=False) as wav_handle:
        wav_path = wav_handle.name
    try:
        _write_wav(audio_bytes, wav_path, sample_rate)
        ffmpeg = subprocess.Popen(
            [
                "ffmpeg", "-y", "-loglevel", "error",
                "-f", "rawvideo", "-pix_fmt", "bgra",
                "-s", f"{width}x{height}", "-r", str(EXPORT_FPS), "-i", "-",
                "-i", wav_path,
                "-c:v", "libx264", "-preset", "veryfast", "-crf", "18",
                "-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest",
                output_path,
            ],
            stdin=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
        frame_index = 0
        for surface, renderer in _frames_from_audio(audio_bytes, settings,
                                                    width, height, sample_rate,
                                                    gain, oversample):
            ffmpeg.stdin.write(renderer.frame_bytes())
            frame_index += 1
            if progress_callback and frame_index % 30 == 0:
                progress_callback(frame_index / total_frames)
        ffmpeg.stdin.close()
        error_output = ffmpeg.stderr.read().decode(errors="replace")
        if ffmpeg.wait() != 0:
            raise RuntimeError(f"ffmpeg failed: {error_output[:300]}")
    finally:
        os.unlink(wav_path)
    return output_path


def _decode_arguments(path):
    """ffmpeg input arguments for `path` — raw s16le flags when it's a
    .phos signal postcard, nothing extra otherwise."""
    phos = phos_header(path)
    if phos is None:
        return []
    import phosphor_precompute
    return ["-skip_initial_bytes", str(phosphor_precompute.HEADER_BYTES),
            "-f", "s16le", "-ar", str(int(phos["rate"])), "-ac", "2"]


def render_file(input_path, output_path, settings, sample_rate,
                progress_callback=None):
    """Render a whole audio file to mp4 through the live pipeline,
    streaming (no full-track buffer): one ffmpeg decodes to the scope
    rate, frames are traced and piped to a second ffmpeg that encodes
    video and muxes the original audio. Returns output_path."""
    width, height = export_size(settings)
    computer, renderer = _build_offline_pipeline(settings, width, height,
                                                 sample_rate)
    duration = probe_duration_seconds(input_path)
    total_frames = int((duration or 0) * EXPORT_FPS) or None
    input_arguments = _decode_arguments(input_path)

    decoder = subprocess.Popen(
        ["ffmpeg", "-v", "error", *input_arguments, "-i", input_path,
         "-f", "f32le", "-ac", "2", "-ar", str(sample_rate), "-"],
        stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    encoder = subprocess.Popen(
        ["ffmpeg", "-y", "-loglevel", "error",
         "-f", "rawvideo", "-pix_fmt", "bgra",
         "-s", f"{width}x{height}", "-r", str(EXPORT_FPS), "-i", "-",
         *input_arguments, "-i", input_path,
         "-map", "0:v", "-map", "1:a",
         "-c:v", "libx264", "-preset", "veryfast", "-crf", "18",
         "-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest", output_path],
        stdin=subprocess.PIPE, stderr=subprocess.PIPE)

    bytes_per_frame = (sample_rate // EXPORT_FPS) * 8
    frame_index = 0
    try:
        while True:
            chunk = decoder.stdout.read(bytes_per_frame)
            if len(chunk) < bytes_per_frame:
                break               # trailing partial frame: done
            samples = array("f")
            samples.frombytes(chunk)
            segments = computer.compute(samples, width, height)
            renderer.render_frame(segments)
            encoder.stdin.write(renderer.frame_bytes())
            frame_index += 1
            if progress_callback and frame_index % 60 == 0:
                progress_callback(frame_index, total_frames)
        decoder_error = decoder.stderr.read().decode(errors="replace")
        if decoder.wait() != 0:
            raise RuntimeError(f"decode failed: {decoder_error[:300]}")
        encoder.stdin.close()
        encoder_error = encoder.stderr.read().decode(errors="replace")
        if encoder.wait() != 0:
            raise RuntimeError(f"encode failed: {encoder_error[:300]}")
    except BaseException:
        decoder.kill()
        encoder.kill()
        raise
    if frame_index == 0:
        raise RuntimeError("no audio decoded — is the input an audio file?")
    return output_path


def render_main(arguments):
    """`phosphor --render input output.mp4 [--rate N]` — exit code 0 on
    success, 1 on failure, 2 on usage errors (argparse's convention)."""
    import argparse
    parser = argparse.ArgumentParser(
        prog="phosphor --render",
        description="Render a whole audio file (or .phos signal postcard) "
                    "to mp4 through Phosphor's offline pipeline, using the "
                    "saved settings for mode, theme, and gain.")
    parser.add_argument("input", help="audio file or .phos stream")
    parser.add_argument("output", help="output .mp4 path")
    parser.add_argument("--rate", type=int, default=None,
                        help="scope detail rate in Hz "
                             "(default: the saved Scope detail setting)")
    options = parser.parse_args(
        [argument for argument in arguments if argument != "--render"])

    from phosphor_settings import Settings
    settings = Settings.load()
    sample_rate = options.rate or settings.scope_sample_rate

    def report(done, total):
        if total:
            print(f"\r  {done}/{total} frames "
                  f"({100 * done // total}%)", end="", flush=True)
        else:
            print(f"\r  {done} frames", end="", flush=True)

    print(f"rendering {options.input} → {options.output} "
          f"[{settings.display_mode} @ {sample_rate} Hz]")
    try:
        render_file(options.input, options.output, settings, sample_rate,
                    progress_callback=report)
    except (RuntimeError, OSError) as error:
        print(f"\nrender failed: {error}", file=sys.stderr)
        return 1
    print(f"\ndone: {options.output}")
    return 0


def save_clip_async(audio_bytes, settings, on_progress, on_done, on_error,
                    sample_rate=DEFAULT_SAMPLE_RATE, gain=None, oversample=1):
    """Fire-and-forget thread wrapper; callbacks receive plain values and are
    the caller's job to marshal onto the UI thread."""
    def worker():
        try:
            path = save_clip(audio_bytes, settings, progress_callback=on_progress,
                             sample_rate=sample_rate, gain=gain,
                             oversample=oversample)
            on_done(path)
        except (RuntimeError, OSError) as error:
            on_error(str(error))
    thread = threading.Thread(target=worker, daemon=True)
    thread.start()
    return thread
