# SPDX-License-Identifier: GPL-3.0-or-later
"""Replay the golden fixtures against the current engine — exact bytes.

While the v3 Python engine is alive, every fixture must replay to the
byte; this test is the proof that the capture harness itself is sound.
The v4 Rust engine compares against the same fixtures with the tolerance
contract in tests/golden/README.md instead of byte equality.

Run directly:  python3 tests/test_golden_replay.py
"""

import os
import subprocess
import sys

os.environ["PHOSPHOR_NO_NATIVE"] = "1"     # before any phosphor import

TESTS_DIRECTORY = os.path.dirname(os.path.abspath(__file__))
PROJECT = os.path.dirname(TESTS_DIRECTORY)
sys.path.insert(0, TESTS_DIRECTORY)
sys.path.insert(0, PROJECT)

import golden_lib as lib


def _load_case(directory, name):
    case = lib.read_json(os.path.join(directory, name + ".json"))
    with open(os.path.join(directory, name + ".segments.bin"),
              "rb") as handle:
        return case, handle.read()


def _input_bytes(case):
    path = os.path.join(lib.GOLDEN, "inputs", case["input"]["file"])
    with open(path, "rb") as handle:
        data = handle.read()
    data = data[:case["input"]["floats"] * 4]
    assert lib.sha256(data) == case["input"]["sha256"], (
        f"{case['name']}: input {case['input']['file']} changed on disk")
    return data


def _replay_segment_set(directory, names):
    for name in names:
        case, expected = _load_case(directory, name)
        counts, produced = lib.run_case(case, _input_bytes(case))
        assert counts == case["segment_counts_recorded"], (
            f"{name}: recorded-call segment counts diverged: "
            f"{counts} != {case['segment_counts_recorded']}")
        assert produced == expected, (
            f"{name}: segment bytes diverged from golden "
            f"({len(produced)} vs {len(expected)} bytes)")
    label = os.path.relpath(directory, lib.GOLDEN)
    print(f"  {len(names)} cases replay exact ({label})")


def test_reference_segments():
    manifest = lib.read_json(os.path.join(lib.GOLDEN, "manifest.json"))
    _replay_segment_set(os.path.join(lib.GOLDEN, "cases"),
                        manifest["cases"])


def test_kit_audio():
    import phosphor_kit
    manifest = lib.read_json(os.path.join(lib.GOLDEN, "manifest.json"))
    directory = os.path.join(lib.GOLDEN, "kits")
    for name in manifest["kit_audio"]:
        case = lib.read_json(os.path.join(directory, name + ".json"))
        source = case["kit"]["source"]
        if source.startswith("kits/"):     # starter kit: loader agreement
            _kit_name, _author, loaded = phosphor_kit.load(
                os.path.join(PROJECT, source))
            loaded = [[op, list(params)] for op, params in loaded]
            assert loaded == case["kit"]["stages"], (
                f"{name}: {source} no longer canonicalizes to the "
                f"captured stages")
        with open(os.path.join(directory, case["output"]["file"]),
                  "rb") as handle:
            expected = handle.read()
        assert lib.sha256(expected) == case["output"]["sha256"]
        produced = lib.run_kit_audio_case(case, _input_bytes(case))
        assert produced == expected, f"{name}: kit audio diverged"
    print(f"  {len(manifest['kit_audio'])} kit audio captures replay exact")


def test_phos_round_trips():
    import phosphor_precompute
    manifest = lib.read_json(os.path.join(lib.GOLDEN, "manifest.json"))
    directory = os.path.join(lib.GOLDEN, "phos")
    for name in manifest["phos"]:
        record = lib.read_json(os.path.join(directory, name + ".json"))
        path = os.path.join(directory, record["file"])
        with open(path, "rb") as handle:
            file_bytes = handle.read()
        assert lib.sha256(file_bytes) == record["file_sha256"], (
            f"{name}: .phos bytes changed")
        assert phosphor_precompute.HEADER_BYTES == record["header_bytes"]
        header = phosphor_precompute.read_header(path)
        assert header == record["header_parsed"], (
            f"{name}: header no longer parses to the recorded fields: "
            f"{header}")
        payload = file_bytes[record["header_bytes"]:]
        assert lib.sha256(payload) == record["payload_sha256"]
        track = phosphor_precompute.PrecomputedTrack(path)
        assert track.frame_count == record["frames"]
        assert track.sample_rate == record["rate"]
        track.close()
        import numpy
        first = numpy.frombuffer(payload[:32], dtype="<i2").astype(
            numpy.float32) / phosphor_precompute.INT16_SCALE
        assert [float(value) for value in first] == record["first_decoded_f32"]
    print(f"  {len(manifest['phos'])} .phos round-trips verified")


def _native_replay():
    """Runs in a subprocess with the native core allowed."""
    import phosphor_signal
    assert phosphor_signal.native_available(), "native core not loadable"
    manifest = lib.read_json(os.path.join(lib.GOLDEN, "native-v3",
                                          "manifest.json"))
    _replay_segment_set(os.path.join(lib.GOLDEN, "native-v3", "cases"),
                        manifest["cases"])


def test_native_set():
    if not os.path.isdir(os.path.join(lib.GOLDEN, "native-v3")):
        print("  native-v3 set not captured: skipping")
        return
    environment = {key: value for key, value in os.environ.items()
                   if key != "PHOSPHOR_NO_NATIVE"}
    probe = subprocess.run(
        [sys.executable, "-c",
         f"import sys; sys.path.insert(0, {PROJECT!r}); "
         "import phosphor_signal; "
         "sys.exit(0 if phosphor_signal.native_available() else 3)"],
        env=environment)
    if probe.returncode != 0:
        print("  native core not loadable here: rust-v3 replay skipped")
        return
    result = subprocess.run(
        [sys.executable, os.path.abspath(__file__), "--native"],
        env=environment)
    assert result.returncode == 0, "rust-v3 golden replay failed"


if __name__ == "__main__":
    if "--native" in sys.argv:
        del os.environ["PHOSPHOR_NO_NATIVE"]   # set at import, undo for doc
        _native_replay()
    else:
        test_reference_segments()
        test_kit_audio()
        test_phos_round_trips()
        test_native_set()
        print("GOLDEN REPLAY OK")
