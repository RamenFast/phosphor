# SPDX-License-Identifier: GPL-3.0-or-later
"""Precomputed scope streams: trade disk for a guaranteed-perfect trace.

A .phos file holds a track's fully decoded, sinc-reconstructed sample stream
at the chosen scope-detail rate (int16 stereo). At play time the scope reads
it by the playback clock instead of chasing a realtime pipe, so:

  - the audible pipe drops to 48 kHz (sound doesn't need 384 kHz),
  - the reconstruction math is already done, however slow the machine,
  - a slow frame simply traces more of the stream late — samples are never
    dropped, and seeks are just an index jump.

The stream is stored in signal space, so it serves every XY mode at any
gain, window size, or frame rate — one file per (track content, rate).
Generation is one ffmpeg decode+resample pass, faster than realtime.

File layout: a fixed 256-byte header record ("PHOSC1" + JSON, space-padded)
followed by raw interleaved s16le stereo frames.
"""

import hashlib
import json
import mmap
import os
import subprocess
from array import array

try:
    import numpy
except ImportError:
    numpy = None

from phosphor_audio import probe_duration_seconds

CACHE_DIRECTORY = os.path.expanduser("~/.local/share/phosphor/precomputed")
POSTCARD_DIRECTORY = os.path.expanduser("~/Music/Phosphor")
AUDIO_PIPE_RATE = 48000     # audible pipe while the scope reads from cache
MAGIC = b"PHOSC1"
HEADER_BYTES = 256
INT16_SCALE = 32767.0


def pack_header(fields):
    """The fixed 256-byte header record for `fields`. The JSON must fit the
    record, so free-text fields (title, credit, source) are trimmed until it
    does — a shared .phos should never fail over a long album title."""
    for keep in (80, 48, 24, 8, 0):
        candidate = dict(fields)
        for key in ("title", "credit", "source"):
            value = candidate.get(key)
            if value:
                candidate[key] = value[:keep]
            if not candidate.get(key):
                candidate.pop(key, None)
        encoded = MAGIC + json.dumps(candidate, ensure_ascii=False).encode()
        if len(encoded) <= HEADER_BYTES - 1:
            return encoded.ljust(HEADER_BYTES - 1) + b"\n"
    raise ValueError("phos header does not fit")


def read_header(path):
    """The header fields of a .phos file, or None if it isn't one."""
    try:
        with open(path, "rb") as source:
            header = source.read(HEADER_BYTES)
    except OSError:
        return None
    if not header.startswith(MAGIC):
        return None
    try:
        return json.loads(header[len(MAGIC):].decode().strip())
    except (UnicodeDecodeError, ValueError):
        return None


def _content_key(path, rate):
    """Cache key from the file's size plus head+tail content — stable across
    renames and touch, changes when the audio itself changes."""
    digest = hashlib.sha1()
    size = os.path.getsize(path)
    digest.update(str(size).encode())
    with open(path, "rb") as source:
        digest.update(source.read(131072))
        if size > 262144:
            source.seek(-131072, os.SEEK_END)
            digest.update(source.read(131072))
    return f"{digest.hexdigest()[:20]}-{rate // 1000}k"


def cache_path_for(path, rate):
    return os.path.join(CACHE_DIRECTORY, _content_key(path, rate) + ".phos")


class PrecomputedTrack:
    """Random access into a .phos stream, memory-mapped (no RAM blowup)."""

    def __init__(self, path):
        self.path = path
        self._file = open(path, "rb")
        header = self._file.read(HEADER_BYTES)
        if not header.startswith(MAGIC):
            self._file.close()
            raise ValueError("not a phos stream")
        fields = json.loads(header[len(MAGIC):].decode().strip())
        self.sample_rate = int(fields["rate"])
        self.frame_count = int(fields["frames"])
        self.source_name = fields.get("source", "")
        self.title = fields.get("title") or ""
        self.credit = fields.get("credit") or ""
        self._map = mmap.mmap(self._file.fileno(), 0, access=mmap.ACCESS_READ)

    @property
    def duration_seconds(self):
        return self.frame_count / self.sample_rate

    def close(self):
        try:
            self._map.close()
            self._file.close()
        except OSError:
            pass

    def samples_between(self, start_seconds, end_seconds):
        """Interleaved float32 stereo covering [start, end) of the track."""
        first = max(0, min(self.frame_count, int(start_seconds * self.sample_rate)))
        last = max(0, min(self.frame_count, int(end_seconds * self.sample_rate)))
        if last <= first:
            return numpy.empty(0, dtype=numpy.float32) if numpy is not None \
                else array("f")
        raw = self._map[HEADER_BYTES + first * 4:HEADER_BYTES + last * 4]
        if numpy is not None:
            return (numpy.frombuffer(raw, dtype=numpy.int16)
                    .astype(numpy.float32) / INT16_SCALE)
        integers = array("h")
        integers.frombytes(raw)
        return array("f", (value / INT16_SCALE for value in integers))


