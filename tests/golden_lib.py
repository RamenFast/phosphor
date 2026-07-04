# SPDX-License-Identifier: GPL-3.0-or-later
"""Shared machinery for the v3 golden fixtures (capture AND replay).

One module owns signal generation, the case table, and the case runner so
the recorder (capture_golden.py) and the verifier (test_golden_replay.py)
cannot drift apart: both feed the engine through run_case() below.

IMPORTANT: this module must not import phosphor_* at module level — the
callers decide PHOSPHOR_NO_NATIVE before the first engine import, so all
engine imports here are lazy (inside functions).
"""

import hashlib
import json
import os

import numpy

TESTS_DIRECTORY = os.path.dirname(os.path.abspath(__file__))
PROJECT = os.path.dirname(TESTS_DIRECTORY)
GOLDEN = os.path.join(TESTS_DIRECTORY, "golden")

# Engine parameters for every segment case — the parity test's values.
GAIN = 1.3
BEAM_ENERGY = 8.0
FRAME_GLOW_KEEP = 0.82
WIDTH, HEIGHT = 800, 600
CAMERA = {"yaw": 0.9, "pitch": 0.25, "dolly": 2.4}   # fixed 3D viewpoint

SIGNAL_SECONDS = 0.35        # long enough for every history-driven mode
SHORT_SECONDS = 0.15         # kit cases and the viewport case
CHUNK_CYCLE = (1600, 1024, 2048, 512, 3000)          # floats per compute()
OVERSAMPLE_CHUNK_CYCLE = (512, 1024, 800, 640)       # native os cases
OVERSAMPLE_CALLS = 12

ALL_MODES = ("xy", "xy45", "xy_swirl", "xy_dots", "xyz_takens", "helix",
             "waveform", "ring", "spectrum", "spectrum_radial", "tunnel")
THREE_D_MODES = ("xyz_takens", "helix")

# Kit chains reproduced verbatim from tests/test_native_parity.py.
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
STARTER_KITS = ("haunt", "heartbeat", "orbit")       # kits/*.phoskit


# --------------------------------------------------------------- signals --

def make_signal(name, sample_rate, seconds=SIGNAL_SECONDS):
    """Interleaved stereo float32 for one deterministic test signal.

    Everything is generated in float64 and rounded through float32 exactly
    once; the stored .f32 bytes are precisely what the engine consumes.
    """
    count = int(seconds * sample_rate)
    t = numpy.arange(count, dtype=numpy.float64) / sample_rate
    tau = 2.0 * numpy.pi
    if name == "sweep":              # the parity test's awkward pair
        frequency = 220.0 + 400.0 * t
        left = 0.6 * numpy.sin(tau * frequency * t)
        right = 0.6 * numpy.sin(tau * (frequency * 1.5) * t + 0.7)
    elif name == "sine":
        left = 0.6 * numpy.sin(tau * 440.0 * t)
        right = 0.6 * numpy.sin(tau * 220.0 * t)
    elif name == "chord":            # detuned just triads: the torus-maker
        left = 0.28 * (numpy.sin(tau * 220.0 * t)
                       + numpy.sin(tau * 275.0 * t)
                       + numpy.sin(tau * 330.0 * t))
        right = 0.28 * (numpy.sin(tau * 222.2 * t + 0.5)
                        + numpy.sin(tau * 277.75 * t + 0.5)
                        + numpy.sin(tau * 333.3 * t + 0.5))
    elif name == "burst":            # silence -> wideband hit -> quiet tail
        left = numpy.zeros(count, dtype=numpy.float64)
        right = numpy.zeros(count, dtype=numpy.float64)
        silence_end = int(0.12 * sample_rate)
        burst_end = min(count, silence_end + int(0.08 * sample_rate))
        local = t[silence_end:burst_end] - t[silence_end] if burst_end > silence_end else t[:0]
        amplitude = 0.8 / numpy.sqrt(40.0)
        for k in range(1, 41):
            left[silence_end:burst_end] += amplitude * numpy.sin(
                tau * (97.0 * k) * local + 0.7 * k * k)
            right[silence_end:burst_end] += amplitude * numpy.sin(
                tau * (97.0 * k) * local + 0.7 * k * k + 0.3)
        left[burst_end:] = 0.05 * numpy.sin(tau * 330.0 * t[burst_end:])
        right[burst_end:] = 0.05 * numpy.sin(tau * 331.0 * t[burst_end:])
    else:
        raise ValueError(f"unknown signal {name!r}")
    interleaved = numpy.empty(2 * count, dtype=numpy.float64)
    interleaved[0::2] = left
    interleaved[1::2] = right
    return interleaved.astype(numpy.float32)


