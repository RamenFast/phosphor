# SPDX-License-Identifier: GPL-3.0-or-later
"""ctypes bridge to Phosphor's native signal core (core/ -> Rust cdylib).

The library mirrors phosphor_signal.SegmentComputer and adds in-core
windowed-sinc oversampling, so high "Scope detail" rates no longer need the
full-rate audio piped through PulseAudio. Everything degrades gracefully:
if the .so is missing (or PHOSPHOR_NO_NATIVE is set), callers fall back to
the Python signal path and nothing else changes.
"""

import ctypes
import os
from array import array

try:
    import numpy
except ImportError:
    numpy = None

_PROJECT_DIRECTORY = os.path.dirname(os.path.abspath(__file__))
LIBRARY_LOCATIONS = (
    os.path.join(_PROJECT_DIRECTORY, "core", "target", "release",
                 "libphosphor_core.so"),
    os.path.join(_PROJECT_DIRECTORY, "libphosphor_core.so"),
    "/usr/lib/phosphor/libphosphor_core.so",
)
API_VERSION = 1

MODE_IDS = {"xy": 0, "xy45": 1, "xy_dots": 2, "waveform": 3,
            "spectrum": 4, "spectrum_radial": 5}


def _load_library():
    if os.environ.get("PHOSPHOR_NO_NATIVE"):
        return None
    for location in LIBRARY_LOCATIONS:
        try:
            library = ctypes.CDLL(location)
        except OSError:
            continue
        library.pc_version.restype = ctypes.c_uint32
        if library.pc_version() != API_VERSION:
            continue
        library.pc_new.restype = ctypes.c_void_p
        library.pc_free.argtypes = [ctypes.c_void_p]
        library.pc_configure.argtypes = [ctypes.c_void_p, ctypes.c_uint32,
                                         ctypes.c_uint32]
        library.pc_reset.argtypes = [ctypes.c_void_p]
        library.pc_compute.restype = ctypes.c_size_t
        library.pc_compute.argtypes = [
            ctypes.c_void_p, ctypes.c_uint32, ctypes.c_float, ctypes.c_float,
            ctypes.c_float, ctypes.c_void_p, ctypes.c_size_t, ctypes.c_float,
            ctypes.c_float, ctypes.c_void_p, ctypes.c_size_t]
        return library
    return None


_library = _load_library()


def available():
    return _library is not None


class NativeComputer:
    """One native computer handle; call sites serialize access themselves
    (Phosphor's compute lock), matching the Python SegmentComputer."""

    def __init__(self):
        if _library is None:
            raise RuntimeError("native core not available")
        self._handle = _library.pc_new()
        self.oversample = 1

    def __del__(self):
        handle, self._handle = getattr(self, "_handle", None), None
        if handle and _library is not None:
            _library.pc_free(handle)

    def configure(self, sample_rate, oversample=1):
        self.oversample = max(1, int(oversample))
        _library.pc_configure(self._handle, int(sample_rate), self.oversample)

    def reset(self):
        _library.pc_reset(self._handle)

    def compute(self, mode, gain, beam_energy, glow_keep, samples,
                width, height):
        """Segments for this frame as an (n, 5) float32 array (or a list of
        tuples when numpy is unavailable)."""
        if numpy is not None:
            sample_data = numpy.ascontiguousarray(samples, dtype=numpy.float32)
            sample_pointer = ctypes.c_void_p(
                sample_data.ctypes.data if len(sample_data) else None)
            float_count = len(sample_data)
        else:
            sample_data = (samples if isinstance(samples, array)
                           else array("f", samples))
            address, float_count = sample_data.buffer_info()
            sample_pointer = ctypes.c_void_p(address if float_count else None)
        # worst case: one segment per (oversampled) sample frame, plus the
        # waveform's two traces across the widest plausible viewport
        capacity = max((float_count // 2) * self.oversample + 8,
                       2 * int(width) + 64, 4096)
        if numpy is not None:
            out = numpy.empty((capacity, 5), dtype=numpy.float32)
            out_pointer = ctypes.c_void_p(out.ctypes.data)
        else:
            out = (ctypes.c_float * (capacity * 5))()
            out_pointer = ctypes.c_void_p(ctypes.addressof(out))
        count = _library.pc_compute(
            self._handle, MODE_IDS[mode], gain, beam_energy, glow_keep,
            sample_pointer, float_count, width, height, out_pointer, capacity)
        if numpy is not None:
            return out[:count]
        return [tuple(out[i * 5:i * 5 + 5]) for i in range(count)]
