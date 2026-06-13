# SPDX-License-Identifier: GPL-3.0-or-later
"""Audio capture for Phosphor.

Two kinds of capture target:
  - a device: any output's monitor (everything that sink plays) or a mic,
    captured with `parec --device=...`
  - a single application: one playing stream (a "sink input"), captured with
    `parec --monitor-stream=INDEX` — so you can scope just the music player
    while a game makes noise, or vice versa.

The stream also keeps a rolling ring of the last CLIP_SECONDS of audio so
snapshots and clip exports can re-render what you just saw and heard.
"""

import os
import re
import signal
import subprocess
import threading
from array import array

DEFAULT_SAMPLE_RATE = 48000
BYTES_PER_STEREO_FRAME = 8  # 2 channels x float32
PENDING_BACKLOG_SECONDS = 1
CLIP_SECONDS = 10


def _run_pactl(arguments):
    try:
        return subprocess.run(
            ["pactl"] + arguments, capture_output=True, text=True, timeout=5
        ).stdout
    except (OSError, subprocess.TimeoutExpired):
        return ""


class CaptureTarget:
    """Something parec can record: kind is 'device' or 'app'.

    App targets are keyed by application name, not sink-input index: indexes
    are reassigned every time playback restarts (every song change in a
    browser), so an index-keyed selection would go stale immediately. The
    name stays put, letting the scope re-find "Google Chrome" forever.
    """

    def __init__(self, kind, identifier, label, stable_key=None):
        self.kind = kind
        self.identifier = identifier      # device name, or sink-input index
        self.label = label
        self.stable_key = stable_key or identifier

    @property
    def combo_id(self):
        return f"{self.kind}:{self.stable_key}"

    def parec_argument(self):
        if self.kind == "app":
            return f"--monitor-stream={self.identifier}"
        return f"--device={self.identifier}"


def list_capture_targets():
    """All capturable things: playing apps first, then monitors, then mics."""
    targets = []
    seen_app_keys = set()

    for block in re.split(r"^Sink Input #", _run_pactl(["list", "sink-inputs"]), flags=re.MULTILINE):
        index_match = re.match(r"(\d+)", block)
        if not index_match:
            continue
        index = index_match.group(1)
        application = re.search(r'application\.name = "(.*?)"', block)
        media_title = re.search(r'media\.name = "(.*?)"', block)
        parts = [match.group(1) for match in (application, media_title) if match]
        label = " — ".join(parts) or f"stream #{index}"
        app_key = application.group(1) if application else f"stream-{index}"
        while app_key in seen_app_keys:        # two streams from one app
            app_key += "+"
        seen_app_keys.add(app_key)
        targets.append(CaptureTarget("app", index, f"APP · {label}",
                                     stable_key=app_key))

    monitors, microphones = [], []
    for block in re.split(r"^Source #\d+", _run_pactl(["list", "sources"]), flags=re.MULTILINE):
        name_match = re.search(r"^\s*Name:\s*(\S+)", block, re.MULTILINE)
        description_match = re.search(r"^\s*Description:\s*(.+)$", block, re.MULTILINE)
        if not name_match:
            continue
        device_name = name_match.group(1)
        description = description_match.group(1).strip() if description_match else device_name
        if device_name.endswith(".monitor"):
            if description.startswith("Monitor of "):
                description = description[len("Monitor of "):]
            monitors.append(CaptureTarget("device", device_name, f"OUT · {description}"))
        else:
            microphones.append(CaptureTarget("device", device_name, f"IN · {description}"))

    return targets + sorted(monitors, key=lambda t: t.label) + sorted(microphones, key=lambda t: t.label)


def default_monitor_target_id():
    sink = _run_pactl(["get-default-sink"]).strip()
    return f"device:{sink}.monitor" if sink else None


def probe_duration_seconds(path):
    """Length of an audio file via ffprobe; None if it can't be determined."""
    try:
        output = subprocess.run(
            ["ffprobe", "-v", "error", "-show_entries", "format=duration",
             "-of", "default=noprint_wrappers=1:nokey=1", path],
            capture_output=True, text=True, timeout=5,
        ).stdout.strip()
        return float(output) if output else None
    except (OSError, subprocess.TimeoutExpired, ValueError):
        return None