def input_name(signal, sample_rate):
    return f"{signal}-{sample_rate}"


def chunk_plan(total_floats, cycle=CHUNK_CYCLE, max_calls=None):
    """The exact chunk sizes (floats) fed to compute(), full chunks only."""
    chunks = []
    consumed = 0
    index = 0
    while consumed + cycle[index % len(cycle)] <= total_floats:
        size = cycle[index % len(cycle)]
        chunks.append(size)
        consumed += size
        index += 1
        if max_calls is not None and len(chunks) >= max_calls:
            break
    return chunks


def recorded_calls(total_calls, head, tail):
    """Which call indices a case stores: the first `head` (cold start) and
    the last `tail` (mature state), deduplicated and ordered."""
    picked = sorted(set(list(range(min(head, total_calls)))
                        + list(range(max(0, total_calls - tail), total_calls))))
    return picked


# ------------------------------------------------------------ case table --

def _record_policy(mode):
    """(head, tail) recorded-call counts per mode family, sized so heavy
    emitters stay small while cheap ones keep broad coverage."""
    if mode in ("waveform", "ring", "helix"):
        return 1, 2
    if mode in ("spectrum", "spectrum_radial"):
        return 2, 6
    return 2, 4          # xy family, takens, tunnel, kit-over-mode


def reference_cases():
    """Every python-engine segment case, fully described (no engine calls)."""
    matrix = {
        "xy": ("sweep", "sine", "chord", "burst"),
        "xy45": ("sweep", "sine", "chord", "burst"),
        "xy_swirl": ("sweep", "sine"),
        "xy_dots": ("sweep", "sine"),
        "xyz_takens": ("sweep", "sine", "burst"),
        "helix": ("sweep", "sine", "burst"),
        "waveform": ("sweep", "sine", "chord", "burst"),
        "ring": ("sweep", "burst"),
        "spectrum": ("sweep", "sine", "chord", "burst"),
        "spectrum_radial": ("sweep", "sine", "chord", "burst"),
        "tunnel": ("sweep", "chord", "burst"),
    }
    cases = []
    for mode in ALL_MODES:
        for signal in matrix[mode]:
            for rate in (48000, 96000):
                cases.append(_segment_case(
                    f"{mode}__{signal}__{rate}", "numpy", mode, signal, rate))
    # python full-rate truth for the high detail rates (what the native
    # sinc oversampler approximates)
    for mode in ("xy", "xyz_takens", "waveform"):
        for rate in (192000, 384000):
            cases.append(_segment_case(
                f"{mode}__sweep__{rate}", "numpy", mode, "sweep", rate))
    # one aspect-ratio variant: the square postcard/export viewport
    square = _segment_case("xy__sweep__48000__720x720", "numpy", "xy",
                           "sweep", 48000, seconds=SHORT_SECONDS)
    square["width"] = square["height"] = 720
    cases.append(square)
    # kit chains over modes
    for kit_name, stages in KIT_CASES.items():
        for mode in ("xy", "spectrum"):
            case = _segment_case(
                f"kit-{kit_name}__{mode}__sweep__48000", "numpy", mode,
                "sweep", 48000, seconds=SHORT_SECONDS)
            case["kit"] = {"source": f"KIT_CASES:{kit_name}",
                           "stages": [[op, list(params)]
                                      for op, params in stages]}
            cases.append(case)
    return cases