def find(path, rate):
    """The matching PrecomputedTrack, or None."""
    try:
        return PrecomputedTrack(cache_path_for(path, rate))
    except (OSError, ValueError, KeyError):
        return None


def generate(path, rate, progress_callback=None):
    """Decode + reconstruct `path` at `rate` into the cache (blocking; run on
    a worker thread). Returns the PrecomputedTrack. ffmpeg's swr does the
    band-limited resampling — the same math as the live pipe, done once."""
    os.makedirs(CACHE_DIRECTORY, exist_ok=True)
    final_path = cache_path_for(path, rate)
    scratch_path = final_path + ".part"
    duration = probe_duration_seconds(path)
    expected_bytes = int((duration or 0) * rate) * 4

    decoder = subprocess.Popen(
        ["ffmpeg", "-v", "error", "-i", path, "-f", "s16le", "-ac", "2",
         "-ar", str(rate), "-"],
        stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    written = 0
    try:
        with open(scratch_path, "wb") as sink:
            sink.write(b"\0" * HEADER_BYTES)   # placeholder, rewritten below
            while True:
                chunk = decoder.stdout.read(1 << 20)
                if not chunk:
                    break
                sink.write(chunk)
                written += len(chunk)
                if progress_callback and expected_bytes:
                    progress_callback(min(1.0, written / expected_bytes))
            error_output = decoder.stderr.read().decode(errors="replace")
            if decoder.wait() != 0 or written < 4:
                raise RuntimeError(f"decode failed: {error_output[:200]}")
            sink.seek(0)
            sink.write(pack_header({
                "rate": rate,
                "frames": written // 4,
                "source": os.path.basename(path),
            }))
        os.replace(scratch_path, final_path)
    except BaseException:
        decoder.kill()
        try:
            os.unlink(scratch_path)
        except OSError:
            pass
        raise
    return PrecomputedTrack(final_path)


def export_postcard(source_phos_path, destination_path, title, credit):
    """Copy a .phos stream with title/credit stamped into its header — the
    shareable "signal postcard". Returns the destination path."""
    fields = read_header(source_phos_path)
    if fields is None:
        raise ValueError("not a phos stream")
    if title:
        fields["title"] = title
    if credit:
        fields["credit"] = credit
    destination_directory = os.path.dirname(destination_path)
    if destination_directory:
        os.makedirs(destination_directory, exist_ok=True)
    scratch_path = destination_path + ".part"
    try:
        with open(source_phos_path, "rb") as source, \
                open(scratch_path, "wb") as sink:
            source.seek(HEADER_BYTES)
            sink.write(pack_header(fields))
            while True:
                chunk = source.read(1 << 20)
                if not chunk:
                    break
                sink.write(chunk)
        os.replace(scratch_path, destination_path)
    except BaseException:
        try:
            os.unlink(scratch_path)
        except OSError:
            pass
        raise
    return destination_path


def postcard_path_for(source_path, rate, title=None):
    """Default export location: ~/Music/Phosphor/<name>-<rate>k.phos."""
    stem = (title or
            os.path.splitext(os.path.basename(source_path))[0] or "postcard")
    safe = "".join(ch if ch.isalnum() or ch in " -_." else "_"
                   for ch in stem).strip() or "postcard"
    return os.path.join(POSTCARD_DIRECTORY,
                        f"{safe}-{rate // 1000}k.phos")


def cache_size_bytes():
    total = 0
    try:
        for name in os.listdir(CACHE_DIRECTORY):
            if name.endswith(".phos"):
                total += os.path.getsize(os.path.join(CACHE_DIRECTORY, name))
    except OSError:
        pass
    return total


def clear_cache():
    """Delete every cached stream; returns bytes freed."""
    freed = 0
    try:
        for name in os.listdir(CACHE_DIRECTORY):
            if name.endswith(".phos") or name.endswith(".part"):
                full_path = os.path.join(CACHE_DIRECTORY, name)
                freed += os.path.getsize(full_path)
                os.unlink(full_path)
    except OSError:
        pass
    return freed
