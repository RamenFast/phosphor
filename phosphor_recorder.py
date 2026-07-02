# SPDX-License-Identifier: GPL-3.0-or-later
"""Snapshot and clip export for Phosphor.

Rather than screen-grabbing, both exports re-render the captured audio
through the same signal/renderer code the live scope uses — deterministic,
works no matter which renderer is on screen, and the clip gets the actual
audio muxed in. Snapshots go to ~/Pictures/Phosphor/, clips (mp4 with
sound, via ffmpeg) to ~/Videos/Phosphor/.
"""

import os
import struct
import subprocess
import tempfile
import threading
import time
import wave
from array import array

from phosphor_audio import DEFAULT_SAMPLE_RATE
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
    if settings.display_mode in ("xy", "xy45", "xy_swirl", "ring", "tunnel"):
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