def native_cases():
    """The rust-v3 secondary set: MODE_IDS modes plus production
    oversampling (pipe 96 kHz x2 and x4 — the 192k/384k detail plans)."""
    native_modes = ("xy", "xy45", "xy_dots", "waveform", "spectrum",
                    "spectrum_radial")
    cases = []
    for mode in native_modes:
        for signal in ("sweep", "sine"):
            for rate in (48000, 96000):
                cases.append(_segment_case(
                    f"{mode}__{signal}__{rate}", "rust-v3", mode, signal,
                    rate))
    for oversample in (2, 4):
        case = _segment_case(f"xy__sweep__96000__os{oversample}", "rust-v3",
                             "xy", "sweep", 96000)
        case["oversample"] = oversample
        case["chunks"] = chunk_plan(case["input"]["floats"],
                                    OVERSAMPLE_CHUNK_CYCLE, OVERSAMPLE_CALLS)
        case["recorded_calls"] = recorded_calls(len(case["chunks"]), 2, 2)
        cases.append(case)
    return cases


def _segment_case(name, engine, mode, signal, rate, seconds=SIGNAL_SECONDS):
    total_floats = 2 * int(seconds * rate)
    chunks = chunk_plan(total_floats)
    head, tail = _record_policy(mode)
    case = {
        "name": name,
        "engine": engine,
        "mode": mode,
        "sample_rate": rate,
        "oversample": 1,
        "gain": GAIN,
        "beam_energy": BEAM_ENERGY,
        "frame_glow_keep": FRAME_GLOW_KEEP,
        "width": WIDTH,
        "height": HEIGHT,
        "input": {"signal": signal, "file": input_name(signal, rate) + ".f32",
                  "floats": total_floats},
        "chunks": chunks,
        "recorded_calls": recorded_calls(len(chunks), head, tail),
    }
    if mode in THREE_D_MODES:
        case["camera"] = dict(CAMERA)
    return case


def kit_audio_cases():
    """Raw KitChain.process() captures — the pure DSP gate for v4."""
    cases = []
    for kit_name, stages in KIT_CASES.items():
        cases.append({
            "name": f"kitaudio-{kit_name}__sweep__48000",
            "kit": {"source": f"KIT_CASES:{kit_name}",
                    "stages": [[op, list(params)] for op, params in stages]},
            "sample_rate": 48000,
            "input": {"signal": "sweep", "file": "sweep-48000.f32",
                      "floats": 2 * int(SHORT_SECONDS * 48000)},
            "chunks": chunk_plan(2 * int(SHORT_SECONDS * 48000)),
        })
    for kit_name in ("rotate", "chandelay"):     # rate-dependent state
        cases.append({
            "name": f"kitaudio-{kit_name}__sweep__96000",
            "kit": {"source": f"KIT_CASES:{kit_name}",
                    "stages": [[op, list(params)]
                               for op, params in KIT_CASES[kit_name]]},
            "sample_rate": 96000,
            "input": {"signal": "sweep", "file": "sweep-96000.f32",
                      "floats": 2 * int(SHORT_SECONDS * 96000)},
            "chunks": chunk_plan(2 * int(SHORT_SECONDS * 96000)),
        })
    for kit_name in STARTER_KITS:
        cases.append({
            "name": f"kitaudio-starter-{kit_name}__sine__48000",
            "kit": {"source": f"kits/{kit_name}.phoskit", "stages": None},
            "sample_rate": 48000,
            "input": {"signal": "sine", "file": "sine-48000.f32",
                      "floats": 2 * int(SHORT_SECONDS * 48000)},
            "chunks": chunk_plan(2 * int(SHORT_SECONDS * 48000)),
        })
    return cases