class AudioCaptureStream:
    """Owns the parec process. While stopped, nothing runs and nothing polls."""

    def __init__(self, on_stream_ended, sample_rate=DEFAULT_SAMPLE_RATE):
        self._process = None
        self._playback_process = None      # pacat, only during file playback
        self.playback_paused = False
        self._reader_thread = None
        self._pending_bytes = bytearray()
        self._history_bytes = bytearray()  # rolling last CLIP_SECONDS for export
        self._lock = threading.Lock()
        self._on_stream_ended = on_stream_ended  # called from reader thread
        self._streamed_bytes = 0           # decoded so far, for seek position
        self._stream_start_seconds = 0.0   # seek offset of the current stream
        self.configure_sample_rate(sample_rate)

    def configure_sample_rate(self, sample_rate):
        """Set the scope feed rate; takes effect on the next start.

        Devices are resampled by PulseAudio/PipeWire, files by ffmpeg — both
        do proper sinc-style reconstruction, so above 48 kHz the scope traces
        the true inter-sample curves instead of straight lines.
        """
        self.sample_rate = sample_rate
        self._max_pending_bytes = (sample_rate * BYTES_PER_STEREO_FRAME
                                   * PENDING_BACKLOG_SECONDS)
        self._max_history_bytes = (sample_rate * BYTES_PER_STEREO_FRAME
                                   * CLIP_SECONDS)

    @property
    def is_running(self):
        return self._process is not None

    @property
    def playback_position_seconds(self):
        """How far into the current file playback we are, in seconds."""
        return (self._stream_start_seconds + self._streamed_bytes
                / (self.sample_rate * BYTES_PER_STEREO_FRAME))

    def start(self, target):
        self.stop()
        self._process = subprocess.Popen(
            [
                "parec",
                target.parec_argument(),
                "--format=float32le",
                f"--rate={self.sample_rate}",
                "--channels=2",
                "--latency-msec=20",
                "--raw",
            ],
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
        )
        self._start_reader(self._process, playback=None)

    def start_file(self, path, seek_seconds=0.0, loop=False):
        """Play an audio file out loud while feeding the scope directly.

        ffmpeg decodes to the scope's native format; pacat makes it audible.
        pacat's pipe backpressure paces the reader loop, keeping the picture
        in sync with the sound. `seek_seconds` starts mid-file; `loop`
        repeats the file forever (used by compose mode's drawn loops).
        """
        self.stop()
        decoder_command = ["ffmpeg", "-v", "quiet"]
        if seek_seconds > 0.0:
            decoder_command += ["-ss", f"{seek_seconds:.3f}"]
        if loop:
            decoder_command += ["-stream_loop", "-1"]
        decoder_command += ["-i", path, "-f", "f32le", "-ac", "2",
                            "-ar", str(self.sample_rate), "-"]
        decoder = subprocess.Popen(
            decoder_command,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
        )
        playback = subprocess.Popen(
            [
                "pacat",
                "--format=float32le",
                f"--rate={self.sample_rate}",
                "--channels=2",
                "--latency-msec=60",
                "--client-name=Phosphor",
                "--raw",
            ],
            stdin=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
        )
        self._process = decoder
        self._playback_process = playback
        self._start_reader(decoder, playback, stream_start_seconds=seek_seconds)

    def _start_reader(self, process, playback, stream_start_seconds=0.0):
        self._stream_start_seconds = stream_start_seconds
        self._streamed_bytes = 0
        self._reader_thread = threading.Thread(
            target=self._reader_loop, args=(process, playback), daemon=True
        )
        self._reader_thread.start()

    def set_playback_paused(self, paused):
        """Freeze/unfreeze file playback. SIGSTOP holds both the decoder and
        pacat in place, so resuming continues exactly where the sound left
        off — no buffer to rebuild, no seek needed."""
        if self._playback_process is None or paused == self.playback_paused:
            return
        stop_or_continue = signal.SIGSTOP if paused else signal.SIGCONT
        for child in (self._process, self._playback_process):
            if child is not None and child.poll() is None:
                try:
                    os.kill(child.pid, stop_or_continue)
                except ProcessLookupError:
                    pass
        self.playback_paused = paused

    def stop(self):
        self.set_playback_paused(False)    # SIGTERM stays pending on a stopped process
        process, self._process = self._process, None
        playback, self._playback_process = self._playback_process, None
        for child in (process, playback):
            if child is None:
                continue
            child.terminate()
            try:
                child.wait(timeout=2)
            except subprocess.TimeoutExpired:
                child.kill()
        with self._lock:
            self._pending_bytes.clear()
            # history is kept so a clip can still be saved right after pausing

    def _reader_loop(self, process, playback):
        while True:
            chunk = process.stdout.read(8192)
            if not chunk:
                break
            if playback is not None:
                try:
                    playback.stdin.write(chunk)   # blocks → paces file playback
                    playback.stdin.flush()
                except (BrokenPipeError, OSError):
                    break
            self._streamed_bytes += len(chunk)
            with self._lock:
                self._pending_bytes.extend(chunk)
                self._trim_front(self._pending_bytes, self._max_pending_bytes)
                self._history_bytes.extend(chunk)
                self._trim_front(self._history_bytes, self._max_history_bytes)
        if playback is not None and playback.stdin is not None:
            try:
                playback.stdin.close()    # let the tail of the file drain
                playback.wait(timeout=5)
            except (OSError, subprocess.TimeoutExpired):
                pass
        if self._process is process:
            # The stream ended on its own (file finished, app stopped, device gone…).
            self._on_stream_ended()

    @staticmethod
    def _trim_front(buffer, max_bytes):
        """Cap a rolling buffer, amortized: deleting the front of a bytearray
        memmoves everything behind it, so wait until the overshoot is worth
        one big move instead of paying a full-buffer move on every chunk."""
        overflow = len(buffer) - max_bytes
        if overflow > max_bytes // 4:
            overflow -= overflow % BYTES_PER_STEREO_FRAME  # keep frame alignment
            del buffer[:overflow]

    def take_stereo_samples(self):
        """Drain captured audio as a flat float array [L, R, L, R, ...]."""
        with self._lock:
            usable = len(self._pending_bytes) - (len(self._pending_bytes) % BYTES_PER_STEREO_FRAME)
            if usable == 0:
                return array("f")
            raw = bytes(self._pending_bytes[:usable])
            del self._pending_bytes[:usable]
        samples = array("f")
        samples.frombytes(raw)
        return samples

    def copy_history(self, seconds=CLIP_SECONDS):
        """The most recent audio as raw float32 stereo bytes, for export."""
        with self._lock:
            wanted = min(len(self._history_bytes),
                         int(seconds * self.sample_rate) * BYTES_PER_STEREO_FRAME)
            raw = bytes(self._history_bytes[-wanted:]) if wanted else b""
        return raw[len(raw) % BYTES_PER_STEREO_FRAME:]