# ------------------------------------------------------------ execution --

def load_starter_stages(kit_name):
    import phosphor_kit
    path = os.path.join(PROJECT, "kits", f"{kit_name}.phoskit")
    _name, _author, stages = phosphor_kit.load(path)
    return [[op, list(params)] for op, params in stages]


def resolve_kit_stages(case):
    """Canonical [(op, [p0..p3])] for a case's kit description."""
    kit = case.get("kit")
    if not kit:
        return None
    stages = kit.get("stages")
    if stages is None:                     # starter kit: load from kits/
        kit_name = kit["source"].split("/")[-1].split(".")[0]
        stages = load_starter_stages(kit_name)
    return [(op, list(params)) for op, params in stages]


def normalize_segments(segments):
    """Segment rows as canonical little-endian f32 bytes (n rows x 5).

    The numpy engine returns float32 ndarrays for the xy/takens/helix
    family and plain lists of tuples (float64 python math) for the
    waveform/ring/tunnel/spectrum family; the f32 cast here is the
    fixtures' storage precision, far inside the comparison tolerance.
    """
    if segments is None or len(segments) == 0:
        return b""
    rows = numpy.ascontiguousarray(
        numpy.asarray(segments, dtype="<f4").reshape(len(segments), 5))
    return rows.tobytes()


def run_case(case, input_bytes):
    """Feed a case exactly as recorded; returns (recorded_counts, bytes).

    recorded_counts are the per-call segment counts of the recorded calls
    only, in recorded order; bytes are those calls' rows concatenated.
    """
    import phosphor_signal

    computer = phosphor_signal.SegmentComputer()
    if case["engine"] == "numpy":
        computer._native = None            # belt over the env-var braces
        assert computer.engine == "numpy", computer.engine
    else:
        assert computer._native is not None, "native core required"
    computer.mode = case["mode"]
    computer.gain = case["gain"]
    computer.beam_energy = case["beam_energy"]
    computer.frame_glow_keep = case["frame_glow_keep"]
    computer.set_sample_rate(case["sample_rate"],
                             oversample=case.get("oversample", 1))
    camera = case.get("camera")
    if camera:
        computer.set_camera(**camera)
    stages = resolve_kit_stages(case)
    if stages:
        computer.set_kit(stages)

    samples = numpy.frombuffer(input_bytes, dtype="<f4")
    recorded_set = set(case["recorded_calls"])
    counts = []
    recorded = []
    offset = 0
    for index, size in enumerate(case["chunks"]):
        piece = samples[offset:offset + size]
        offset += size
        chunk_bytes = normalize_segments(
            computer.compute(piece, case["width"], case["height"]))
        if index in recorded_set:
            counts.append(len(chunk_bytes) // 20)
            recorded.append(chunk_bytes)
    return counts, b"".join(recorded)


def run_kit_audio_case(case, input_bytes):
    """KitChain.process() over the case's chunks; returns output bytes."""
    import phosphor_signal

    stages = resolve_kit_stages(case)
    chain = phosphor_signal.KitChain([(op, params) for op, params in stages])
    chain.configure(case["sample_rate"])
    samples = numpy.frombuffer(input_bytes, dtype="<f4")
    pieces = []
    offset = 0
    for size in case["chunks"]:
        piece = samples[offset:offset + size]
        offset += size
        processed = chain.process(piece)
        pieces.append(numpy.ascontiguousarray(
            numpy.asarray(processed, dtype="<f4")).tobytes())
    return b"".join(pieces)


def sha256(data):
    return hashlib.sha256(data).hexdigest()


def read_json(path):
    with open(path) as handle:
        return json.load(handle)


def write_json(path, payload):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w") as handle:
        json.dump(payload, handle, indent=1, sort_keys=True)
        handle.write("\n")
